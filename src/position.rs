use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::store;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    Page,
    Width,
    Free,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub path: String,
    pub page: usize,
    pub within: f32,
    pub across: f32,
    pub fit: Fit,
    pub zoom: f32,
}

pub struct Positions {
    file: Option<PathBuf>,
    last: Option<Position>,
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, &byte| {
        (hash ^ byte as u64).wrapping_mul(0x100000001b3)
    })
}

fn parse(bytes: &[u8], path: &str) -> Option<Position> {
    serde_json::from_slice::<Position>(bytes)
        .ok()
        .filter(|position| position.path == path && position.page >= 1 && position.zoom > 0.0)
}

impl Positions {
    pub fn open(path: &str) -> (Positions, Option<Position>) {
        let file = store::dir("positions")
            .map(|dir| dir.join(format!("{:016x}.json", fnv1a(path.as_bytes()))));
        let saved = file
            .as_ref()
            .and_then(|file| fs::read(file).ok())
            .and_then(|bytes| parse(&bytes, path));
        (Positions { file, last: saved.clone() }, saved)
    }

    pub fn save(&mut self, position: Position) {
        if self.last.as_ref() == Some(&position) {
            return;
        }
        if let Some(file) = &self.file
            && let Ok(json) = serde_json::to_vec(&position)
        {
            store::write_atomic(file, &json);
        }
        self.last = Some(position);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_published_fnv1a_vectors() {
        assert_eq!(fnv1a(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a(b"a"), 0xaf63dc4c8601ec8c);
    }

    fn sample() -> Position {
        Position {
            path: "/a b.pdf".to_string(),
            page: 3,
            within: 0.25,
            across: 0.5,
            fit: Fit::Width,
            zoom: 1.0,
        }
    }

    #[test]
    fn a_saved_position_round_trips() {
        let bytes = serde_json::to_vec(&sample()).unwrap();
        assert_eq!(parse(&bytes, "/a b.pdf"), Some(sample()));
    }

    #[test]
    fn a_position_for_another_file_is_ignored() {
        let bytes = serde_json::to_vec(&sample()).unwrap();
        assert_eq!(parse(&bytes, "/other.pdf"), None);
    }

    #[test]
    fn corrupt_or_nonsense_positions_are_ignored() {
        assert_eq!(parse(b"{not json", "/a b.pdf"), None);
        let bad_page = Position { page: 0, ..sample() };
        assert_eq!(parse(&serde_json::to_vec(&bad_page).unwrap(), "/a b.pdf"), None);
        let bad_zoom = Position { zoom: 0.0, ..sample() };
        assert_eq!(parse(&serde_json::to_vec(&bad_zoom).unwrap(), "/a b.pdf"), None);
    }
}
