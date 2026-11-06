// the journal on disk: one json line per entry, appended after every operation.
// at start the world is rebuilt from it (World::replay), so everything that was
// done, edits of the developer included, survives a restart.
// an old journal is never overwritten: --fresh moves it aside as an archive volume

use crate::ecs::Entry;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Store {
    pub path: PathBuf,
    // how many entries of the journal are on disk
    pub saved: usize,
}

// the entries of a journal file; None when there is no file or it is empty.
// a last line without its newline is a write that was cut off (power loss):
// it is left out and reported, every other bad line is an error
pub fn load(path: &Path) -> Result<Option<(Vec<Entry>, Option<String>)>, String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {}", path.display(), e)),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let complete = text.ends_with('\n');
    let lines: Vec<&str> = text.lines().collect();
    let mut entries = Vec::with_capacity(lines.len());
    let mut warning = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Entry>(line) {
            Ok(e) => entries.push(e),
            Err(_) if i + 1 == lines.len() && !complete => {
                warning = Some(format!("the last line of {} was cut off and is left out", path.display()));
            }
            Err(e) => return Err(format!("{} line {}: {}", path.display(), i + 1, e)),
        }
    }
    Ok(Some((entries, warning)))
}

// moves a journal aside: journal.jsonl -> journal-<seconds>.jsonl
pub fn archive(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("journal");
    let to = path.with_file_name(format!("{}-{}.jsonl", stem, secs));
    fs::rename(path, &to).map_err(|e| format!("cannot move {} aside: {}", path.display(), e))?;
    Ok(Some(to))
}

impl Store {
    pub fn new(path: PathBuf, saved: usize) -> Store {
        Store { path, saved }
    }

    // writes the entries that are not on disk yet, and makes sure they are
    pub fn append(&mut self, journal: &[Entry]) -> Result<(), String> {
        if journal.len() <= self.saved {
            return Ok(());
        }
        if let Some(dir) = self.path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {}", dir.display(), e))?;
            }
        }
        let mut out = String::new();
        for e in &journal[self.saved..] {
            out.push_str(&serde_json::to_string(e).map_err(|e| e.to_string())?);
            out.push('\n');
        }
        let mut f = OpenOptions::new().create(true).append(true).open(&self.path).map_err(|e| format!("cannot open {}: {}", self.path.display(), e))?;
        f.write_all(out.as_bytes()).map_err(|e| format!("cannot write {}: {}", self.path.display(), e))?;
        f.sync_data().map_err(|e| format!("cannot sync {}: {}", self.path.display(), e))?;
        self.saved = journal.len();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::World;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("radost_store_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn what_is_appended_comes_back() {
        let d = dir("roundtrip");
        let path = d.join("world").join("journal.jsonl");
        let mut w = World::new();
        let mut s = Store::new(path.clone(), 0);
        s.append(&w.journal).unwrap();
        w.current_role = String::from("dev");
        w.set("x", "desc", "a line\nwith \"quotes\" and \u{00e4}").unwrap();
        s.append(&w.journal).unwrap();
        s.append(&w.journal).unwrap(); // nothing new: nothing written twice
        let (back, warn) = load(&path).unwrap().unwrap();
        assert!(warn.is_none());
        assert_eq!(back, w.journal);
        assert_eq!(World::replay(back).unwrap().value("x", "desc"), w.value("x", "desc"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_cut_off_last_line_is_left_out_and_said() {
        let d = dir("cut");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("journal.jsonl");
        let w = World::new();
        let mut s = Store::new(path.clone(), 0);
        s.append(&w.journal).unwrap();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"seq\":99,\"ro").unwrap();
        let (back, warn) = load(&path).unwrap().unwrap();
        assert_eq!(back.len(), w.journal.len());
        assert!(warn.unwrap().contains("cut off"));
        // a broken line in the middle is an error
        fs::write(&path, "nonsense\n{}\n").unwrap();
        assert!(load(&path).is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn archive_moves_the_old_journal_aside() {
        let d = dir("archive");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("journal.jsonl");
        assert!(archive(&path).unwrap().is_none());
        fs::write(&path, "x\n").unwrap();
        let to = archive(&path).unwrap().unwrap();
        assert!(!path.exists() && to.exists());
        assert!(load(&path).unwrap().is_none());
        let _ = fs::remove_dir_all(&d);
    }
}
