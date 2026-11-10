// a project: a folder of lua files that makes one world. the program (lust dev, lust run)
// and an AI over MCP (dev.rs) change only these files; nothing of the rust core changes.
//   world.lua      the start: the spec, the types, the named entities, nameless ones
//   sys/<n>.lua    a system, entity sys.<n> with tick = true: return function(dt) ... end
//   draw/<n>.lua   a drawing, entity draw.<n> with draw = true, read-only, after the ticks
//   rule/<n>.lua   a rule, entity rule.<n> with system = true, after every operation
//   op/<n>.lua     an operation, entity op.<n> (params, roles, desc in world.lua)
//   view/<n>.lua   a view, entity view.<n>
//   lib/<n>.lua    shared code, entity lib.<n>, reached with ecs.call('lib.<n>')
//   test/<n>.lua   a test: driver code (driver.rs), never part of the world
//   <n>.md         notes
// a change is hot: the world keeps its values, the new code runs from the next tick (the
// host compiles code again when its text changed). a change of world.lua applies what
// changed in it. restart builds the world again from the files.
// every change made through Project::write is kept in .lust/history, so undo takes it back.
use crate::ecs::{Kind, Spec, World, CODE_MAX, CORE_ROLE};
use crate::host::{clip, new_sandbox, reason_of, Host};
use crate::screen::{self, Input, Screen};
use mlua::LuaSerdeExt;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub const FILE_MAX: usize = CODE_MAX;
pub const FILES_MAX: usize = 2000;
pub const HISTORY_MAX: usize = 500;
const HISTORY_DIR: &str = ".lust/history";

// the folders of code: the prefix of the entity and the flag that makes it run
pub const CODE_DIRS: &[(&str, Option<&str>)] = &[("sys", Some("tick")), ("draw", Some("draw")), ("rule", Some("system")), ("op", None), ("view", None), ("lib", None)];
pub const TEST_DIR: &str = "test";

// code every project world has, as entities, unless the project has a file of the same
// name: lib.ui (src/lua/ui.lua)
pub const BUILTIN: &[(&str, &str)] = &[("lib.ui", include_str!("lua/ui.lua"))];

// the types every project world has, besides the genesis types of a world
const PROJECT_TYPES: &[(&str, &str)] = &[("src", "code"), ("tick", "bool"), ("draw", "bool"), ("system", "bool"), ("params", "text"), ("roles", "names")];

fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

// a path an AI may write: world.lua, <n>.md, <dir>/<n>.lua
pub fn valid_path(p: &str) -> bool {
    if p == "world.lua" {
        return true;
    }
    if let Some(stem) = p.strip_suffix(".md") {
        return name_ok(stem);
    }
    match p.split_once('/') {
        Some((dir, file)) => {
            let known = dir == TEST_DIR || CODE_DIRS.iter().any(|(d, _)| *d == dir);
            known && file.strip_suffix(".lua").is_some_and(name_ok)
        }
        None => false,
    }
}

// the entity of a code file and its flag; None for world.lua, notes and tests
pub fn entity_of(path: &str) -> Option<(String, Option<&'static str>)> {
    let (dir, file) = path.split_once('/')?;
    let name = file.strip_suffix(".lua")?;
    let (d, flag) = CODE_DIRS.iter().find(|(d, _)| *d == dir)?;
    Some((format!("{}.{}", d, name), *flag))
}

// ---------- world.lua ----------

// what world.lua describes
#[derive(Clone, Debug, PartialEq)]
pub struct Desc {
    pub spec: Spec,
    pub run_ms: Option<u32>,
    // type -> its components (kind, write, desc, ...)
    pub types: BTreeMap<String, BTreeMap<String, String>>,
    // (entity, component) -> value
    pub values: BTreeMap<(String, String), String>,
    pub spawn: Vec<BTreeMap<String, String>>,
}

impl Default for Desc {
    fn default() -> Self {
        Desc { spec: Spec::game(), run_ms: None, types: BTreeMap::new(), values: BTreeMap::new(), spawn: Vec::new() }
    }
}

fn text_of(v: &Json, at: &str) -> Result<String, String> {
    match v {
        Json::String(s) => Ok(s.clone()),
        Json::Bool(b) => Ok(b.to_string()),
        Json::Number(n) => Ok(match n.as_i64() {
            Some(i) => i.to_string(),
            None => format!("{}", n.as_f64().unwrap_or(0.0)),
        }),
        Json::Array(a) => {
            let parts: Result<Vec<String>, String> = a.iter().map(|x| text_of(x, at)).collect();
            Ok(parts?.join(" "))
        }
        _ => Err(format!("world.lua: {}: a value is a string, a number, a boolean or a list", at)),
    }
}

fn object<'a>(v: &'a Json, at: &str) -> Result<Option<&'a serde_json::Map<String, Json>>, String> {
    match v {
        Json::Object(m) => Ok(Some(m)),
        // an empty lua table
        Json::Array(a) if a.is_empty() => Ok(None),
        Json::Null => Ok(None),
        _ => Err(format!("world.lua: {} must be a table with names", at)),
    }
}

fn spec_of(v: &Json) -> Result<Spec, String> {
    match v {
        Json::String(s) if s == "game" => Ok(Spec::game()),
        Json::String(s) if s == "ledger" => Ok(Spec::ledger()),
        Json::Object(m) => {
            let base = match m.get("base").and_then(|b| b.as_str()) {
                Some("ledger") => Spec::ledger(),
                _ => Spec::game(),
            };
            let flag = |k: &str, d: bool| m.get(k).and_then(|x| x.as_bool()).unwrap_or(d);
            let mut s = Spec { journal: flag("journal", base.journal), roles: flag("roles", base.roles), undo: flag("undo", base.undo), keep_lua: flag("keep_lua", base.keep_lua), ..base };
            if let Some(mb) = m.get("memory_mb").and_then(|x| x.as_u64()) {
                s.memory = (mb.clamp(1, 4096) as usize) * 1024 * 1024;
            }
            for k in m.keys() {
                if !["base", "journal", "roles", "undo", "keep_lua", "memory_mb"].contains(&k.as_str()) {
                    return Err(format!("world.lua: spec: unknown key {}", clip(k, 40)));
                }
            }
            Ok(s)
        }
        _ => Err(String::from("world.lua: spec is 'game', 'ledger' or a table")),
    }
}

// evaluates world.lua in a sandbox of its own: it is lua, so loops may build entities
pub fn parse_world(src: &str) -> Result<Desc, String> {
    let (lua, budget) = new_sandbox().map_err(|e| reason_of(&e))?;
    budget.set_run_ms(1000);
    budget.arm();
    let v: mlua::Value = lua.load(src).set_name("world.lua").eval().map_err(|e| reason_of(&e).replace("[string \"world.lua\"]", "world.lua"))?;
    if v.is_nil() {
        return Err(String::from("world.lua returns nothing: return { ... }"));
    }
    let json: Json = lua.from_value(v).map_err(|e| format!("world.lua: {}", clip(&e.to_string(), 120)))?;
    let top = object(&json, "the table world.lua returns")?.cloned().unwrap_or_default();
    let mut d = Desc::default();
    for (k, v) in &top {
        match k.as_str() {
            "spec" => d.spec = spec_of(v)?,
            "run_ms" => d.run_ms = Some(v.as_u64().ok_or("world.lua: run_ms is a number")?.clamp(1, 60_000) as u32),
            "types" => {
                for (t, meta) in object(v, "types")?.cloned().unwrap_or_default() {
                    let mut m = BTreeMap::new();
                    match &meta {
                        Json::String(kind) => {
                            m.insert(String::from("kind"), kind.clone());
                        }
                        _ => {
                            for (c, x) in object(&meta, &format!("types.{}", t))?.cloned().unwrap_or_default() {
                                m.insert(c.clone(), text_of(&x, &format!("types.{}.{}", t, c))?);
                            }
                        }
                    }
                    if !m.contains_key("kind") {
                        return Err(format!("world.lua: types.{} has no kind", clip(&t, 40)));
                    }
                    d.types.insert(t, m);
                }
            }
            "entities" => {
                for (e, comps) in object(v, "entities")?.cloned().unwrap_or_default() {
                    for (c, x) in object(&comps, &format!("entities.{}", e))?.cloned().unwrap_or_default() {
                        let t = text_of(&x, &format!("entities.{}.{}", e, c))?;
                        d.values.insert((e.clone(), c), t);
                    }
                }
            }
            "spawn" => {
                let list = match v {
                    Json::Array(a) => a.clone(),
                    Json::Null => Vec::new(),
                    _ => return Err(String::from("world.lua: spawn is a list of tables")),
                };
                for (i, comps) in list.iter().enumerate() {
                    let mut m = BTreeMap::new();
                    for (c, x) in object(comps, &format!("spawn[{}]", i + 1))?.cloned().unwrap_or_default() {
                        m.insert(c.clone(), text_of(&x, &format!("spawn[{}].{}", i + 1, c))?);
                    }
                    d.spawn.push(m);
                }
            }
            other => return Err(format!("world.lua: unknown key {} (spec, types, entities, spawn, run_ms)", clip(other, 40))),
        }
    }
    Ok(d)
}

// ---------- a world built from the files ----------

// a world made from a project, with its screen. files: entity -> the file of its code,
// so an error can name the file
pub struct App {
    pub host: Rc<Host>,
    pub screen: Rc<Screen>,
    // ecs.save and ecs.load (saves.rs); in memory until a session gives it a folder
    pub saves: Rc<crate::saves::Saves>,
    pub files: BTreeMap<String, String>,
}

// the file and line instead of the entity: [string "sys.ball"]:12 -> sys/ball.lua:12
pub fn explain(files: &BTreeMap<String, String>, msg: &str) -> String {
    let mut m = msg.to_string();
    for (e, path) in files {
        m = m.replace(&format!("[string \"{}\"]", e), path);
        m = m.replace(&format!("{}: {}", e, path), path);
        if let Some(rest) = m.strip_prefix(&format!("{}: ", e)) {
            m = format!("{}: {}", path, rest);
        }
    }
    m
}

impl App {
    pub fn explain(&self, msg: &str) -> String {
        explain(&self.files, msg)
    }

    // one frame of the world: the tick with this input, then the drawing
    pub fn frame(&self, dt: f64, input: Input) -> (Result<(), String>, Result<String, String>) {
        *self.screen.input.borrow_mut() = input;
        let t = self.host.tick(dt).map_err(|e| self.explain(&e));
        screen::after_tick(&self.screen);
        crate::saves::after_tick(&self.host, &self.saves);
        let d = screen::draw(&self.host, &self.screen).map_err(|e| self.explain(&e));
        (t, d)
    }
}

// a change of the world made by the developer: as the core role, which every right allows
pub fn as_core<T>(host: &Host, f: impl FnOnce(&mut World) -> T) -> T {
    let mut w = host.world.borrow_mut();
    let role = std::mem::replace(&mut w.current_role, CORE_ROLE.to_string());
    let op = std::mem::replace(&mut w.current_op, String::from("dev"));
    let out = f(&mut w);
    w.current_role = role;
    w.current_op = op;
    out
}

fn set_types(w: &mut World, old: Option<&BTreeMap<String, BTreeMap<String, String>>>, new: &BTreeMap<String, BTreeMap<String, String>>, out: &mut Vec<String>) {
    for (t, meta) in new {
        let before = old.and_then(|o| o.get(t));
        if before == Some(meta) {
            continue;
        }
        // the kind first: the other components of a type need it to exist
        let mut order: Vec<(&String, &String)> = meta.iter().filter(|(c, _)| c.as_str() == "kind").collect();
        order.extend(meta.iter().filter(|(c, _)| c.as_str() != "kind"));
        for (c, v) in order {
            if before.and_then(|b| b.get(c)) == Some(v) {
                continue;
            }
            if let Err(e) = w.set(t, c, v) {
                out.push(format!("world.lua: types.{}.{}: {}", t, c, e));
            }
        }
        if let Some(b) = before {
            for c in b.keys() {
                if !meta.contains_key(c) && c != "kind" {
                    if let Err(e) = w.del(t, c) {
                        out.push(format!("world.lua: types.{}.{}: {}", t, c, e));
                    }
                }
            }
        }
    }
}

fn set_values(w: &mut World, old: Option<&BTreeMap<(String, String), String>>, new: &BTreeMap<(String, String), String>, out: &mut Vec<String>) {
    // references last: they name entities that must exist first
    for refs in [false, true] {
        for ((e, c), v) in new {
            if (w.kind_of(c) == Some(Kind::Ref)) != refs || old.and_then(|o| o.get(&(e.clone(), c.clone()))) == Some(v) {
                continue;
            }
            if let Err(err) = w.set(e, c, v) {
                out.push(format!("world.lua: entities.{}.{}: {}", e, c, err));
            }
        }
    }
    if let Some(o) = old {
        for (e, c) in o.keys() {
            if !new.contains_key(&(e.clone(), c.clone())) && w.own(e, c).is_some() {
                if let Err(err) = w.del(e, c) {
                    out.push(format!("world.lua: entities.{}.{}: {}", e, c, err));
                }
            }
        }
    }
}

fn spawn_all(w: &mut World, list: &[BTreeMap<String, String>], out: &mut Vec<String>) {
    for (i, comps) in list.iter().enumerate() {
        let name = w.new_anon();
        for (c, v) in comps {
            if let Err(e) = w.set(&name, c, v) {
                out.push(format!("world.lua: spawn[{}].{}: {}", i + 1, c, e));
            }
        }
    }
}

// the code of one file into the world. compiled first: code that does not compile is
// refused and the old code stays
pub fn install(host: &Host, files: &BTreeMap<String, String>, path: &str, src: &str) -> Result<(), String> {
    let (entity, flag) = match entity_of(path) {
        Some(x) => x,
        None => return Ok(()),
    };
    if src.trim().is_empty() {
        return Err(format!("{}: the file is empty", path));
    }
    host.check_code(&entity, src).map_err(|e| explain(files, &e))?;
    as_core(host, |w| {
        w.set(&entity, "src", src)?;
        if let Some(f) = flag {
            w.set(&entity, f, "true")?;
        }
        Ok::<(), String>(())
    })
    .map_err(|e| format!("{}: {}", path, e))
}

// a code file is gone: its entity loses its code and its flag (what world.lua gives it stays)
pub fn uninstall(host: &Host, path: &str) {
    if let Some((entity, flag)) = entity_of(path) {
        as_core(host, |w| {
            if w.own(&entity, "src").is_some() {
                // the built-in code comes back when the project's own file is gone
                match BUILTIN.iter().find(|(e, _)| *e == entity) {
                    Some((_, src)) => {
                        let _ = w.set(&entity, "src", src);
                    }
                    None => {
                        let _ = w.del(&entity, "src");
                    }
                }
            }
            if let Some(f) = flag {
                if w.own(&entity, f).is_some() {
                    let _ = w.del(&entity, f);
                }
            }
        });
    }
}

fn code_files(files: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    files.keys().filter_map(|p| entity_of(p).map(|(e, _)| (e, p.clone()))).collect()
}

// a world from these files. Err only when world.lua cannot be read; the problems of single
// files come back with the world
pub fn build(files: &BTreeMap<String, String>) -> Result<(App, Desc, Vec<String>), String> {
    let desc = match files.get("world.lua") {
        Some(src) => parse_world(src)?,
        None => Desc::default(),
    };
    let host = Host::with_spec(desc.spec.clone()).map_err(|e| reason_of(&e))?;
    let screen = screen::attach(&host)?;
    let saves = crate::saves::attach(&host)?;
    if let Some(ms) = desc.run_ms {
        host.budget.set_run_ms(ms);
    }
    let mut out = Vec::new();
    as_core(&host, |w| {
        for (t, k) in PROJECT_TYPES {
            if let Err(e) = w.set(t, "kind", k) {
                out.push(format!("type {}: {}", t, e));
            }
        }
        set_types(w, None, &desc.types, &mut out);
        set_values(w, None, &desc.values, &mut out);
        spawn_all(w, &desc.spawn, &mut out);
    });
    let names = code_files(files);
    for (entity, src) in BUILTIN {
        if !names.contains_key(*entity) {
            if let Err(e) = as_core(&host, |w| w.set(entity, "src", src)) {
                out.push(format!("{}: {}", entity, e));
            }
        }
    }
    for (path, src) in files {
        if let Err(e) = install(&host, &names, path, src) {
            out.push(e);
        }
    }
    Ok((App { host: Rc::new(host), screen, saves, files: names }, desc, out))
}

// ---------- the folder ----------

pub struct Project {
    pub root: PathBuf,
    // what the live world was last given, per file
    seen: BTreeMap<String, String>,
    // the world.lua the live world was given; None when it could not be read
    desc: Option<Desc>,
    // files whose next change is already in the history (or is an undo)
    noted: BTreeSet<String>,
}

fn read_text(p: &Path) -> Result<String, String> {
    let bytes = fs::read(p).map_err(|e| format!("cannot read {}: {}", p.display(), e))?;
    if bytes.len() > FILE_MAX {
        return Err(format!("{}: more than {} bytes", p.display(), FILE_MAX));
    }
    String::from_utf8(bytes).map_err(|_| format!("{}: not utf-8", p.display()))
}

impl Project {
    // the folder must exist: the person chose it
    pub fn open(root: &Path) -> Result<Project, String> {
        if !root.is_dir() {
            return Err(format!("no folder {}", root.display()));
        }
        Ok(Project { root: root.to_path_buf(), seen: BTreeMap::new(), desc: None, noted: BTreeSet::new() })
    }

    // the files of the project on disk; other files are left alone (and named in skipped)
    pub fn scan(&self) -> Result<(BTreeMap<String, String>, Vec<String>), String> {
        let mut files = BTreeMap::new();
        let mut skipped = Vec::new();
        let mut dirs = vec![String::new()];
        dirs.extend(CODE_DIRS.iter().map(|(d, _)| d.to_string()));
        dirs.push(TEST_DIR.to_string());
        for d in dirs {
            let dir = if d.is_empty() { self.root.clone() } else { self.root.join(&d) };
            let list = match fs::read_dir(&dir) {
                Ok(l) => l,
                Err(_) => continue,
            };
            let mut names: Vec<String> = list.filter_map(|x| x.ok()).filter(|x| x.path().is_file()).map(|x| x.file_name().to_string_lossy().to_string()).collect();
            names.sort();
            for n in names {
                let rel = if d.is_empty() { n.clone() } else { format!("{}/{}", d, n) };
                if !valid_path(&rel) {
                    skipped.push(rel);
                    continue;
                }
                if files.len() >= FILES_MAX {
                    return Err(format!("more than {} files", FILES_MAX));
                }
                match read_text(&dir.join(&n)) {
                    Ok(t) => {
                        files.insert(rel, t);
                    }
                    Err(e) => skipped.push(format!("{} ({})", rel, e)),
                }
            }
        }
        Ok((files, skipped))
    }

    pub fn read(&self, path: &str) -> Result<String, String> {
        if !valid_path(path) {
            return Err(format!("bad_path: {}", clip(path, 60)));
        }
        read_text(&self.root.join(path))
    }

    fn history_dir(&self) -> PathBuf {
        self.root.join(HISTORY_DIR)
    }

    fn history(&self) -> Vec<(u64, PathBuf)> {
        let mut list: Vec<(u64, PathBuf)> = match fs::read_dir(self.history_dir()) {
            Ok(l) => l.filter_map(|x| x.ok()).filter_map(|x| x.file_name().to_string_lossy().parse::<u64>().ok().map(|n| (n, x.path()))).collect(),
            Err(_) => Vec::new(),
        };
        list.sort();
        list
    }

    // writes (or with None deletes) one file; what was there goes to the history first
    pub fn write(&mut self, path: &str, text: Option<&str>) -> Result<(), String> {
        if !valid_path(path) {
            return Err(format!("bad_path: {} (world.lua, <name>.md or sys|draw|rule|op|view|lib|test/<name>.lua, names [a-z0-9_])", clip(path, 60)));
        }
        if let Some(t) = text {
            if t.len() > FILE_MAX {
                return Err(format!("too_big: at most {} bytes", FILE_MAX));
            }
        }
        let full = self.root.join(path);
        let old = if full.is_file() { Some(read_text(&full)?) } else { None };
        if old.as_deref() == text {
            return Ok(());
        }
        if old.is_none() && text.is_none() {
            return Err(format!("no_file: {}", path));
        }
        let hist = self.history();
        let next = hist.last().map(|(n, _)| n + 1).unwrap_or(1);
        fs::create_dir_all(self.history_dir()).map_err(|e| e.to_string())?;
        let record = format!("{} {}\n{}", path, if old.is_some() { 1 } else { 0 }, old.unwrap_or_default());
        fs::write(self.history_dir().join(format!("{:06}", next)), record).map_err(|e| e.to_string())?;
        if hist.len() >= HISTORY_MAX {
            for (_, p) in &hist[..hist.len() + 1 - HISTORY_MAX] {
                let _ = fs::remove_file(p);
            }
        }
        match text {
            Some(t) => {
                if let Some(parent) = full.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::write(&full, t).map_err(|e| e.to_string())?;
            }
            None => fs::remove_file(&full).map_err(|e| e.to_string())?,
        }
        self.noted.insert(path.to_string());
        Ok(())
    }

    // the last n changes back, newest first; the files they touched
    pub fn undo(&mut self, n: usize) -> Result<Vec<String>, String> {
        let mut hist = self.history();
        let mut out = Vec::new();
        for _ in 0..n {
            let (_, p) = match hist.pop() {
                Some(x) => x,
                None => break,
            };
            let text = read_text(&p)?;
            let (head, body) = text.split_once('\n').unwrap_or((text.as_str(), ""));
            let (path, existed) = head.rsplit_once(' ').ok_or("broken history")?;
            if !valid_path(path) {
                return Err(String::from("broken history"));
            }
            let full = self.root.join(path);
            if existed == "1" {
                if let Some(parent) = full.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::write(&full, body).map_err(|e| e.to_string())?;
            } else if full.is_file() {
                fs::remove_file(&full).map_err(|e| e.to_string())?;
            }
            fs::remove_file(&p).map_err(|e| e.to_string())?;
            self.noted.insert(path.to_string());
            out.push(path.to_string());
        }
        if out.is_empty() {
            return Err(String::from("nothing to undo"));
        }
        Ok(out)
    }

    // a world from the files on disk, remembered as the live one
    pub fn build(&mut self) -> Result<(App, Vec<String>), String> {
        let (files, _) = self.scan()?;
        self.seen = files.clone();
        self.noted.clear();
        match build(&files) {
            Ok((app, desc, problems)) => {
                self.desc = Some(desc);
                Ok((app, problems))
            }
            Err(e) => {
                self.desc = None;
                Err(e)
            }
        }
    }

    // a world from the files on disk that nothing remembers: for a try or a test
    pub fn fresh(&self) -> Result<(App, Vec<String>), String> {
        let (files, _) = self.scan()?;
        build(&files).map(|(app, _, p)| (app, p))
    }

    // the changes on disk since the live world was last given its files, into that world.
    // a change by the person (an editor) goes to the history too. returns what happened,
    // one line a file; Restart when world.lua must build the world again
    pub fn sync(&mut self, app: &mut App) -> Result<Vec<String>, String> {
        let (files, _) = self.scan()?;
        let mut changed: Vec<String> = files.iter().filter(|(p, t)| self.seen.get(*p) != Some(t)).map(|(p, _)| p.clone()).collect();
        changed.extend(self.seen.keys().filter(|p| !files.contains_key(*p)).cloned());
        let mut out = Vec::new();
        if changed.is_empty() {
            return Ok(out);
        }
        // a change by the person: keep what was there, so undo can take it back
        for p in &changed {
            if !self.noted.remove(p) {
                let old = self.seen.get(p).cloned();
                self.note_history(p, old);
            }
        }
        app.files = code_files(&files);
        let world_changed = changed.iter().any(|p| p == "world.lua");
        if world_changed {
            match files.get("world.lua").map(|s| parse_world(s)).unwrap_or_else(|| Ok(Desc::default())) {
                Err(e) => out.push(e),
                Ok(new) => match &self.desc {
                    None => {
                        out.push(String::from("world.lua: read again: the world starts again"));
                        let (a, problems) = self.build()?;
                        *app = a;
                        out.extend(problems);
                        return Ok(out);
                    }
                    Some(old) => {
                        let mut problems = Vec::new();
                        if old.spec != new.spec {
                            problems.push(String::from("world.lua: the spec changed; lust_restart applies it"));
                        }
                        if old.spawn != new.spawn {
                            problems.push(String::from("world.lua: spawn changed; lust_restart applies it"));
                        }
                        if old.run_ms != new.run_ms {
                            if let Some(ms) = new.run_ms {
                                app.host.budget.set_run_ms(ms);
                            }
                        }
                        as_core(&app.host, |w| {
                            set_types(w, Some(&old.types), &new.types, &mut problems);
                            set_values(w, Some(&old.values), &new.values, &mut problems);
                        });
                        if problems.is_empty() {
                            out.push(String::from("world.lua: applied"));
                        }
                        out.extend(problems);
                        self.desc = Some(new);
                    }
                },
            }
        }
        for p in changed.iter().filter(|p| entity_of(p).is_some()) {
            match files.get(p) {
                Some(src) => match install(&app.host, &app.files, p, src) {
                    Ok(()) => out.push(format!("{}: applied", p)),
                    Err(e) => out.push(format!("{} (the old code stays)", e)),
                },
                None => {
                    uninstall(&app.host, p);
                    out.push(format!("{}: removed", p));
                }
            }
        }
        self.seen = files;
        Ok(out)
    }

    fn note_history(&mut self, path: &str, old: Option<String>) {
        let next = self.history().last().map(|(n, _)| n + 1).unwrap_or(1);
        if fs::create_dir_all(self.history_dir()).is_ok() {
            let record = format!("{} {}\n{}", path, if old.is_some() { 1 } else { 0 }, old.unwrap_or_default());
            let _ = fs::write(self.history_dir().join(format!("{:06}", next)), record);
        }
    }
}

// ---------- a running project ----------

// the live world of a project and why it stopped, if it did
pub struct Session {
    pub project: Project,
    pub app: App,
    pub paused: Option<String>,
    // what the last build or change reported
    pub problems: Vec<String>,
    // where a person sees the world (web.rs), when it is shown
    pub url: Option<String>,
}

impl Session {
    pub fn open(root: &Path) -> Result<Session, String> {
        let mut project = Project::open(root)?;
        let (app, problems) = match project.build() {
            Ok(x) => x,
            Err(e) => {
                let (app, _, _) = build(&BTreeMap::new())?;
                (app, vec![e])
            }
        };
        let s = Session { project, app, paused: None, problems, url: None };
        s.wire();
        Ok(s)
    }

    // the saves of the live world go to the folder of the user's data for this game
    fn wire(&self) {
        self.app.saves.set_dir(crate::saves::dir_for(&crate::saves::id_of(&self.project.root)));
    }

    // the world again from the files
    pub fn restart(&mut self) -> Vec<String> {
        match self.project.build() {
            Ok((app, problems)) => {
                self.app = app;
                self.wire();
                self.paused = None;
                self.problems = problems;
            }
            Err(e) => self.problems = vec![e],
        }
        self.problems.clone()
    }

    // the changes on disk into the live world; a world that stopped runs again
    pub fn sync(&mut self) -> Vec<String> {
        match self.project.sync(&mut self.app) {
            Ok(lines) => {
                if !lines.is_empty() {
                    self.paused = None;
                    self.problems = lines.iter().filter(|l| !l.ends_with(": applied") && !l.ends_with(": removed")).cloned().collect();
                }
                lines
            }
            Err(e) => vec![e],
        }
    }

    // one frame of the live world; it stops at the first error and keeps drawing
    pub fn frame(&mut self, dt: f64, input: Input) -> String {
        if self.paused.is_some() {
            *self.app.screen.input.borrow_mut() = input;
            return match screen::draw(&self.app.host, &self.app.screen) {
                Ok(f) => f,
                Err(_) => String::from("size 800 600\n"),
            };
        }
        let (t, d) = self.app.frame(dt, input);
        if let Err(e) = t {
            self.paused = Some(format!("tick {}: {}", self.app.host.ticks(), e));
        }
        // a save that failed does not stop the game; it is told on the status line
        let failed = self.app.saves.take_errors();
        if !failed.is_empty() {
            self.problems = failed;
        }
        match d {
            Ok(f) => f,
            Err(e) => {
                if self.paused.is_none() {
                    self.paused = Some(format!("draw: {}", e));
                }
                String::from("size 800 600\n")
            }
        }
    }

    pub fn status(&self) -> String {
        match &self.paused {
            Some(why) => format!("live: stopped at {}", why),
            None => format!("live: tick {}, running", self.app.host.ticks()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("lust-project-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    const WORLD: &str = "return { types = { x = 'num', vx = 'num' }, entities = { ball = { x = 0, vx = 2 } } }";
    const MOVE: &str = "return function(dt) ecs.set('ball', 'x', ecs.get('ball', 'x') + ecs.get('ball', 'vx') * dt) end";

    #[test]
    fn paths_are_checked() {
        for ok in ["world.lua", "notes.md", "sys/ball.lua", "draw/main_2.lua", "test/a.lua", "lib/vec.lua"] {
            assert!(valid_path(ok), "{}", ok);
        }
        for bad in ["../x.lua", "sys/../x.lua", "sys/Ball.lua", "src/main.rs", "sys/a/b.lua", "/etc/passwd", "sys/.lua", "x.lua", ".lust/history/000001", "C:\\x.lua"] {
            assert!(!valid_path(bad), "{}", bad);
        }
        assert_eq!(entity_of("sys/ball.lua"), Some((String::from("sys.ball"), Some("tick"))));
        assert_eq!(entity_of("test/a.lua"), None);
    }

    #[test]
    fn world_lua_is_lua_and_typos_are_named() {
        let d = parse_world("local e = {} for i = 1, 3 do e['b' .. i] = { x = i * 0.5 } end return { types = { x = 'num' }, entities = e, spawn = { { x = 1 } } }").unwrap();
        assert_eq!(d.values.get(&(String::from("b3"), String::from("x"))).map(|s| s.as_str()), Some("1.5"));
        assert_eq!(d.spawn.len(), 1);
        assert!(parse_world("return { entites = {} }").unwrap_err().contains("unknown key entites"));
        assert!(parse_world("return { types = { x = {} } }").unwrap_err().contains("no kind"));
        assert!(parse_world("return {").unwrap_err().contains("world.lua"));
    }

    #[test]
    fn a_folder_becomes_a_world_and_a_change_is_hot() {
        let root = folder("hot");
        fs::write(root.join("world.lua"), WORLD).unwrap();
        fs::create_dir_all(root.join("sys")).unwrap();
        fs::write(root.join("sys/move.lua"), MOVE).unwrap();
        let mut s = Session::open(&root).unwrap();
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        for _ in 0..10 {
            s.frame(0.5, Input::default());
        }
        assert_eq!(s.app.host.world.borrow().value_text("ball", "x").as_deref(), Some("10"));
        // the code changes, the value stays: the next tick runs the new code
        s.project.write("sys/move.lua", Some(&MOVE.replace("* dt", "* dt * 10"))).unwrap();
        assert_eq!(s.sync(), vec![String::from("sys/move.lua: applied")]);
        s.frame(0.5, Input::default());
        assert_eq!(s.app.host.world.borrow().value_text("ball", "x").as_deref(), Some("20"));
        // world.lua: what changed in it is applied, the rest of the state stays
        s.project.write("world.lua", Some(&WORLD.replace("vx = 2", "vx = 4"))).unwrap();
        assert_eq!(s.sync(), vec![String::from("world.lua: applied")]);
        s.frame(0.5, Input::default());
        assert_eq!(s.app.host.world.borrow().value_text("ball", "x").as_deref(), Some("40"));
        // code that does not compile is refused, the old code goes on
        fs::write(root.join("sys/move.lua"), "return function(dt) ecs.set( end").unwrap();
        let lines = s.sync();
        assert!(lines[0].contains("sys/move.lua:1:") && lines[0].contains("old code stays"), "{:?}", lines);
        s.frame(0.5, Input::default());
        assert_eq!(s.app.host.world.borrow().value_text("ball", "x").as_deref(), Some("60"));
        // a runtime error stops the live world and names the file and the line
        fs::write(root.join("sys/move.lua"), "return function(dt)\n  ecs.set('ball', 'x', nil + 1)\nend").unwrap();
        s.sync();
        s.frame(0.5, Input::default());
        let why = s.paused.clone().unwrap();
        assert!(why.contains("sys/move.lua:2:"), "{}", why);
        // undo: the person's broken edits are taken back one by one
        let _ = s.project.undo(2).unwrap();
        s.sync();
        assert!(s.paused.is_none());
        s.frame(0.5, Input::default());
        assert_eq!(s.app.host.world.borrow().value_text("ball", "x").as_deref(), Some("80"));
        // restart: from the start of world.lua
        s.restart();
        assert_eq!(s.app.host.world.borrow().value_text("ball", "x").as_deref(), Some("0"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_deleted_file_stops_its_code_and_undo_brings_it_back() {
        let root = folder("delete");
        fs::write(root.join("world.lua"), WORLD).unwrap();
        let mut s = Session::open(&root).unwrap();
        s.project.write("sys/move.lua", Some(MOVE)).unwrap();
        s.sync();
        s.frame(1.0, Input::default());
        s.project.write("sys/move.lua", None).unwrap();
        assert_eq!(s.sync(), vec![String::from("sys/move.lua: removed")]);
        s.frame(1.0, Input::default());
        assert_eq!(s.app.host.world.borrow().value_text("ball", "x").as_deref(), Some("2"));
        assert_eq!(s.project.undo(1).unwrap(), vec![String::from("sys/move.lua")]);
        s.sync();
        s.frame(1.0, Input::default());
        assert_eq!(s.app.host.world.borrow().value_text("ball", "x").as_deref(), Some("4"));
        assert!(s.project.write("../evil.lua", Some("x")).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_ledger_project_has_roles_operations_and_rights() {
        let root = folder("ledger");
        fs::write(
            root.join("world.lua"),
            "return { spec = 'ledger', types = { stock = { kind = 'int', write = 'clerk' } },
              entities = { ['role.clerk'] = { desc = 'sells' }, tea = { stock = 5 },
                           ['op.sell'] = { params = 'item:id n:int', roles = 'clerk', desc = 'sells n' } } }",
        )
        .unwrap();
        fs::create_dir_all(root.join("op")).unwrap();
        fs::write(root.join("op/sell.lua"), "return function(p) ecs.set(p.item, 'stock', ecs.get(p.item, 'stock') - p.n) end").unwrap();
        let s = Session::open(&root).unwrap();
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        s.app.host.run("clerk", "sell", &[("item", "tea"), ("n", "2")]).unwrap();
        assert_eq!(s.app.host.world.borrow().value("tea", "stock"), Some("3"));
        assert!(s.app.host.run("guest", "sell", &[("item", "tea"), ("n", "1")]).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
