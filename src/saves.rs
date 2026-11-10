// saves of a game: the values of its world in named slots, kept between starts.
//   ecs.save(slot)   the values now into the slot (after this tick)
//   ecs.load(slot)   the values of the slot back into the world (after this tick)
//   ecs.slots()      the names of the slots there are, sorted
//   ecs.erase(slot)  the slot away
// a slot is 1..32 of a-z 0-9 _ -. only the data is saved: the code (entities with src) and
// the types (entities with kind) come from the files, so a change of the code never breaks
// a save. a load checks the save against the types of the game and refuses it, with the
// reason, when a type is gone or has another kind. the saves of a game are in the folder of
// the user's data (dir_for), never beside the game: a packed game is unpacked anew at each
// start. a world without that folder (tests, lust_try) keeps its saves in memory
use crate::ecs::World;
use crate::host::Host;
use crate::project::as_core;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub const SAVE_FORMAT: &str = "lust-save-1";
pub const SLOTS_MAX: usize = 20;
pub const SAVE_BYTES_MAX: usize = 4 * 1024 * 1024;
const SLOT_MAX: usize = 32;

#[derive(Serialize, Deserialize)]
struct SaveFile {
    format: String,
    // the kind of every component a value of the save has
    kinds: BTreeMap<String, String>,
    values: Vec<(String, String, String)>,
}

// what to do after the tick
enum Job {
    Save(String),
    Load(String),
}

#[derive(Default)]
pub struct Saves {
    // the folder of the saves; None keeps them in memory
    dir: RefCell<Option<PathBuf>>,
    mem: RefCell<BTreeMap<String, String>>,
    jobs: RefCell<Vec<Job>>,
    // what went wrong since it was last asked (disk, a refused load)
    errors: RefCell<Vec<String>>,
}

pub fn slot_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= SLOT_MAX && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn bad_slot(s: &str) -> mlua::Error {
    mlua::Error::RuntimeError(format!("bad_slot: '{}' (1..{} of a-z 0-9 _ -)", crate::host::clip(s, 40), SLOT_MAX))
}

// the folder of the user's data for a game named id: %APPDATA%\lust\<id> on windows,
// $XDG_DATA_HOME/lust/<id> or ~/.local/share/lust/<id> elsewhere
pub fn dir_for(id: &str) -> Option<PathBuf> {
    let id: String = id.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == ' ' { c } else { '_' }).take(64).collect();
    let id = id.trim().trim_matches('.').to_string();
    if id.is_empty() {
        return None;
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
    };
    base.map(|b| b.join("lust").join(id))
}

// the name of a game for its saves: the file .id (a packed game has it), else its folder
pub fn id_of(root: &Path) -> String {
    if let Ok(t) = fs::read_to_string(root.join(".id")) {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    root.canonicalize().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| String::from("game"))
}

// an entity of data: not a type, not code
fn is_data(w: &World, e: &str) -> bool {
    w.own(e, "kind").is_none() && w.own(e, "src").is_none()
}

// the data of a world as the text of a save
pub fn encode(w: &World) -> Result<String, String> {
    let snap = w.snapshot();
    let mut data: BTreeMap<String, bool> = BTreeMap::new();
    let mut kinds = BTreeMap::new();
    let mut values = Vec::new();
    for (e, c, v) in snap.values {
        let ok = *data.entry(e.clone()).or_insert_with(|| is_data(w, &e));
        if !ok {
            continue;
        }
        if let Some(k) = w.own(&c, "kind") {
            kinds.insert(c.clone(), k.to_string());
        }
        values.push((e, c, v));
    }
    let text = serde_json::to_string(&SaveFile { format: SAVE_FORMAT.to_string(), kinds, values }).map_err(|e| e.to_string())?;
    if text.len() > SAVE_BYTES_MAX {
        return Err(format!("save_too_big: {} KB, at most {} KB", text.len() / 1024, SAVE_BYTES_MAX / 1024));
    }
    Ok(text)
}

// a save into a world: the data of the world becomes the data of the save. nothing changes
// when the save does not fit the types of the world
pub fn apply(w: &mut World, text: &str) -> Result<(), String> {
    if text.len() > SAVE_BYTES_MAX {
        return Err(String::from("save_too_big"));
    }
    let f: SaveFile = serde_json::from_str(text).map_err(|e| format!("not a save: {}", e))?;
    if f.format != SAVE_FORMAT {
        return Err(format!("not a save of this kind: {}", crate::host::clip(&f.format, 40)));
    }
    let mut wrong = Vec::new();
    for (c, k) in &f.kinds {
        match w.own(c, "kind") {
            Some(now) if now == k => {}
            Some(now) => wrong.push(format!("{} is {} in the save, {} in the game", c, k, now)),
            None => wrong.push(format!("{} is no type of the game now", c)),
        }
    }
    for (e, _, _) in &f.values {
        if w.own(e, "kind").is_some() || w.own(e, "src").is_some() {
            wrong.push(format!("{} is a type or code in the game, not data", e));
        }
    }
    if !wrong.is_empty() {
        wrong.sort();
        wrong.dedup();
        wrong.truncate(5);
        return Err(format!("save_does_not_fit: {}", wrong.join("; ")));
    }
    let keep: BTreeSet<(String, String)> = f.values.iter().map(|(e, c, _)| (e.clone(), c.clone())).collect();
    let now: Vec<(String, String)> = w.snapshot().values.into_iter().filter(|(e, _, _)| is_data(w, e)).map(|(e, c, _)| (e, c)).collect();
    let mut errs = Vec::new();
    for (e, c) in now {
        if !keep.contains(&(e.clone(), c.clone())) {
            if let Err(x) = w.del(&e, &c) {
                errs.push(format!("{}.{}: {}", e, c, x));
            }
        }
    }
    // references last: they name entities that must exist first
    for refs in [false, true] {
        for (e, c, v) in &f.values {
            let is_ref = f.kinds.get(c).map(|k| k == "ref").unwrap_or(false);
            if is_ref != refs {
                continue;
            }
            if let Err(x) = w.set(e, c, v) {
                errs.push(format!("{}.{}: {}", e, c, x));
            }
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        errs.truncate(5);
        Err(format!("load: {}", errs.join("; ")))
    }
}

impl Saves {
    pub fn set_dir(&self, dir: Option<PathBuf>) {
        *self.dir.borrow_mut() = dir;
    }

    pub fn dir(&self) -> Option<PathBuf> {
        self.dir.borrow().clone()
    }

    pub fn slots(&self) -> Vec<String> {
        let mut out: Vec<String> = match &*self.dir.borrow() {
            None => self.mem.borrow().keys().cloned().collect(),
            Some(d) => match fs::read_dir(d) {
                Ok(rd) => rd.flatten().filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".save").map(|s| s.to_string())).filter(|s| slot_ok(s)).collect(),
                Err(_) => Vec::new(),
            },
        };
        out.sort();
        out
    }

    fn read(&self, slot: &str) -> Result<String, String> {
        match &*self.dir.borrow() {
            None => self.mem.borrow().get(slot).cloned().ok_or_else(|| format!("no_slot: {}", slot)),
            Some(d) => {
                let p = d.join(format!("{}.save", slot));
                let len = fs::metadata(&p).map(|m| m.len() as usize).map_err(|_| format!("no_slot: {}", slot))?;
                if len > SAVE_BYTES_MAX {
                    return Err(String::from("save_too_big"));
                }
                fs::read_to_string(&p).map_err(|e| format!("cannot read {}: {}", p.display(), e))
            }
        }
    }

    fn write(&self, slot: &str, text: String) -> Result<(), String> {
        let have = self.slots();
        if !have.iter().any(|s| s == slot) && have.len() >= SLOTS_MAX {
            return Err(format!("too_many_slots: at most {}; erase one first", SLOTS_MAX));
        }
        match &*self.dir.borrow() {
            None => {
                self.mem.borrow_mut().insert(slot.to_string(), text);
                Ok(())
            }
            Some(d) => {
                fs::create_dir_all(d).map_err(|e| format!("cannot make {}: {}", d.display(), e))?;
                // written beside it first and then renamed: a cut write leaves the old save whole
                let p = d.join(format!("{}.save", slot));
                let tmp = d.join(format!("{}.tmp", slot));
                fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {}", tmp.display(), e))?;
                fs::rename(&tmp, &p).map_err(|e| format!("cannot write {}: {}", p.display(), e))
            }
        }
    }

    pub fn erase(&self, slot: &str) -> Result<(), String> {
        match &*self.dir.borrow() {
            None => {
                self.mem.borrow_mut().remove(slot);
                Ok(())
            }
            Some(d) => match fs::remove_file(d.join(format!("{}.save", slot))) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            },
        }
    }

    // what went wrong since the last time, once
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut *self.errors.borrow_mut())
    }
}

// gives the world of a host ecs.save, ecs.load, ecs.slots and ecs.erase
pub fn attach(host: &Host) -> Result<Rc<Saves>, String> {
    let s = Rc::new(Saves::default());
    let lua = host.lua();
    let err = |e: mlua::Error| e.to_string();
    let sv = Rc::clone(&s);
    let save = lua
        .create_function(move |_, slot: String| {
            if !slot_ok(&slot) {
                return Err(bad_slot(&slot));
            }
            sv.jobs.borrow_mut().push(Job::Save(slot));
            Ok(true)
        })
        .map_err(err)?;
    host.add_to_ecs("save", save)?;
    let sv = Rc::clone(&s);
    let load = lua
        .create_function(move |_, slot: String| {
            if !slot_ok(&slot) {
                return Err(bad_slot(&slot));
            }
            if !sv.slots().contains(&slot) {
                return Ok(false);
            }
            sv.jobs.borrow_mut().push(Job::Load(slot));
            Ok(true)
        })
        .map_err(err)?;
    host.add_to_ecs("load", load)?;
    let sv = Rc::clone(&s);
    let slots = lua.create_function(move |_, ()| Ok(sv.slots())).map_err(err)?;
    host.add_to_ecs("slots", slots)?;
    let sv = Rc::clone(&s);
    let erase = lua
        .create_function(move |_, slot: String| {
            if !slot_ok(&slot) {
                return Err(bad_slot(&slot));
            }
            sv.erase(&slot).map_err(mlua::Error::RuntimeError)?;
            Ok(true)
        })
        .map_err(err)?;
    host.add_to_ecs("erase", erase)?;
    Ok(s)
}

// after a tick: the saves and loads asked for during it, in their order
pub fn after_tick(host: &Host, s: &Saves) {
    let jobs = std::mem::take(&mut *s.jobs.borrow_mut());
    for job in jobs {
        let r = match job {
            Job::Save(slot) => {
                let text = encode(&host.world.borrow());
                text.and_then(|t| s.write(&slot, t)).map_err(|e| format!("save {}: {}", slot, e))
            }
            Job::Load(slot) => match s.read(&slot) {
                Ok(text) => as_core(host, |w| apply(w, &text)).map_err(|e| format!("load {}: {}", slot, e)),
                Err(e) => Err(format!("load {}: {}", slot, e)),
            },
        };
        if let Err(e) = r {
            s.errors.borrow_mut().push(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::driver::Driver;
    use crate::project::{build, App};

    fn game(score_kind: &str) -> App {
        let mut files = BTreeMap::new();
        files.insert(String::from("world.lua"), format!("return {{ types = {{ score = '{}', name = 'text' }}, entities = {{ p = {{ score = 1, name = 'ann' }} }} }}", score_kind));
        files.insert(String::from("sys/io.lua"), String::from("return function() if input.pressed('s') then ecs.save('one') end if input.pressed('l') then ecs.load('one') end end"));
        let (app, _, problems) = build(&files).unwrap();
        assert!(problems.is_empty(), "{:?}", problems);
        app
    }

    #[test]
    fn a_game_saves_and_loads_its_data_not_its_code() {
        let app = game("num");
        let code = "set('p', 'score', 5) set('b', 'score', 7) press('s') step() \
            set('p', 'score', 9) set('p', 'name', 'bob') set('c', 'score', 3) press('l') step() \
            return get('p', 'score'), get('p', 'name'), get('b', 'score'), get('c', 'score')";
        let out = Driver::new(&app).unwrap().run("t", code).unwrap();
        assert_eq!(out.trim(), "5\tann\t7\tnil", "{}", out);
        assert_eq!(app.saves.slots(), vec![String::from("one")]);
        // the code is not in the save
        let text = encode(&app.host.world.borrow()).unwrap();
        assert!(!text.contains("sys.io") && !text.contains("\"src\""), "{}", text);
    }

    #[test]
    fn a_save_that_does_not_fit_the_types_changes_nothing() {
        let a = game("num");
        let text = encode(&a.host.world.borrow()).unwrap();
        let b = game("text");
        as_core(&b.host, |w| w.set("p", "score", "high")).unwrap();
        let r = as_core(&b.host, |w| apply(w, &text));
        let e = r.unwrap_err();
        assert!(e.contains("score is num in the save, text in the game"), "{}", e);
        assert_eq!(b.host.world.borrow().own("p", "score"), Some("high"));
        assert!(as_core(&b.host, |w| apply(w, "nonsense")).is_err());
    }

    #[test]
    fn saves_go_to_their_folder_and_are_counted() {
        let d = std::env::temp_dir().join(format!("lust-saves-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let app = game("num");
        app.saves.set_dir(Some(d.clone()));
        Driver::new(&app).unwrap().run("t", "press('s') step()").unwrap();
        assert!(d.join("one.save").is_file());
        for i in 0..SLOTS_MAX {
            app.saves.write(&format!("s{}", i), String::from("{}")).ok();
        }
        assert!(app.saves.write("more", String::from("{}")).is_err());
        assert!(app.saves.write("one", String::from("{}")).is_ok());
        app.saves.erase("one").unwrap();
        assert!(!d.join("one.save").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_slot_has_a_short_plain_name() {
        for ok in ["a", "slot1", "auto_save", "level-3"] {
            assert!(slot_ok(ok), "{}", ok);
        }
        for bad in ["", "A", "../x", "a b", "a.save", &"x".repeat(33)] {
            assert!(!slot_ok(bad), "{}", bad);
        }
    }

    #[test]
    fn a_data_folder_has_a_safe_name() {
        if let Some(d) = dir_for("../My Game") {
            let last = d.file_name().unwrap().to_string_lossy().to_string();
            assert!(!last.contains('/') && !last.contains(".."), "{}", last);
        }
        assert!(dir_for("...").is_none());
    }
}
