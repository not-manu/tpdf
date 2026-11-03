use std::env;
use std::io::{Read, Write, stdout};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::pdf::Doc;

pub const INDEX_FLAG: &str = "--index";
const SEPARATOR: u8 = 0;

pub struct Index {
    child: Option<Child>,
    arriving: Option<Receiver<Vec<String>>>,
    pages: Vec<String>,
}

pub fn write(path: &Path) -> Result<(), String> {
    let doc = Doc::open(path)?;
    let mut output = Vec::new();
    for page in 0..doc.pages() {
        output.extend_from_slice(doc.folded_page(page).as_bytes());
        output.push(SEPARATOR);
    }
    stdout().write_all(&output).map_err(|error| error.to_string())
}

fn parse(bytes: &[u8], pages: usize) -> Vec<String> {
    let mut parsed: Vec<String> = bytes
        .split(|byte| *byte == SEPARATOR)
        .map(|page| String::from_utf8_lossy(page).into_owned())
        .collect();
    parsed.pop();
    if parsed.len() == pages { parsed } else { Vec::new() }
}

impl Index {
    pub fn build(path: &Path, pages: usize) -> Index {
        let spawned = env::current_exe().and_then(|program| {
            Command::new(program)
                .arg(INDEX_FLAG)
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
        });
        let Ok(mut child) = spawned else {
            return Index { child: None, arriving: None, pages: Vec::new() };
        };
        let Some(mut output) = child.stdout.take() else {
            return Index { child: Some(child), arriving: None, pages: Vec::new() };
        };
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            if output.read_to_end(&mut bytes).is_ok() {
                let _ = sender.send(parse(&bytes, pages));
            }
        });
        Index { child: Some(child), arriving: Some(receiver), pages: Vec::new() }
    }

    pub fn receive(&mut self) {
        let Some(pages) = self.arriving.as_ref().and_then(|arriving| arriving.try_recv().ok()) else {
            return;
        };
        self.pages = pages;
        self.arriving = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }

    pub fn may_contain(&mut self, page: usize, folded_needle: &str) -> bool {
        self.receive();
        self.pages.get(page).is_none_or(|text| text.contains(folded_needle))
    }
}

impl Drop for Index {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(pages: &[&str]) -> Index {
        Index {
            child: None,
            arriving: None,
            pages: pages.iter().map(|page| page.to_string()).collect(),
        }
    }

    #[test]
    fn parsing_splits_pages_and_keeps_empty_ones() {
        assert_eq!(parse(b"ONE\0\0THREE\0", 3), ["ONE", "", "THREE"]);
    }

    #[test]
    fn a_truncated_or_mismatched_build_gives_no_index() {
        assert!(parse(b"ONE\0TW", 2).is_empty());
        assert!(parse(b"ONE\0", 2).is_empty());
        assert!(parse(b"", 1).is_empty());
    }

    #[test]
    fn an_index_rules_out_pages_without_the_needle() {
        let mut index = ready(&["FIRST", "SECOND"]);
        assert!(index.may_contain(0, "FIR"));
        assert!(!index.may_contain(1, "FIR"));
    }

    #[test]
    fn an_empty_needle_cannot_rule_out_a_page() {
        let mut index = ready(&["FIRST"]);
        assert!(index.may_contain(0, ""));
    }

    #[test]
    fn without_an_index_every_page_may_contain_the_needle() {
        let mut index = ready(&[]);
        assert!(index.may_contain(0, "ANYTHING"));
    }

    #[test]
    fn a_page_past_the_index_may_contain_the_needle() {
        let mut index = ready(&["FIRST"]);
        assert!(index.may_contain(5, "ANYTHING"));
    }

    #[test]
    fn an_arriving_index_is_picked_up_by_the_next_search() {
        let (sender, receiver) = mpsc::channel();
        let mut index = Index { child: None, arriving: Some(receiver), pages: Vec::new() };
        assert!(index.may_contain(0, "ABSENT"));
        sender.send(vec!["FIRST".to_string()]).unwrap();
        assert!(!index.may_contain(0, "ABSENT"));
    }
}
