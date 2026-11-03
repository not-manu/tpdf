use std::collections::HashMap;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, path};

use serde::{Deserialize, Serialize};

use crate::pdf::{self, Doc};

const PNG_LONG_EDGE: f32 = 1568.0;
const PNG_MAX_SCALE: f32 = 4.0;
const PNG_GAP: usize = 12;
const PNG_GAP_SHADE: u8 = 0x80;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub page: usize,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub pid: u32,
    pub path: String,
    pub page: usize,
    pub pages: usize,
    pub zoom: String,
    pub search: Option<String>,
    pub visible: Vec<Region>,
    pub tmux_socket: Option<String>,
    pub tmux_pane: Option<String>,
    pub updated: u64,
}

pub struct Publisher {
    file: Option<PathBuf>,
    last: Option<State>,
}

#[derive(Default)]
struct AgentTmux {
    socket: Option<String>,
    window: Option<String>,
    session: Option<String>,
    panes: HashMap<String, Pane>,
}

struct Pane {
    window: String,
    session: String,
    on_screen: bool,
}

#[derive(Serialize)]
struct VisibleOutput<'a> {
    #[serde(flatten)]
    region: &'a Region,
    text: String,
}

#[derive(Serialize)]
struct OtherReader<'a> {
    pid: u32,
    path: &'a str,
    page: usize,
}

#[derive(Serialize)]
struct Output<'a> {
    pid: u32,
    path: &'a str,
    page: usize,
    pages: usize,
    zoom: &'a str,
    search: &'a Option<String>,
    relation: &'static str,
    seconds_since_change: u64,
    png: Option<String>,
    visible: Vec<VisibleOutput<'a>>,
    page_text: String,
    other_readers: Vec<OtherReader<'a>>,
}

fn state_dir() -> Option<PathBuf> {
    let base = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| Path::new(&home).join(".local/state")))?;
    Some(base.join("tpdf"))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

pub fn tmux_socket() -> Option<String> {
    let value = env::var("TMUX").ok().filter(|value| !value.is_empty())?;
    value.split(',').next().map(str::to_string)
}

pub fn tmux_pane() -> Option<String> {
    env::var("TMUX_PANE").ok().filter(|value| !value.is_empty())
}

impl Publisher {
    pub fn new() -> Publisher {
        let file = state_dir().and_then(|dir| {
            DirBuilder::new().recursive(true).mode(0o700).create(&dir).ok()?;
            Some(dir.join(format!("{}.json", process::id())))
        });
        Publisher { file, last: None }
    }

    pub fn publish(&mut self, state: State) {
        if self.last.as_ref() == Some(&state) {
            return;
        }
        self.last = Some(state.clone());
        let Some(file) = &self.file else {
            return;
        };
        let stamped = State { updated: now(), ..state };
        let Ok(json) = serde_json::to_vec(&stamped) else {
            return;
        };
        let temporary = file.with_extension("json.tmp");
        let written = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .and_then(|mut handle| handle.write_all(&json));
        if written.is_ok() {
            let _ = fs::rename(&temporary, file);
        }
    }

    pub fn remove(&self) {
        if let Some(file) = &self.file {
            let _ = fs::remove_file(file);
        }
    }
}

fn is_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn read_states() -> Vec<State> {
    let Some(entries) = state_dir().and_then(|dir| fs::read_dir(dir).ok()) else {
        return Vec::new();
    };
    let mut states = Vec::new();
    for entry in entries.flatten() {
        let file = entry.path();
        if file.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let parsed = fs::read(&file).ok().and_then(|bytes| serde_json::from_slice::<State>(&bytes).ok());
        match parsed {
            Some(state) if is_alive(state.pid) => states.push(state),
            _ => {
                let _ = fs::remove_file(&file);
            }
        }
    }
    states
}

fn agent_tmux() -> AgentTmux {
    let Some(socket) = tmux_socket() else {
        return AgentTmux::default();
    };
    let format = "#{pane_id} #{window_id} #{session_id} #{window_active} #{session_attached}";
    let output = Command::new("tmux")
        .args(["list-panes", "-a", "-F", format])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let text = output.ok().and_then(|output| String::from_utf8(output.stdout).ok()).unwrap_or_default();
    let panes: HashMap<String, Pane> = text.lines().filter_map(parse_pane).collect();
    let own = tmux_pane().and_then(|id| panes.get(&id));
    AgentTmux {
        window: own.map(|pane| pane.window.clone()),
        session: own.map(|pane| pane.session.clone()),
        socket: Some(socket),
        panes,
    }
}

fn parse_pane(line: &str) -> Option<(String, Pane)> {
    let mut fields = line.split(' ');
    let id = fields.next()?.to_string();
    let window = fields.next()?.to_string();
    let session = fields.next()?.to_string();
    let active = fields.next()? == "1";
    let attached = fields.next()? != "0";
    Some((id, Pane { window, session, on_screen: active && attached }))
}

fn pane_of<'a>(state: &State, agent: &'a AgentTmux) -> Option<&'a Pane> {
    if agent.socket.is_none() || state.tmux_socket != agent.socket {
        return None;
    }
    agent.panes.get(state.tmux_pane.as_ref()?)
}

fn rank(state: &State, agent: &AgentTmux) -> (bool, bool, bool, u64) {
    let pane = pane_of(state, agent);
    let same = |value: &Option<String>, field: fn(&Pane) -> &String| {
        pane.is_some_and(|pane| value.as_ref() == Some(field(pane)))
    };
    (
        same(&agent.window, |pane| &pane.window),
        same(&agent.session, |pane| &pane.session),
        pane.is_some_and(|pane| pane.on_screen),
        state.updated,
    )
}

fn relation(state: &State, agent: &AgentTmux) -> &'static str {
    match rank(state, agent) {
        (true, ..) => "same tmux window",
        (_, true, ..) => "same tmux session",
        (_, _, true, _) => "on screen in another tmux session",
        _ => "elsewhere",
    }
}

fn area(region: &Region) -> [f32; 4] {
    [region.x0, region.y0, region.x1, region.y1]
}

fn absolute(file: &Path) -> String {
    path::absolute(file).unwrap_or_else(|_| file.to_path_buf()).to_string_lossy().into_owned()
}

fn write_png(doc: &Doc, regions: &[Region], file: &Path) -> Result<(), String> {
    let width_points = regions.iter().map(|region| region.x1 - region.x0).fold(0.0, f32::max);
    let height_points: f32 = regions.iter().map(|region| region.y1 - region.y0).sum();
    if width_points <= 0.0 || height_points <= 0.0 {
        return Err("nothing is visible in this reader".to_string());
    }
    let scale = (PNG_LONG_EDGE / width_points.max(height_points)).min(PNG_MAX_SCALE);

    let sized: Vec<(&Region, usize, usize)> = regions
        .iter()
        .map(|region| {
            let width = ((region.x1 - region.x0) * scale).ceil().max(1.0) as usize;
            let height = ((region.y1 - region.y0) * scale).ceil().max(1.0) as usize;
            (region, width, height)
        })
        .collect();
    let width = sized.iter().map(|size| size.1).max().unwrap_or(1);
    let height = sized.iter().map(|size| size.2).sum::<usize>() + PNG_GAP * (sized.len() - 1);

    let mut canvas = vec![PNG_GAP_SHADE; width * height * 3];
    let mut top = 0;
    for (region, tile_width, tile_height) in sized {
        let tile = pdf::Tile {
            x: (region.x0 * scale) as i32,
            y: (region.y0 * scale) as i32,
            width: tile_width as i32,
            height: tile_height as i32,
        };
        let mut pixels = vec![0u8; tile_width * tile_height * 3];
        if !doc.render(region.page - 1, scale, &tile, false, &mut pixels) {
            return Err(format!("could not render page {}", region.page));
        }
        let left = (width - tile_width) / 2;
        for (row, source) in pixels.chunks_exact(tile_width * 3).enumerate() {
            let start = ((top + row) * width + left) * 3;
            canvas[start..start + source.len()].copy_from_slice(source);
        }
        top += tile_height + PNG_GAP;
    }

    if let Some(parent) = file.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    if !doc.save_png(file, width, height, &mut canvas) {
        return Err(format!("could not write {}", file.display()));
    }
    Ok(())
}

pub fn run(pid: Option<u32>, png: Option<PathBuf>) -> Result<(), String> {
    let states = read_states();
    let agent = agent_tmux();
    let chosen = match pid {
        Some(pid) => states.iter().find(|state| state.pid == pid),
        None => states.iter().max_by_key(|state| rank(state, &agent)),
    };
    let Some(state) = chosen else {
        return Err(match pid {
            Some(pid) => format!("no tpdf reader with pid {pid}"),
            None => "no tpdf reader is open".to_string(),
        });
    };

    let doc = Doc::open(Path::new(&state.path)).map_err(|error| format!("{}: {error}", state.path))?;
    if let Some(file) = &png {
        write_png(&doc, &state.visible, file)?;
    }

    let visible = state
        .visible
        .iter()
        .map(|region| VisibleOutput { region, text: doc.text(region.page - 1, area(region)) })
        .collect();
    let page_text = doc
        .bounds(state.page - 1)
        .map(|(width, height)| doc.text(state.page - 1, [0.0, 0.0, width, height]))
        .unwrap_or_default();
    let other_readers = states
        .iter()
        .filter(|other| other.pid != state.pid)
        .map(|other| OtherReader { pid: other.pid, path: &other.path, page: other.page })
        .collect();

    let output = Output {
        pid: state.pid,
        path: &state.path,
        page: state.page,
        pages: state.pages,
        zoom: &state.zoom,
        search: &state.search,
        relation: relation(state, &agent),
        seconds_since_change: now().saturating_sub(state.updated),
        png: png.as_deref().map(absolute),
        visible,
        page_text,
        other_readers,
    };
    let json = serde_json::to_string_pretty(&output).map_err(|error| error.to_string())?;
    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(pid: u32, pane: &str, updated: u64) -> State {
        State {
            pid,
            path: format!("/{pid}.pdf"),
            page: 1,
            pages: 1,
            zoom: "fit".to_string(),
            search: None,
            visible: Vec::new(),
            tmux_socket: Some("/sock".to_string()),
            tmux_pane: Some(pane.to_string()),
            updated,
        }
    }

    fn agent() -> AgentTmux {
        let lines = [
            "%1 @1 $1 1 1",
            "%2 @1 $1 1 1",
            "%3 @2 $1 0 1",
            "%4 @3 $2 1 1",
            "%5 @4 $3 1 0",
        ];
        AgentTmux {
            socket: Some("/sock".to_string()),
            window: Some("@1".to_string()),
            session: Some("$1".to_string()),
            panes: lines.iter().filter_map(|line| parse_pane(line)).collect(),
        }
    }

    fn pick(states: &[State], agent: &AgentTmux) -> u32 {
        states.iter().max_by_key(|state| rank(state, agent)).unwrap().pid
    }

    #[test]
    fn same_window_beats_a_more_recent_reader_elsewhere() {
        let states = [state(1, "%2", 10), state(2, "%3", 99), state(3, "%4", 99)];
        assert_eq!(pick(&states, &agent()), 1);
        assert_eq!(relation(&states[0], &agent()), "same tmux window");
    }

    #[test]
    fn same_session_beats_another_session() {
        let states = [state(2, "%3", 10), state(3, "%4", 99)];
        assert_eq!(pick(&states, &agent()), 2);
        assert_eq!(relation(&states[0], &agent()), "same tmux session");
    }

    #[test]
    fn on_screen_beats_a_more_recent_detached_session() {
        let states = [state(3, "%4", 10), state(4, "%5", 99)];
        assert_eq!(pick(&states, &agent()), 3);
        assert_eq!(relation(&states[1], &agent()), "elsewhere");
    }

    #[test]
    fn most_recent_wins_when_nothing_else_differs() {
        let states = [state(4, "%5", 10), state(5, "%9", 99)];
        assert_eq!(pick(&states, &agent()), 5);
    }

    #[test]
    fn a_reader_on_another_tmux_server_is_not_treated_as_nearby() {
        let mut other = state(1, "%2", 10);
        other.tmux_socket = Some("/other".to_string());
        assert_eq!(relation(&other, &agent()), "elsewhere");
    }

    #[test]
    fn an_agent_outside_tmux_falls_back_to_most_recent() {
        let states = [state(1, "%2", 10), state(2, "%3", 99)];
        assert_eq!(pick(&states, &AgentTmux::default()), 2);
    }
}
