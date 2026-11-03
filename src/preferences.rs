use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::store;

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub dark: bool,
}

fn file() -> Option<PathBuf> {
    store::dir("").map(|dir| dir.join("preferences.json"))
}

fn parse(bytes: &[u8]) -> Preferences {
    serde_json::from_slice(bytes).unwrap_or_default()
}

impl Preferences {
    pub fn load() -> Preferences {
        file().and_then(|file| fs::read(file).ok()).map_or_else(Preferences::default, |bytes| parse(&bytes))
    }

    pub fn save(&self) {
        if let Some(file) = file()
            && let Ok(json) = serde_json::to_vec(self)
        {
            store::write_atomic(&file, &json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_preferences_round_trip() {
        let bytes = serde_json::to_vec(&Preferences { dark: true }).unwrap();
        assert_eq!(parse(&bytes), Preferences { dark: true });
    }

    #[test]
    fn corrupt_preferences_fall_back_to_light() {
        assert_eq!(parse(b"{not json"), Preferences { dark: false });
    }

    #[test]
    fn missing_and_unknown_fields_are_tolerated() {
        assert_eq!(parse(b"{}"), Preferences { dark: false });
        assert_eq!(parse(br#"{"dark":true,"later":1}"#), Preferences { dark: true });
    }
}
