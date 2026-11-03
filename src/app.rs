use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};

use crate::context::{self, Publisher, Region, State};
use crate::input::{self, Key};
use crate::kitty::{self, Graphics, Image};
use crate::layout::{Layout, TILE_COLS, TILE_ROWS};
use crate::pdf::{self, Doc};
use crate::position::{Fit, Position, Positions};
use crate::preferences::Preferences;
use crate::term::{self, RESIZED, STOP, Size};

const IMAGE_BUDGET: usize = 64 << 20;
const FIRST_IMAGE_ID: u8 = 16;
const DEFAULT_PAGE: (f32, f32) = (612.0, 792.0);
const LINE_STEP: i64 = 3;
const COLUMN_STEP: i64 = 6;
const ZOOM_STEP: f32 = 1.25;
const ZOOM_STEP_FAST: f32 = 1.6;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 16.0;
const RELOAD_INTERVAL: Duration = Duration::from_millis(500);
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const ROWS_BETWEEN_DISCARD_MARKS: i64 = 16;
const HIGHLIGHT: [u16; 3] = [0xD0, 0xA2, 0x15];

type TileKey = (usize, i64, i64);

enum Mode {
    Normal,
    Command(String),
    Search { text: String, forward: bool },
}

struct Tile {
    id: u8,
    bytes: usize,
    used: u64,
    name: CString,
}

struct SearchJob {
    page: usize,
    remaining: usize,
    forward: bool,
}

pub struct App {
    path: PathBuf,
    doc: Doc,
    modified: Option<SystemTime>,
    sizes: Vec<(f32, f32)>,
    size: Size,
    view_rows: i64,
    view_cols: i64,
    show_status: bool,
    fit: Fit,
    scale: f32,
    layout: Layout,
    scroll_row: i64,
    scroll_col: i64,
    dark: bool,
    graphics: Graphics,
    tiles: HashMap<TileKey, Tile>,
    free_ids: Vec<u8>,
    image_bytes: usize,
    tick: u64,
    out: Vec<u8>,
    painted: Vec<Vec<u8>>,
    dirty: bool,
    settled: bool,
    mode: Mode,
    count: Option<i64>,
    pending_g: bool,
    message: String,
    needle: String,
    job: Option<SearchJob>,
    hits: Vec<[f32; 4]>,
    hits_page: Option<usize>,
    hit: usize,
    quit: bool,
    tmux: bool,
    discarded: u64,
    drew_since_check: bool,
    last_frame: Instant,
    publisher: Publisher,
    published_path: String,
    positions: Positions,
}

fn page_sizes(doc: &Doc) -> Vec<(f32, f32)> {
    let mut last = DEFAULT_PAGE;
    (0..doc.pages())
        .map(|index| {
            last = doc.bounds(index).unwrap_or(last);
            last
        })
        .collect()
}

fn modified(path: &PathBuf) -> Option<SystemTime> {
    fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}

fn view_size(size: Size, show_status: bool) -> (i64, i64) {
    let bar = if show_status { 1 } else { 0 };
    ((size.rows - bar).max(1), size.cols.max(1))
}

fn fit_scale(page: (f32, f32), fit: Fit, view: (i64, i64), cell: (i64, i64)) -> f32 {
    let by_width = (view.1 * cell.0) as f32 / page.0;
    let by_height = (view.0 * cell.1) as f32 / page.1;
    match fit {
        Fit::Width => by_width,
        _ => by_width.min(by_height),
    }
}

fn blend(pixels: &mut [u8], width: i64, height: i64, rect: [i64; 4]) {
    let (left, top) = (rect[0].max(0), rect[1].max(0));
    let (right, bottom) = (rect[2].min(width), rect[3].min(height));
    for row in top..bottom {
        for col in left..right {
            let offset = ((row * width + col) * 3) as usize;
            for (channel, tint) in pixels[offset..offset + 3].iter_mut().zip(HIGHLIGHT) {
                *channel = ((*channel as u16 * 3 + tint * 2) / 5) as u8;
            }
        }
    }
}

impl App {
    pub fn new(path: PathBuf, doc: Doc, first_page: Option<usize>, tmux: bool) -> App {
        let sizes = page_sizes(&doc);
        let size = term::size();
        let show_status = true;
        let (view_rows, view_cols) = view_size(size, show_status);
        let published_path = fs::canonicalize(&path)
            .unwrap_or_else(|_| path.clone())
            .to_string_lossy()
            .into_owned();
        let (positions, saved) = Positions::open(&published_path);
        let resume = saved.filter(|_| first_page.is_none());
        let page = first_page
            .or(resume.as_ref().map(|position| position.page - 1))
            .unwrap_or(0)
            .min(sizes.len() - 1);
        let fit = resume.as_ref().map_or(Fit::Page, |position| position.fit);
        let view = (view_rows, view_cols);
        let scale = match (fit, &resume) {
            (Fit::Free, Some(position)) => {
                fit_scale(sizes[page], Fit::Page, view, size.cell) * position.zoom.clamp(MIN_ZOOM, MAX_ZOOM)
            }
            _ => fit_scale(sizes[page], fit, view, size.cell),
        };
        let layout = Layout::new(&sizes, scale, size.cell, view_cols);
        let mut app = App {
            publisher: Publisher::new(),
            published_path,
            positions,
            modified: modified(&path),
            path,
            doc,
            sizes,
            size,
            view_rows,
            view_cols,
            show_status,
            fit,
            scale,
            layout,
            scroll_row: 0,
            scroll_col: 0,
            dark: Preferences::load().dark,
            graphics: Graphics::new(tmux),
            tiles: HashMap::new(),
            free_ids: (FIRST_IMAGE_ID..=u8::MAX).rev().collect(),
            image_bytes: 0,
            tick: 0,
            out: Vec::with_capacity(1 << 16),
            painted: Vec::new(),
            dirty: true,
            settled: false,
            mode: Mode::Normal,
            count: None,
            pending_g: false,
            message: String::new(),
            needle: String::new(),
            job: None,
            hits: Vec::new(),
            hits_page: None,
            hit: 0,
            quit: false,
            tmux,
            discarded: term::tmux_discarded(),
            drew_since_check: false,
            last_frame: Instant::now() - FRAME_INTERVAL,
        };
        app.go_to_page(page);
        if let Some(position) = resume {
            let rows = app.layout.rows[page] as f32;
            app.scroll_row += (position.within.clamp(0.0, 1.0) * rows) as i64;
            app.scroll_col = (position.across * app.layout.canvas_cols as f32) as i64 - view_cols / 2;
            app.clamp();
            if page > 0 || position.within > 0.0 {
                app.message = format!("resumed at p.{}", page + 1);
            }
        }
        app
    }

    fn save_position(&mut self) {
        let round = |value: f64| ((value * 10000.0).round() / 10000.0) as f32;
        let page = self.layout.page_at(self.scroll_row.max(0));
        let within = (self.scroll_row - self.layout.tops[page]).max(0) as f64
            / self.layout.rows[page].max(1) as f64;
        let across = (self.scroll_col + self.view_cols / 2) as f64 / self.layout.canvas_cols.max(1) as f64;
        let zoom = match self.fit {
            Fit::Free => self.scale as f64 / self.fit_for(Fit::Page) as f64,
            _ => 1.0,
        };
        self.positions.save(Position {
            path: self.published_path.clone(),
            page: page + 1,
            within: round(within.min(1.0)),
            across: round(across),
            fit: self.fit,
            zoom: round(zoom),
        });
    }

    pub fn run(&mut self) {
        let mut buffer = [0u8; 4096];
        let mut last_check = Instant::now();
        while !self.quit && !STOP.load(Ordering::Relaxed) {
            if RESIZED.swap(false, Ordering::Relaxed) {
                self.relayout(None);
            }
            if term::readable(0) {
                self.read_keys(&mut buffer);
            } else if self.dirty {
                let wait = FRAME_INTERVAL.saturating_sub(self.last_frame.elapsed());
                if wait.is_zero() {
                    self.draw();
                } else {
                    term::readable(wait.as_millis() as i32 + 1);
                }
            } else if self.search_step() {
                continue;
            } else if !self.settled {
                self.settled = !self.prefetch();
                if self.settled {
                    self.publish();
                    self.save_position();
                }
            } else if term::readable(RELOAD_INTERVAL.as_millis() as i32) {
                self.read_keys(&mut buffer);
            } else if last_check.elapsed() >= RELOAD_INTERVAL {
                self.reload_if_changed();
                self.recover_discarded_images();
                last_check = Instant::now();
            }
        }
    }

    fn recover_discarded_images(&mut self) {
        if !self.tmux || !std::mem::take(&mut self.drew_since_check) {
            return;
        }
        let discarded = term::tmux_discarded();
        if discarded != self.discarded {
            self.discarded = discarded;
            self.clear_tiles();
            self.repaint();
        }
    }

    pub fn publish(&mut self) {
        let from = self.scroll_row - self.voffset();
        let (cell_w, cell_h) = self.size.cell;
        let scale = self.scale;
        let round = |value: f32| (value * 10.0).round() / 10.0;
        let visible = self
            .layout
            .spans(from, from + self.view_rows, self.scroll_col, self.view_cols)
            .into_iter()
            .map(|span| {
                let (width, height) = self.sizes[span.page];
                Region {
                    page: span.page + 1,
                    x0: round((span.cols.start * cell_w) as f32 / scale),
                    y0: round((span.rows.start * cell_h) as f32 / scale),
                    x1: round(((span.cols.end * cell_w) as f32 / scale).min(width)),
                    y1: round(((span.rows.end * cell_h) as f32 / scale).min(height)),
                }
            })
            .collect();
        let state = State {
            pid: std::process::id(),
            path: self.published_path.clone(),
            page: self.current_page() + 1,
            pages: self.sizes.len(),
            zoom: self.zoom_label(),
            search: self.hits_page.map(|_| self.needle.clone()),
            visible,
            tmux_socket: context::tmux_socket(),
            tmux_pane: context::tmux_pane(),
            updated: 0,
        };
        self.publisher.publish(state);
    }

    pub fn shutdown(&mut self) {
        self.save_position();
        self.publisher.remove();
        self.clear_tiles();
        term::write_all(&self.out);
        self.out.clear();
    }

    fn read_keys(&mut self, buffer: &mut [u8]) {
        match term::read(buffer) {
            Some(count) => {
                for key in input::parse(&buffer[..count]) {
                    self.handle_key(key);
                }
            }
            None => self.quit = true,
        }
    }

    fn handle_key(&mut self, key: Key) {
        self.dirty = true;
        match &mut self.mode {
            Mode::Normal => self.normal_key(key),
            Mode::Command(_) | Mode::Search { .. } => self.prompt_key(key),
        }
    }

    fn prompt_key(&mut self, key: Key) {
        let text = match &mut self.mode {
            Mode::Command(text) | Mode::Search { text, .. } => text,
            Mode::Normal => return,
        };
        match key {
            Key::Char(character) => text.push(character),
            Key::Ctrl('u') => text.clear(),
            Key::Backspace => {
                if text.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            Key::Esc | Key::Ctrl('c') => self.mode = Mode::Normal,
            Key::Enter => match std::mem::replace(&mut self.mode, Mode::Normal) {
                Mode::Command(text) => self.run_command(text.trim()),
                Mode::Search { text, forward } => {
                    if !text.is_empty() {
                        self.needle = text;
                        self.start_search(self.current_page(), forward);
                    }
                }
                Mode::Normal => {}
            },
            _ => {}
        }
    }

    fn normal_key(&mut self, key: Key) {
        self.message.clear();
        if let Key::Char(digit @ '0'..='9') = key {
            if digit != '0' || self.count.is_some() {
                let value = digit as i64 - '0' as i64;
                self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(value));
                return;
            }
        }
        let pending_g = std::mem::take(&mut self.pending_g);
        let count = self.count.take();
        let times = count.unwrap_or(1).min(1 << 20);
        let half = (self.view_rows / 2).max(1);
        let full = (self.view_rows - 2).max(1);
        match key {
            Key::Char('g') if pending_g => {
                self.go_to_page(count.map_or(0, |page| (page - 1).max(0) as usize));
            }
            Key::Char('g') => {
                self.pending_g = true;
                self.count = count;
            }
            Key::Home => self.go_to_page(0),
            Key::End => self.go_to_end(),
            Key::Char('G') => match count {
                Some(page) => self.go_to_page((page - 1).max(0) as usize),
                None => self.go_to_end(),
            },
            Key::Char('j') | Key::Down => self.scroll_by(LINE_STEP * times, 0),
            Key::Char('k') | Key::Up => self.scroll_by(-LINE_STEP * times, 0),
            Key::Char('h') | Key::Left => self.scroll_by(0, -COLUMN_STEP * times),
            Key::Char('l') | Key::Right => self.scroll_by(0, COLUMN_STEP * times),
            Key::WheelDown => self.scroll_by(LINE_STEP, 0),
            Key::WheelUp => self.scroll_by(-LINE_STEP, 0),
            Key::Ctrl('d') => self.scroll_by(half * times, 0),
            Key::Ctrl('u') => self.scroll_by(-half * times, 0),
            Key::Ctrl('f') | Key::PageDown | Key::Char(' ') => self.scroll_by(full * times, 0),
            Key::Ctrl('b') | Key::PageUp => self.scroll_by(-full * times, 0),
            Key::Char('J') => {
                let page = self.layout.page_at(self.scroll_row.max(0));
                self.go_to_page(page + times as usize);
            }
            Key::Char('K') => {
                let page = self.layout.page_at(self.scroll_row.max(0)) as i64;
                let inside = self.scroll_row > self.layout.tops[page as usize];
                let target = if inside { page + 1 - times } else { page - times };
                self.go_to_page(target.max(0) as usize);
            }
            Key::Char('=') => self.zoom(ZOOM_STEP),
            Key::Char('-') => self.zoom(1.0 / ZOOM_STEP),
            Key::Char('+') => self.zoom(ZOOM_STEP_FAST),
            Key::Char('_') => self.zoom(1.0 / ZOOM_STEP_FAST),
            Key::Char('0') => self.set_fit(Fit::Page),
            Key::Char('w') => {
                self.set_fit(if self.fit == Fit::Width { Fit::Page } else { Fit::Width });
            }
            Key::Char('z') => {
                self.dark = !self.dark;
                Preferences { dark: self.dark }.save();
                self.clear_tiles();
                self.touch();
            }
            Key::Char('f') => {
                self.show_status = !self.show_status;
                self.relayout(None);
            }
            Key::Char('r') => {
                self.modified = None;
                self.reload_if_changed();
            }
            Key::Ctrl('l') => {
                self.clear_tiles();
                self.repaint();
            }
            Key::Char('/') => self.mode = Mode::Search { text: String::new(), forward: true },
            Key::Char('?') => self.mode = Mode::Search { text: String::new(), forward: false },
            Key::Char(':') => self.mode = Mode::Command(String::new()),
            Key::Char('n') => self.next_hit(true),
            Key::Char('N') => self.next_hit(false),
            Key::Esc => self.clear_search(),
            Key::Char('q') | Key::Ctrl('c') => self.quit = true,
            _ => {}
        }
    }

    fn run_command(&mut self, command: &str) {
        if command == "q" || command == "quit" {
            self.quit = true;
        } else if let Some(percent) = command.strip_suffix('%') {
            match percent.trim().parse::<f32>() {
                Ok(value) if value > 0.0 => {
                    let base = self.fit_for(Fit::Page);
                    self.fit = Fit::Free;
                    self.relayout(Some(base * value / 100.0));
                }
                _ => self.message = format!("bad zoom: {command}"),
            }
        } else if let Ok(page) = command.parse::<usize>() {
            self.go_to_page(page.saturating_sub(1));
        } else if !command.is_empty() {
            self.message = format!("unknown command: {command}");
        }
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.settled = false;
    }

    fn repaint(&mut self) {
        self.painted.clear();
        self.out.extend_from_slice(b"\x1b[0m\x1b[2J");
        self.touch();
    }

    fn voffset(&self) -> i64 {
        ((self.view_rows - self.layout.total_rows) / 2).max(0)
    }

    fn clamp(&mut self) {
        let max_row = (self.layout.total_rows - self.view_rows).max(0);
        let max_col = (self.layout.canvas_cols - self.view_cols).max(0);
        self.scroll_row = self.scroll_row.clamp(0, max_row);
        self.scroll_col = self.scroll_col.clamp(0, max_col);
    }

    fn scroll_by(&mut self, rows: i64, cols: i64) {
        self.scroll_row += rows;
        self.scroll_col += cols;
        self.clamp();
        self.touch();
    }

    fn go_to_page(&mut self, page: usize) {
        let page = page.min(self.sizes.len() - 1);
        self.scroll_row = self.layout.tops[page];
        self.clamp();
        self.touch();
    }

    fn go_to_end(&mut self) {
        self.scroll_row = self.layout.total_rows;
        self.clamp();
        self.touch();
    }

    fn center_row(&self) -> i64 {
        (self.scroll_row - self.voffset() + self.view_rows / 2).max(0)
    }

    fn current_page(&self) -> usize {
        self.layout.page_at(self.center_row())
    }

    fn fit_for(&self, fit: Fit) -> f32 {
        let page = self.sizes[self.current_page()];
        fit_scale(page, fit, (self.view_rows, self.view_cols), self.size.cell)
    }

    fn set_fit(&mut self, fit: Fit) {
        self.fit = fit;
        self.relayout(None);
    }

    fn zoom(&mut self, factor: f32) {
        let base = self.fit_for(Fit::Page);
        let scale = (self.scale * factor).clamp(base * MIN_ZOOM, base * MAX_ZOOM);
        self.fit = Fit::Free;
        self.relayout(Some(scale));
    }

    fn relayout(&mut self, target: Option<f32>) {
        let center = self.center_row();
        let page = self.layout.page_at(center);
        let within = (center - self.layout.tops[page]) as f64 / self.layout.rows[page].max(1) as f64;
        let whole_page = self.layout.rows[page] <= self.view_rows;
        let across = (self.scroll_col + self.view_cols / 2) as f64
            / self.layout.canvas_cols.max(1) as f64;

        let old_cell = self.size.cell;
        self.size = term::size();
        (self.view_rows, self.view_cols) = view_size(self.size, self.show_status);
        let scale = match (target, self.fit) {
            (Some(scale), _) => scale,
            (None, Fit::Free) => self.scale,
            (None, fit) => {
                fit_scale(self.sizes[page], fit, (self.view_rows, self.view_cols), self.size.cell)
            }
        };
        if scale != self.scale || old_cell != self.size.cell {
            self.clear_tiles();
        }
        self.scale = scale;
        self.layout = Layout::new(&self.sizes, scale, self.size.cell, self.view_cols);

        let page = page.min(self.sizes.len() - 1);
        let rows = self.layout.rows[page] as f64;
        self.scroll_row = if whole_page {
            self.layout.tops[page]
        } else {
            self.layout.tops[page] + (within.min(1.0) * rows) as i64 - self.view_rows / 2
        };
        self.scroll_col = (across * self.layout.canvas_cols as f64) as i64 - self.view_cols / 2;
        self.clamp();
        self.repaint();
    }

    fn reload_if_changed(&mut self) {
        let current = modified(&self.path);
        if current.is_none() || current == self.modified {
            return;
        }
        if let Ok(doc) = Doc::open(&self.path) {
            if doc.pages() == 0 {
                return;
            }
            self.doc = doc;
            self.modified = current;
            self.sizes = page_sizes(&self.doc);
            self.hits.clear();
            self.hits_page = None;
            self.job = None;
            self.clear_tiles();
            self.layout = Layout::new(&self.sizes, self.scale, self.size.cell, self.view_cols);
            self.clamp();
            self.relayout(None);
        }
    }

    fn tile_rect(&self, key: TileKey) -> (pdf::Tile, i64, i64) {
        let (page, tile_row, tile_col) = key;
        let cols = TILE_COLS.min(self.layout.cols[page] - tile_col * TILE_COLS);
        let rows = TILE_ROWS.min(self.layout.rows[page] - tile_row * TILE_ROWS);
        let (cell_w, cell_h) = self.size.cell;
        let tile = pdf::Tile {
            x: (tile_col * TILE_COLS * cell_w) as i32,
            y: (tile_row * TILE_ROWS * cell_h) as i32,
            width: (cols * cell_w) as i32,
            height: (rows * cell_h) as i32,
        };
        (tile, cols, rows)
    }

    fn release(&mut self, key: TileKey) {
        if let Some(tile) = self.tiles.remove(&key) {
            self.graphics.delete(&mut self.out, tile.id);
            kitty::unlink(&tile.name);
            self.free_ids.push(tile.id);
            self.image_bytes -= tile.bytes;
        }
    }

    fn clear_tiles(&mut self) {
        let keys: Vec<TileKey> = self.tiles.keys().copied().collect();
        for key in keys {
            self.release(key);
        }
    }

    fn clear_page_tiles(&mut self, page: usize) {
        let keys: Vec<TileKey> = self.tiles.keys().copied().filter(|key| key.0 == page).collect();
        for key in keys {
            self.release(key);
        }
    }

    fn evict_one(&mut self, keep: &HashSet<TileKey>) -> bool {
        let oldest = self
            .tiles
            .iter()
            .filter(|(key, _)| !keep.contains(*key))
            .min_by_key(|(_, tile)| tile.used)
            .map(|(key, _)| *key);
        match oldest {
            Some(key) => {
                self.release(key);
                true
            }
            None => false,
        }
    }

    fn ensure(&mut self, key: TileKey, keep: &HashSet<TileKey>) -> bool {
        if let Some(tile) = self.tiles.get_mut(&key) {
            tile.used = self.tick;
            return true;
        }
        let (rect, cols, rows) = self.tile_rect(key);
        let bytes = rect.width as usize * rect.height as usize * 3;
        while self.free_ids.is_empty() || self.image_bytes + bytes > IMAGE_BUDGET {
            if !self.evict_one(keep) {
                break;
            }
        }
        let Some(id) = self.free_ids.pop() else {
            return false;
        };
        let Ok(mut shared) = self.graphics.shared(bytes) else {
            self.free_ids.push(id);
            return false;
        };
        let pixels = shared.pixels();
        if !self.doc.render(key.0, self.scale, &rect, self.dark, pixels) {
            pixels.fill(0xff);
        }
        if self.hits_page == Some(key.0) {
            for hit in &self.hits {
                let area = [
                    (hit[0] * self.scale) as i64 - rect.x as i64,
                    (hit[1] * self.scale) as i64 - rect.y as i64,
                    (hit[2] * self.scale) as i64 - rect.x as i64,
                    (hit[3] * self.scale) as i64 - rect.y as i64,
                ];
                blend(pixels, rect.width as i64, rect.height as i64, area);
            }
        }
        let name = shared.into_name();
        let image = Image {
            id,
            width: rect.width,
            height: rect.height,
            cols,
            rows,
            name: &name,
        };
        self.graphics.transmit(&mut self.out, &image);
        self.tiles.insert(key, Tile { id, bytes, used: self.tick, name });
        self.image_bytes += bytes;
        true
    }

    fn tiles_between(&self, from: i64, to: i64, keys: &mut Vec<TileKey>) {
        for span in self.layout.spans(from, to, self.scroll_col, self.view_cols) {
            for row in span.rows.start / TILE_ROWS..=(span.rows.end - 1) / TILE_ROWS {
                for col in span.cols.start / TILE_COLS..=(span.cols.end - 1) / TILE_COLS {
                    keys.push((span.page, row, col));
                }
            }
        }
    }

    fn visible_tiles(&self) -> Vec<TileKey> {
        let from = self.scroll_row - self.voffset();
        let mut keys = Vec::new();
        self.tiles_between(from, from + self.view_rows, &mut keys);
        keys
    }

    fn wanted_tiles(&self) -> Vec<TileKey> {
        let from = self.scroll_row - self.voffset();
        let to = from + self.view_rows;
        let mut keys = self.visible_tiles();
        self.tiles_between(to, to + self.view_rows, &mut keys);
        self.tiles_between(from - self.view_rows, from, &mut keys);
        let mut seen = HashSet::new();
        keys.retain(|key| seen.insert(*key));
        keys
    }

    fn prefetch(&mut self) -> bool {
        let wanted = self.wanted_tiles();
        let Some(&key) = wanted.iter().find(|key| !self.tiles.contains_key(*key)) else {
            return false;
        };
        let keep: HashSet<TileKey> = wanted.iter().copied().collect();
        let (rect, _, _) = self.tile_rect(key);
        let bytes = rect.width as usize * rect.height as usize * 3;
        let can_evict = self.tiles.keys().any(|key| !keep.contains(key));
        if self.image_bytes + bytes > IMAGE_BUDGET && !can_evict {
            return false;
        }
        let ensured = self.ensure(key, &keep);
        term::write_all(&self.out);
        self.out.clear();
        ensured
    }

    fn page_row(&self, doc_row: i64, y: i64) -> Vec<u8> {
        let mut row = Vec::with_capacity(self.view_cols as usize * 8 + 32);
        write!(row, "\x1b[{};1H", y + 1).expect("writing to a Vec cannot fail");
        let blank = |row: &mut Vec<u8>| row.extend_from_slice(b"\x1b[0m\x1b[K");
        if doc_row < 0 || doc_row >= self.layout.total_rows {
            blank(&mut row);
            return row;
        }
        let page = self.layout.page_at(doc_row);
        let page_row = doc_row - self.layout.tops[page];
        if page_row >= self.layout.rows[page] {
            blank(&mut row);
            return row;
        }
        let left = self.layout.lefts[page] - self.scroll_col;
        let start = left.max(0);
        let end = (left + self.layout.cols[page]).min(self.view_cols);
        if start >= end {
            blank(&mut row);
            return row;
        }
        if start > 0 {
            write!(row, "\x1b[0m\x1b[{start}X\x1b[{start}C").expect("writing to a Vec cannot fail");
        }
        let tile_row = page_row / TILE_ROWS;
        let inner_row = page_row % TILE_ROWS;
        let mut last_tile = -1;
        let mut image = None;
        for x in start..end {
            let col = x - left;
            let tile_col = col / TILE_COLS;
            if tile_col != last_tile {
                last_tile = tile_col;
                image = self.tiles.get(&(page, tile_row, tile_col)).map(|tile| tile.id);
                match image {
                    Some(id) => write!(row, "\x1b[38;5;{id}m").expect("writing to a Vec cannot fail"),
                    None => row.extend_from_slice(b"\x1b[0m"),
                }
            }
            if image.is_some() {
                kitty::cell(&mut row, inner_row, col % TILE_COLS);
            } else {
                row.push(b' ');
            }
        }
        row.extend_from_slice(b"\x1b[0m");
        if end < self.view_cols {
            row.extend_from_slice(b"\x1b[K");
        }
        row
    }

    fn zoom_label(&self) -> String {
        match self.fit {
            Fit::Page => "fit".to_string(),
            Fit::Width => "width".to_string(),
            Fit::Free => format!("{:.0}%", self.scale / self.fit_for(Fit::Page) * 100.0),
        }
    }

    fn status_row(&self) -> Vec<u8> {
        let name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let left = match &self.mode {
            Mode::Command(text) => format!(":{text}"),
            Mode::Search { text, forward } => format!("{}{text}", if *forward { '/' } else { '?' }),
            Mode::Normal if !self.message.is_empty() => self.message.clone(),
            Mode::Normal => name,
        };
        let zoom = self.zoom_label();
        let count = self.count.map_or(String::new(), |count| format!("{count}  "));
        let right = format!("{count}{}/{}  {zoom}", self.current_page() + 1, self.sizes.len());

        let width = self.size.cols as usize;
        let right_width = right.chars().count();
        let room = width.saturating_sub(right_width + 2);
        let left: String = left.chars().take(room).collect();
        let padding = width.saturating_sub(left.chars().count() + right_width + 1);

        let mut row = Vec::with_capacity(width + 32);
        write!(row, "\x1b[{};1H\x1b[0m {left}{:padding$}{right}", self.size.rows, "")
            .expect("writing to a Vec cannot fail");
        row
    }

    fn draw(&mut self) {
        self.tick += 1;
        let visible = self.visible_tiles();
        let keep: HashSet<TileKey> = visible.iter().copied().collect();
        for &key in &visible {
            self.ensure(key, &keep);
        }

        let mut frame = Vec::with_capacity(self.out.len() + self.size.cols as usize * self.size.rows as usize * 8);
        frame.extend_from_slice(b"\x1b[?2026h");
        frame.append(&mut self.out);
        let first_row = self.scroll_row - self.voffset();
        self.painted.resize(self.size.rows as usize, Vec::new());
        let mut rows_since_mark = ROWS_BETWEEN_DISCARD_MARKS;
        for y in 0..self.view_rows {
            let row = self.page_row(first_row + y, y);
            let slot = &mut self.painted[y as usize];
            if *slot != row {
                if rows_since_mark >= ROWS_BETWEEN_DISCARD_MARKS {
                    self.graphics.prevent_discard(&mut frame);
                    rows_since_mark = 0;
                }
                rows_since_mark += 1;
                frame.extend_from_slice(&row);
                *slot = row;
            }
        }
        if self.show_status {
            let row = self.status_row();
            let slot = &mut self.painted[self.size.rows as usize - 1];
            if *slot != row {
                frame.extend_from_slice(&row);
                *slot = row;
            }
        }
        frame.extend_from_slice(b"\x1b[?2026l");
        term::write_all(&frame);
        self.dirty = false;
        self.drew_since_check = true;
        self.last_frame = Instant::now();
    }

    fn clear_search(&mut self) {
        self.job = None;
        self.hits.clear();
        if let Some(page) = self.hits_page.take() {
            self.clear_page_tiles(page);
        }
        self.touch();
    }

    fn start_search(&mut self, page: usize, forward: bool) {
        self.clear_search();
        self.job = Some(SearchJob { page, remaining: self.sizes.len(), forward });
        self.message = format!("searching: {}", self.needle);
    }

    fn search_step(&mut self) -> bool {
        let Some(job) = self.job.as_mut() else {
            return false;
        };
        let page = job.page;
        let forward = job.forward;
        job.remaining -= 1;
        let exhausted = job.remaining == 0;
        let total = self.sizes.len();
        job.page = if forward { (page + 1) % total } else { (page + total - 1) % total };

        let hits = self.doc.search(page, &self.needle);
        if !hits.is_empty() {
            self.job = None;
            self.hit = if forward { 0 } else { hits.len() - 1 };
            self.hits = hits;
            self.hits_page = Some(page);
            self.clear_page_tiles(page);
            self.show_hit();
        } else if exhausted {
            self.job = None;
            self.message = format!("not found: {}", self.needle);
            self.dirty = true;
        }
        true
    }

    fn next_hit(&mut self, forward: bool) {
        if self.needle.is_empty() {
            self.message = "no search yet".to_string();
            return;
        }
        let Some(page) = self.hits_page else {
            self.start_search(self.current_page(), forward);
            return;
        };
        if forward && self.hit + 1 < self.hits.len() {
            self.hit += 1;
            self.show_hit();
        } else if !forward && self.hit > 0 {
            self.hit -= 1;
            self.show_hit();
        } else {
            let total = self.sizes.len();
            let next = if forward { (page + 1) % total } else { (page + total - 1) % total };
            self.start_search(next, forward);
        }
    }

    fn show_hit(&mut self) {
        let Some(page) = self.hits_page else {
            return;
        };
        let hit = self.hits[self.hit];
        let row = self.layout.tops[page] + (hit[1] * self.scale) as i64 / self.size.cell.1;
        let col = self.layout.lefts[page] + (hit[0] * self.scale) as i64 / self.size.cell.0;
        let in_view = |value: i64, start: i64, span: i64| value >= start && value < start + span;
        if self.layout.rows[page] <= self.view_rows {
            self.scroll_row = self.layout.tops[page];
        } else if !in_view(row, self.scroll_row, self.view_rows) {
            self.scroll_row = row - self.view_rows / 3;
        }
        if !in_view(col, self.scroll_col, self.view_cols) {
            self.scroll_col = col - self.view_cols / 3;
        }
        self.clamp();
        self.message = format!("{}: {}/{} on p.{}", self.needle, self.hit + 1, self.hits.len(), page + 1);
        self.touch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_page_uses_the_tighter_dimension() {
        let scale = fit_scale((612.0, 792.0), Fit::Page, (50, 100), (16, 31));
        assert_eq!(scale, (50.0 * 31.0) / 792.0);
    }

    #[test]
    fn fit_width_ignores_height() {
        let scale = fit_scale((612.0, 792.0), Fit::Width, (50, 100), (16, 31));
        assert_eq!(scale, (100.0 * 16.0) / 612.0);
    }

    #[test]
    fn blend_only_touches_the_clamped_rectangle() {
        let mut pixels = vec![255u8; 3 * 3 * 3];
        blend(&mut pixels, 3, 3, [1, 1, 10, 2]);
        assert_eq!(pixels[0..3], [255, 255, 255]);
        let middle = 4 * 3;
        assert_ne!(pixels[middle..middle + 3], [255, 255, 255]);
        assert_eq!(pixels[(3 * 3 * 3 - 3)..], [255, 255, 255]);
    }
}
