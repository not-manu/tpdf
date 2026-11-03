use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::{env, process};

pub fn dir(name: &str) -> Option<PathBuf> {
    let base = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| Path::new(&home).join(".local/state")))?;
    let dir = base.join("tpdf").join(name);
    DirBuilder::new().recursive(true).mode(0o700).create(&dir).ok()?;
    Some(dir)
}

pub fn write_atomic(file: &Path, bytes: &[u8]) {
    let temporary = file.with_extension(format!("{}.tmp", process::id()));
    let written = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut handle| handle.write_all(bytes));
    if written.is_ok() {
        let _ = fs::rename(&temporary, file);
    } else {
        let _ = fs::remove_file(&temporary);
    }
}
