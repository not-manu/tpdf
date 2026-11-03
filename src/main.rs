mod app;
mod input;
mod kitty;
mod layout;
mod pdf;
mod term;

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use crate::app::App;
use crate::pdf::Doc;

const USAGE: &str = "usage: tpdf <file.pdf> [page]\n       tpdf --bench <file.pdf>";
const BENCH_WIDTH: f32 = 1400.0;

fn view(path: PathBuf, page: usize) -> Result<(), String> {
    let doc = Doc::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    if doc.pages() == 0 {
        return Err(format!("{}: no pages", path.display()));
    }
    let tmux = term::in_tmux();
    if tmux {
        term::allow_passthrough();
    }
    let guard = term::enter()?;
    let mut app = App::new(path, doc, page, tmux);
    app.run();
    app.shutdown();
    drop(guard);
    Ok(())
}

fn peak_memory_mb() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe {
        libc::getrusage(libc::RUSAGE_SELF, &mut usage);
    }
    usage.ru_maxrss as f64 / (1024.0 * 1024.0)
}

fn bench(path: &Path) -> Result<(), String> {
    let start = Instant::now();
    let doc = Doc::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let opened = start.elapsed();

    let start = Instant::now();
    let sizes: Vec<(f32, f32)> = (0..doc.pages()).filter_map(|index| doc.bounds(index)).collect();
    let bounds = start.elapsed();

    let mut total = 0.0;
    let mut slowest: f64 = 0.0;
    let mut buffer = Vec::new();
    for (index, size) in sizes.iter().enumerate() {
        let scale = BENCH_WIDTH / size.0;
        let tile = pdf::Tile {
            x: 0,
            y: 0,
            width: BENCH_WIDTH as i32,
            height: (size.1 * scale) as i32,
        };
        buffer.resize(tile.width as usize * tile.height as usize * 3, 0);
        let start = Instant::now();
        doc.render(index, scale, &tile, false, &mut buffer);
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        total += elapsed;
        slowest = slowest.max(elapsed);
    }

    println!("pages      {}", sizes.len());
    println!("open       {:.1} ms", opened.as_secs_f64() * 1000.0);
    println!("bounds     {:.1} ms", bounds.as_secs_f64() * 1000.0);
    println!("render     {:.1} ms avg, {slowest:.1} ms max", total / sizes.len().max(1) as f64);
    println!("peak rss   {:.1} MB", peak_memory_mb());
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [flag, path] if flag == "--bench" => bench(Path::new(path)),
        [path] => view(PathBuf::from(path), 0),
        [path, page] => match page.to_str().and_then(|text| text.parse::<usize>().ok()) {
            Some(page) => view(PathBuf::from(path), page.saturating_sub(1)),
            None => Err(USAGE.to_string()),
        },
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
