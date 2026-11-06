// lua host: the sandbox, and the code of the world (operations, systems, views and any
// read-only code) that lives in the component "src" of its entity

use crate::ecs::{valid_id, valid_text, Kind, World, CODE_MAX};
use mlua::{Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, Value, VmState};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

const MEMORY_LIMIT: usize = 16 * 1024 * 1024;
// the hook fires every 100 vm instructions; one run may use 10000 hooks (a million
// instructions) and at most RUN_TIME of wall time. the wall clock matters because one
// instruction can be an expensive library call that the instruction count does not see
const HOOK_EVERY: u32 = 100;
pub const HOOK_BUDGET: u32 = 10_000;
const RUN_TIME: Duration = Duration::from_millis(250);
// string.rep may build at most this many bytes per call
const REP_MAX: usize = 1024 * 1024;
const REASON_MAX: usize = 120;

pub fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

pub fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

// the clean reason of a lua error: "no_project", not "runtime error: no_project"
pub fn reason_of(e: &mlua::Error) -> String {
    match e {
        mlua::Error::RuntimeError(msg) => first_line(msg),
        mlua::Error::MemoryError(msg) => format!("memory: {}", first_line(msg)),
        mlua::Error::CallbackError { cause, .. } => reason_of(cause),
        other => first_line(&other.to_string()),
    }
}

// the limits of one run: instructions left and the moment the run must end. the time a
// run may take is set from the settings (limits.run_ms); the instructions grow with it
// (a million for every 250 ms)
pub struct Budget {
    left: Cell<u32>,
    deadline: Cell<Instant>,
    hooks: Cell<u32>,
    time: Cell<Duration>,
}

impl Budget {
    pub fn arm(&self) {
        self.left.set(self.hooks.get());
        self.deadline.set(Instant::now() + self.time.get());
    }

    pub fn set_run_ms(&self, ms: u32) {
        self.time.set(Duration::from_millis(ms as u64));
        self.hooks.set(HOOK_BUDGET.saturating_mul(ms.max(1)) / 250);
    }
}

// a lua state with no system access, a memory limit and a time limit.
// used for the operations and for reading the settings files.
// the caller arms the returned budget before each run
pub fn new_sandbox() -> mlua::Result<(Lua, Rc<Budget>)> {
    // no io, os, package, debug, coroutine: only these libraries exist
    let libs = StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8;
    let lua = Lua::new_with(libs, LuaOptions::default())?;
    lua.set_memory_limit(MEMORY_LIMIT)?;

    // the base library still brings loaders and print: remove them.
    // pcall and xpcall go too: with them a script could catch the time limit
    // and the memory limit errors and keep running forever. getmetatable goes
    // because it reaches the shared string library through the string metatable.
    // operations report failure with error(code, 0); the host rolls back
    let globals = lua.globals();
    for name in ["dofile", "loadfile", "load", "loadstring", "require", "print", "pcall", "xpcall", "collectgarbage", "getmetatable"] {
        globals.set(name, Value::Nil)?;
    }

    // a metatable with __gc gives a finalizer: lua runs it at some later collection, outside
    // the run that made it, as whatever role runs then, with no undo. setmetatable refuses
    // such a table (lua 5.4 marks an object for finalization only when it is set)
    let setmt: Function = globals.get("setmetatable")?;
    let safe_setmt = lua.create_function(move |_, (t, mt): (Table, Option<Table>)| {
        if let Some(m) = &mt {
            if !m.raw_get::<Value>("__gc")?.is_nil() {
                return Err(mlua::Error::runtime("no_gc: a metatable may not have __gc"));
            }
        }
        setmt.call::<Value>((t, mt))
    })?;
    globals.set("setmetatable", safe_setmt)?;

    // lua patterns can run for minutes inside one native call, where the
    // time hook below cannot interrupt them: remove all pattern functions
    // and offer a plain substring search instead (utf-8 text only)
    let string: Table = globals.get("string")?;
    for name in ["find", "match", "gmatch", "gsub"] {
        string.set(name, Value::Nil)?;
    }
    let find_plain = lua.create_function(|_, (s, needle): (String, String)| Ok(s.find(&needle).map(|i| i + 1)))?;
    string.set("find_plain", find_plain)?;

    // string.rep with a size cap, so one call cannot take long
    let rep: Function = string.get("rep")?;
    let capped_rep = lua.create_function(move |_, (s, n, sep): (mlua::LuaString, i64, Option<mlua::LuaString>)| {
        let unit = s.as_bytes().len() + sep.as_ref().map_or(0, |x| x.as_bytes().len());
        if n > 0 && (unit as u128) * (n as u128) > REP_MAX as u128 {
            return Err(mlua::Error::runtime("rep_too_large"));
        }
        rep.call::<mlua::Value>((s, n, sep))
    })?;
    string.set("rep", capped_rep)?;

    // time limit: stop a script that runs too long, for example "while true do end".
    // once the budget is spent every later hook fails again, so nothing can go on
    let budget = Rc::new(Budget {
        left: Cell::new(HOOK_BUDGET),
        deadline: Cell::new(Instant::now() + RUN_TIME),
        hooks: Cell::new(HOOK_BUDGET),
        time: Cell::new(RUN_TIME),
    });
    let hook_budget = Rc::clone(&budget);
    lua.set_hook(HookTriggers::new().every_nth_instruction(HOOK_EVERY), move |_lua, _debug| {
        let left = hook_budget.left.get();
        if left == 0 || Instant::now() > hook_budget.deadline.get() {
            hook_budget.left.set(0);
            return Err(mlua::Error::runtime("time_limit"));
        }
        hook_budget.left.set(left - 1);
        Ok(VmState::Continue)
    })?;

    Ok((lua, budget))
}

// one parameter of an operation, from the component "params" ("ticket:id claim:id note:text?")
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub kind: String, // id, text, int, or long (text up to CODE_MAX bytes, for code)
    pub required: bool,
}

pub fn parse_params(spec: &str) -> Result<Vec<Param>, String> {
    let mut out: Vec<Param> = Vec::new();
    for part in spec.split_whitespace() {
        if part == "-" {
            continue;
        }
        let (name, kind) = part.split_once(':').ok_or_else(|| format!("bad param {}", part))?;
        let required = !kind.ends_with('?');
        let kind = kind.trim_end_matches('?');
        if !valid_id(name) || !["id", "text", "int", "long"].contains(&kind) || out.iter().any(|p| p.name == name) {
            return Err(format!("bad param {}", part));
        }
        out.push(Param { name: name.to_string(), kind: kind.to_string(), required });
    }
    Ok(out)
}

// what a role may run: entities "op.<name>" whose component "roles" names the role
#[derive(Clone, Debug)]
pub struct OpInfo {
    pub name: String,
    pub desc: String,
    pub params: Vec<Param>,
}

pub struct Host {
    lua: Lua,
    pub world: Rc<RefCell<World>>,
    pub budget: Rc<Budget>,
    // the globals every run starts from; each run gets fresh copies,
    // so nothing a script keeps outside the components survives the run
    base: Rc<RefCell<Vec<(String, Value)>>>,
    // what the program built on lust does when an operation is refused or fails (after
    // the run is undone): the role and the reason. None: nothing more than the journal note
    refused: RefCell<Option<Rc<RefusalHook>>>,
}

// the hook of a refusal: the host, the role, the reason
pub type RefusalHook = dyn Fn(&Host, &str, &str);

// a value of the world as lua sees it: numbers, booleans and lists where the kind says so
fn to_lua(lua: &Lua, kind: Option<Kind>, v: &str) -> mlua::Result<Value> {
    Ok(match kind {
        Some(Kind::Int) => Value::Integer(v.parse::<i64>().unwrap_or(0)),
        Some(Kind::Bool) => Value::Boolean(v == "true"),
        Some(Kind::Names) => {
            let t = lua.create_table()?;
            for (i, part) in v.split(' ').enumerate() {
                t.set(i + 1, part)?;
            }
            Value::Table(t)
        }
        _ => Value::String(lua.create_string(v)?),
    })
}

// any lua value back to the text form the world checks
pub fn from_lua(v: Value) -> mlua::Result<String> {
    Ok(match v {
        Value::String(s) => s.to_str()?.to_string(),
        Value::Integer(n) => n.to_string(),
        Value::Number(f) if f.fract() == 0.0 && f.abs() < 1e15 => (f as i64).to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Table(t) => {
            let mut parts = Vec::new();
            for x in t.sequence_values::<String>() {
                parts.push(x?);
            }
            parts.join(" ")
        }
        _ => return Err(mlua::Error::runtime("bad_value")),
    })
}

fn fail(e: String) -> mlua::Error {
    mlua::Error::runtime(e)
}

// the table "ecs": the only way a script reaches the world
// a fresh environment: copies of the base functions and fresh copies of the library tables
fn fresh_env(lua: &Lua, base: &[(String, Value)]) -> mlua::Result<Table> {
    let env = lua.create_table()?;
    for (k, v) in base {
        match v {
            Value::Table(t) => {
                let copy = lua.create_table()?;
                for pair in t.pairs::<Value, Value>() {
                    let (a, b) = pair?;
                    copy.set(a, b)?;
                }
                env.set(k.as_str(), copy)?;
            }
            other => env.set(k.as_str(), other.clone())?,
        }
    }
    Ok(env)
}

// how deep ecs.call may nest
const CALL_DEPTH_MAX: u32 = 8;

fn make_api(lua: &Lua, world: &Rc<RefCell<World>>, base: &Rc<RefCell<Vec<(String, Value)>>>) -> mlua::Result<Table> {
    let api = lua.create_table()?;

    // ecs.get(e, c) -> value, from   (own value, or the one of the prefab chain)
    let w = Rc::clone(world);
    api.set(
        "get",
        lua.create_function(move |lua, (e, c): (String, String)| {
            let w = w.borrow();
            if w.hidden(&c) {
                return Ok((Value::Nil, Value::Nil));
            }
            match w.get(&e, &c) {
                None => Ok((Value::Nil, Value::Nil)),
                Some((v, from)) => Ok((to_lua(lua, w.kind_of(&c), v)?, Value::String(lua.create_string(from)?))),
            }
        })?,
    )?;

    // ecs.own(e, c) -> the own value only
    let w = Rc::clone(world);
    api.set(
        "own",
        lua.create_function(move |lua, (e, c): (String, String)| {
            let w = w.borrow();
            if w.hidden(&c) {
                return Ok(Value::Nil);
            }
            match w.own(&e, &c) {
                None => Ok(Value::Nil),
                Some(v) => to_lua(lua, w.kind_of(&c), v),
            }
        })?,
    )?;

    // ecs.set(e, c, v): raises on any refusal; the host then undoes the whole run
    let w = Rc::clone(world);
    api.set(
        "set",
        lua.create_function(move |lua, (e, c, v): (String, String, Value)| {
            let text = from_lua(v)?;
            if w.borrow().kind_of(&c) == Some(Kind::Code) {
                // code is checked when it is written, not when it first runs
                lua.load(&text).set_name(&e).into_function().map_err(|err| fail(format!("bad_code: {}", clip(&reason_of(&err), 80))))?;
            }
            w.borrow_mut().set(&e, &c, &text).map_err(fail)
        })?,
    )?;

    // ecs.del(e, c): removes an own value
    let w = Rc::clone(world);
    api.set("del", lua.create_function(move |_, (e, c): (String, String)| w.borrow_mut().del(&e, &c).map_err(fail))?)?;

    // ecs.kill(e): removes an entity
    let w = Rc::clone(world);
    api.set("kill", lua.create_function(move |_, e: String| w.borrow_mut().kill(&e).map_err(fail))?)?;

    // ecs.instantiate(prefab, id): id and its children become instances of prefab and its children
    let w = Rc::clone(world);
    api.set(
        "instantiate",
        lua.create_function(move |_, (prefab, id): (String, String)| w.borrow_mut().instantiate(&prefab, &id, 0).map_err(fail))?,
    )?;

    // ecs.spawn(id, {comp = value, ...}): a new entity; is_a and parent are set first
    let w = Rc::clone(world);
    api.set(
        "spawn",
        lua.create_function(move |lua, (id, comps): (String, Table)| {
            if w.borrow().exists(&id) {
                return Err(fail(format!("exists: {}", id)));
            }
            let mut pairs: Vec<(String, Value)> = Vec::new();
            for pair in comps.pairs::<String, Value>() {
                pairs.push(pair?);
            }
            if pairs.is_empty() {
                return Err(fail(String::from("empty_entity")));
            }
            pairs.sort_by_key(|(c, _)| (c != "is_a", c != "parent", c.clone()));
            for (c, v) in pairs {
                let text = from_lua(v)?;
                if w.borrow().kind_of(&c) == Some(Kind::Code) {
                    lua.load(&text).set_name(&id).into_function().map_err(|err| fail(format!("bad_code: {}", clip(&reason_of(&err), 80))))?;
                }
                w.borrow_mut().set(&id, &c, &text).map_err(fail)?;
            }
            Ok(())
        })?,
    )?;

    // ecs.with(c [, v]) -> sorted list of entities with an own value of c (equal to v)
    let w = Rc::clone(world);
    api.set(
        "with",
        lua.create_function(move |_, (c, v): (String, Option<Value>)| {
            let v = match v {
                Some(Value::Nil) | None => None,
                Some(x) => Some(from_lua(x)?),
            };
            // a hidden value cannot be found by guessing it either
            if v.is_some() && w.borrow().hidden(&c) {
                return Ok(Vec::new());
            }
            Ok(w.borrow().with(&c, v.as_deref()))
        })?,
    )?;

    let w = Rc::clone(world);
    api.set("children", lua.create_function(move |_, e: String| Ok(w.borrow().children(&e)))?)?;

    let w = Rc::clone(world);
    api.set("exists", lua.create_function(move |_, e: String| Ok(w.borrow().exists(&e)))?)?;


    // ecs.comps(e) -> the names of its own components
    let w = Rc::clone(world);
    api.set("comps", lua.create_function(move |_, e: String| Ok(w.borrow().own_comps(&e)))?)?;

    // ecs.kind(c) -> the kind of a component type, or nil
    let w = Rc::clone(world);
    api.set("kind", lua.create_function(move |_, c: String| Ok(w.borrow().kind_of(&c).map(|k| k.as_str())))?)?;

    // ecs.call(entity, arg) -> what the code of another entity returns. code is shared
    // this way (a view uses another view, a screen block uses a view). the called code
    // runs with the rights of the run that calls it; the top of its chunk is read-only
    let w = Rc::clone(world);
    let b = Rc::clone(base);
    let depth = Rc::new(Cell::new(0u32));
    api.set(
        "call",
        lua.create_function(move |lua, (e, arg): (String, Value)| {
            if depth.get() >= CALL_DEPTH_MAX {
                return Err(fail(String::from("call_too_deep")));
            }
            let src = w.borrow().value(&e, "src").map(|s| s.to_string()).ok_or_else(|| fail(format!("no_code: {}", e)))?;
            let env = fresh_env(lua, &b.borrow())?;
            let was = w.borrow().read_only;
            w.borrow_mut().read_only = true;
            let f = lua.load(&src).set_name(&e).set_environment(env).call::<Value>(());
            w.borrow_mut().read_only = was;
            let f = match f? {
                Value::Function(f) => f,
                _ => return Err(fail(format!("bad_code: {} returns no function", e))),
            };
            depth.set(depth.get() + 1);
            let r = f.call::<Value>(arg);
            depth.set(depth.get() - 1);
            r
        })?,
    )?;

    // ecs.history(entity, n) -> the last n changes of one entity (n at most 50), newest
    // first: {seq, role, op, comp, value}; what a local-only value was is not shown
    let w = Rc::clone(world);
    api.set(
        "history",
        lua.create_function(move |lua, (e, n): (String, i64)| {
            let w = w.borrow();
            let n = n.clamp(0, 50) as usize;
            let out = lua.create_table()?;
            let mut k = 0;
            for en in w.journal.iter().rev() {
                if k >= n {
                    break;
                }
                if en.entity != e || en.comp == "-" {
                    continue;
                }
                let row = lua.create_table()?;
                row.set("seq", en.seq)?;
                row.set("role", en.role.as_str())?;
                row.set("op", en.op.as_str())?;
                row.set("comp", en.comp.as_str())?;
                let v: String = if w.hidden(&en.comp) { String::from("...") } else { en.value.chars().take(80).collect() };
                row.set("value", v)?;
                k += 1;
                out.set(k, row)?;
            }
            Ok(out)
        })?,
    )?;

    // ecs.journal(n) -> the last n entries (n at most 50): {role, op, entity, comp, value}
    let w = Rc::clone(world);
    api.set(
        "journal",
        lua.create_function(move |lua, n: i64| {
            let w = w.borrow();
            let n = n.clamp(0, 50) as usize;
            let out = lua.create_table()?;
            for (i, e) in w.journal.iter().skip(w.journal.len().saturating_sub(n)).enumerate() {
                let row = lua.create_table()?;
                row.set("seq", e.seq)?;
                row.set("role", e.role.as_str())?;
                row.set("op", e.op.as_str())?;
                row.set("entity", e.entity.as_str())?;
                row.set("comp", e.comp.as_str())?;
                // a private value (hide_private) is not shown, as in ecs.history
                let v = if w.hidden(&e.comp) { String::from("...") } else { e.value.chars().take(60).collect::<String>() };
                row.set("value", v)?;
                out.set(i + 1, row)?;
            }
            Ok(out)
        })?,
    )?;

    // ecs.role() -> the role this run acts as
    let w = Rc::clone(world);
    api.set("role", lua.create_function(move |_, ()| Ok(w.borrow().current_role.clone()))?)?;

    Ok(api)
}

impl Host {
    // a new world with only the genesis types
    pub fn new() -> mlua::Result<Host> {
        Host::with_world(World::new())
    }

    // a host around a world that already exists, for example one replayed from disk
    pub fn with_world(world: World) -> mlua::Result<Host> {
        let (lua, budget) = new_sandbox()?;
        let world = Rc::new(RefCell::new(world));
        let base = Rc::new(RefCell::new(Vec::new()));
        let api = make_api(&lua, &world, &base)?;
        lua.globals().set("ecs", api)?;
        for pair in lua.globals().pairs::<String, Value>() {
            let (k, v) = pair?;
            if k != "_G" {
                base.borrow_mut().push((k, v));
            }
        }
        Ok(Host { lua, world, budget, base, refused: RefCell::new(None) })
    }

    // ---------- the program's own api: what a program built on lust adds to lua ----------
    // a game engine adds its drawing and input, an erp its pages. what is added is seen by
    // every later run; a table is copied fresh for each run, as the libraries are, so a
    // script cannot leave anything in it. an added function that changes the world should
    // do it through World::set, so the rights and the journal hold

    // the lua state, to make the functions and tables to add (and to keep the program's data
    // in it: Lua::set_app_data, read in a function with Lua::app_data_ref)
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    // a global under a new name (ecs and the libraries cannot be replaced)
    pub fn add_global(&self, name: &str, value: Value) -> Result<(), String> {
        let mut base = self.base.borrow_mut();
        if name.is_empty() || base.iter().any(|(k, _)| k == name) {
            return Err(format!("taken: {}", clip(name, 40)));
        }
        base.push((name.to_string(), value));
        Ok(())
    }

    // a function in the table ecs under a new name
    pub fn add_to_ecs(&self, name: &str, f: Function) -> Result<(), String> {
        let base = self.base.borrow();
        let ecs = base.iter().find(|(k, _)| k == "ecs").and_then(|(_, v)| v.as_table().cloned()).ok_or_else(|| String::from("no ecs table"))?;
        let taken: Value = ecs.raw_get(name).map_err(|e| reason_of(&e))?;
        if name.is_empty() || !taken.is_nil() {
            return Err(format!("taken: ecs.{}", clip(name, 40)));
        }
        ecs.raw_set(name, f).map_err(|e| reason_of(&e))
    }

    // a fresh environment: copies of the base functions and fresh copies of the library tables
    fn env(&self) -> mlua::Result<Table> {
        fresh_env(&self.lua, &self.base.borrow())
    }

    // the function an entity's code returns. the chunk itself runs read-only:
    // only the function it returns may change the world
    fn code_of(&self, entity: &str) -> Result<Function, String> {
        // the code may come from a prefab: screen blocks share the code of ui.lib
        let src = self.world.borrow().value(entity, "src").map(|s| s.to_string()).ok_or_else(|| format!("no_code: {}", entity))?;
        let env = self.env().map_err(|e| reason_of(&e))?;
        let was = self.world.borrow().read_only;
        self.world.borrow_mut().read_only = true;
        self.budget.arm();
        let r = self.lua.load(&src).set_name(entity).set_environment(env).call::<Value>(());
        self.world.borrow_mut().read_only = was;
        match r {
            Ok(Value::Function(f)) => Ok(f),
            Ok(_) => Err(format!("bad_code: {} returns no function", entity)),
            Err(e) => Err(format!("bad_code: {}: {}", entity, clip(&reason_of(&e), 80))),
        }
    }

    // a syntax check, nothing runs
    pub fn check_code(&self, name: &str, src: &str) -> Result<(), String> {
        self.lua.load(src).set_name(name).into_function().map(|_| ()).map_err(|e| format!("bad_code: {}", clip(&reason_of(&e), 120)))
    }

    pub fn role_exists(&self, role: &str) -> bool {
        valid_id(role) && self.world.borrow().exists(&format!("role.{}", role))
    }

    // the roles of the world: the entities role.<name>
    pub fn roles(&self) -> Vec<String> {
        let w = self.world.borrow();
        w.entities().filter_map(|e| e.strip_prefix("role.")).filter(|r| !r.contains('.')).map(|r| r.to_string()).collect()
    }

    fn op_info(&self, op: &str) -> Result<OpInfo, String> {
        let w = self.world.borrow();
        let id = format!("op.{}", op);
        if !valid_id(op) || w.own(&id, "src").is_none() {
            return Err(format!("unknown_op: {}", clip(op, 40)));
        }
        let params = parse_params(w.value(&id, "params").unwrap_or("")).map_err(|e| format!("bad_params: {}: {}", op, e))?;
        Ok(OpInfo { name: op.to_string(), desc: w.value(&id, "desc").unwrap_or("").to_string(), params })
    }

    fn may_run(&self, role: &str, op: &str) -> bool {
        let w = self.world.borrow();
        w.value(&format!("op.{}", op), "roles").map_or(false, |r| r.split(' ').any(|x| x == role))
    }

    // the operations a role may run, sorted
    pub fn ops_of(&self, role: &str) -> Vec<OpInfo> {
        let names: Vec<String> = {
            let w = self.world.borrow();
            w.with("src", None).into_iter().filter_map(|e| e.strip_prefix("op.").map(|s| s.to_string())).collect()
        };
        names.iter().filter(|n| self.may_run(role, n)).filter_map(|n| self.op_info(n).ok()).collect()
    }

    // checks a request before anything runs
    pub fn check(&self, role: &str, op: &str, args: &[(&str, &str)]) -> Result<OpInfo, String> {
        if !self.role_exists(role) {
            return Err(format!("unknown_role: {}", clip(role, 40)));
        }
        let info = self.op_info(op)?;
        if !self.may_run(role, op) {
            return Err(format!("forbidden: {} for {}", op, role));
        }
        for (i, (k, _)) in args.iter().enumerate() {
            if !info.params.iter().any(|p| p.name == *k) {
                return Err(format!("unknown_param: {}", clip(k, 40)));
            }
            // a second value for the same name would skip the check and win in run
            if args[..i].iter().any(|(k2, _)| k2 == k) {
                return Err(format!("duplicate_param: {}", clip(k, 40)));
            }
        }
        for p in &info.params {
            match args.iter().find(|(k, _)| *k == p.name) {
                None => {
                    if p.required {
                        return Err(format!("missing_param: {}", p.name));
                    }
                }
                Some((_, v)) => {
                    let ok = match p.kind.as_str() {
                        "id" => valid_id(v),
                        "int" => v.trim().parse::<i64>().is_ok(),
                        "long" => !v.is_empty() && v.len() <= CODE_MAX,
                        _ => valid_text(v),
                    };
                    if !ok {
                        return Err(format!("bad_{}: {}", p.kind, p.name));
                    }
                }
            }
        }
        Ok(info)
    }

    // sets what happens after a refusal or a failed run (see the field refused)
    pub fn on_refusal(&self, hook: Rc<RefusalHook>) {
        *self.refused.borrow_mut() = Some(hook);
    }

    fn tell_refused(&self, role: &str, reason: &str) {
        let hook = self.refused.borrow().clone();
        if let Some(h) = hook {
            h(self, role, reason);
        }
    }

    // runs one operation as a role, then the systems; all or none
    pub fn run(&self, role: &str, op: &str, args: &[(&str, &str)]) -> Result<(), String> {
        let journal_role = if valid_id(role) { role.to_string() } else { String::from("-") };
        let journal_op = if valid_id(op) { op.to_string() } else { String::from("-") };
        let info = match self.check(role, op, args) {
            Ok(i) => i,
            Err(e) => {
                {
                    let mut w = self.world.borrow_mut();
                    w.current_role = journal_role.clone();
                    w.current_op = journal_op;
                    w.note(&format!("refused: {}", clip(&e, REASON_MAX)));
                }
                self.tell_refused(&journal_role, &e);
                return Err(e);
            }
        };
        let len = self.world.borrow().journal.len();
        let r = self.run_inner(role, op, &info, args);
        if let Err(reason) = &r {
            {
                let mut w = self.world.borrow_mut();
                w.current_role = journal_role.clone();
                w.current_op = journal_op;
                w.rollback(len, &clip(reason, REASON_MAX));
            }
            self.tell_refused(&journal_role, reason);
        }
        r
    }

    // an operation that came from outside the person's own hands: a confirmed proposal
    // of an AI ("mcp 12"), a step of a card pack ("pack cards/x.json 3/5"), later an
    // adapter or a linked node. the journal gets a note before ("from <source>") and
    // after ("end <source> ok|failed"), so what came from where can be found again, and
    // crystallized (crystal.rs)
    pub fn run_from(&self, source: &str, role: &str, op: &str, args: &[(&str, &str)]) -> Result<(), String> {
        let journal_role = if valid_id(role) { role.to_string() } else { String::from("-") };
        let journal_op = if valid_id(op) { op.to_string() } else { String::from("-") };
        let mark = |text: String| {
            let mut w = self.world.borrow_mut();
            w.current_role = journal_role.clone();
            w.current_op = journal_op.clone();
            w.note(&text);
        };
        mark(format!("from {}", source));
        let r = self.run(role, op, args);
        mark(format!("end {} {}", source, if r.is_ok() { "ok" } else { "failed" }));
        r
    }

    fn run_inner(&self, role: &str, op: &str, info: &OpInfo, args: &[(&str, &str)]) -> Result<(), String> {
        let f = self.code_of(&format!("op.{}", op))?;
        let p = self.lua.create_table().map_err(|e| reason_of(&e))?;
        for (k, v) in args {
            let kind = info.params.iter().find(|x| x.name == *k).map(|x| x.kind.as_str()).unwrap_or("text");
            let r = if kind == "int" { p.set(*k, v.trim().parse::<i64>().unwrap_or(0)) } else { p.set(*k, *v) };
            r.map_err(|e| reason_of(&e))?;
        }
        {
            let mut w = self.world.borrow_mut();
            w.current_role = role.to_string();
            w.current_op = op.to_string();
        }
        self.budget.arm();
        f.call::<()>(p).map_err(|e| reason_of(&e))?;

        // the systems: rules of the world, run after every operation as the role "system"
        self.systems()
    }

    fn systems(&self) -> Result<(), String> {
        let systems = {
            let w = self.world.borrow();
            let mut s = w.with("system", Some("true"));
            s.sort_by_key(|e| (w.value(e, "ord").and_then(|o| o.parse::<i64>().ok()).unwrap_or(0), e.clone()));
            s
        };
        for s in systems {
            let f = self.code_of(&s)?;
            {
                let mut w = self.world.borrow_mut();
                w.current_role = String::from("system");
                w.current_op = s.clone();
            }
            self.budget.arm();
            f.call::<()>(()).map_err(|e| format!("{}: {}", s, reason_of(&e)))?;
        }
        Ok(())
    }

    // runs read-only code and gives back what it returns, as json
    pub fn read_only_call(&self, entity: &str, arg: Option<&str>) -> Result<serde_json::Value, String> {
        self.read_only_call_with(entity, arg)
    }

    // the same with any arguments lua takes (a tuple for several)
    pub fn read_only_call_with<A: mlua::IntoLuaMulti>(&self, entity: &str, args: A) -> Result<serde_json::Value, String> {
        let f = self.code_of(entity)?;
        let was = self.world.borrow().read_only;
        self.world.borrow_mut().read_only = true;
        self.budget.arm();
        let r = f.call::<Value>(args);
        self.world.borrow_mut().read_only = was;
        let v = r.map_err(|e| reason_of(&e))?;
        // an empty lua table is an empty list, and keys come sorted: the same world gives the same text
        let opts = mlua::serde::DeserializeOptions::new().encode_empty_tables_as_array(true).sort_keys(true);
        self.lua.from_value_with::<serde_json::Value>(v, opts).map_err(|e| first_line(&e.to_string()))
    }

    // a view: what a screen shows. "view.<name>" returns a table
    pub fn view(&self, name: &str, arg: Option<&str>) -> Result<serde_json::Value, String> {
        if !valid_id(name) || !self.world.borrow().exists(&format!("view.{}", name)) {
            return Err(format!("unknown_view: {}", clip(name, 40)));
        }
        self.read_only_call(&format!("view.{}", name), arg)
    }

    // runs a snippet inside the same sandbox and gives back the text it returns: a probe
    // of the sandbox, or a quick question to the world
    pub fn probe(&self, src: &str) -> Result<String, String> {
        let env = self.env().map_err(|e| reason_of(&e))?;
        self.budget.arm();
        self.lua.load(src).set_name("probe").set_environment(env).eval::<String>().map_err(|e| reason_of(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::verify;

    // a tiny world: one role, one component, one operation, one system
    fn host() -> Host {
        let h = Host::new().unwrap();
        {
            let mut w = h.world.borrow_mut();
            w.current_role = String::from("dev");
            for (e, c, v) in [
                ("role.clerk", "desc", "a clerk"),
                ("role.dev", "desc", "the developer"),
                ("roles", "kind", "names"),
                ("roles", "write", "dev"),
                ("params", "kind", "text"),
                ("params", "write", "dev"),
                ("src", "kind", "code"),
                ("src", "write", "dev"),
                ("system", "kind", "bool"),
                ("system", "write", "dev"),
                ("count", "kind", "int"),
                ("count", "write", "clerk system"),
                ("seen", "kind", "int"),
                ("seen", "write", "system"),
                ("op.bump", "desc", "adds n to the counter"),
                ("op.bump", "params", "n:int note:text?"),
                ("op.bump", "roles", "clerk"),
                ("op.bump", "src", "return function(p) local n = ecs.get('c', 'count') or 0 ecs.set('c', 'count', n + p.n) if p.note == 'fail' then error('asked_to_fail', 0) end end"),
                ("sys.watch", "system", "true"),
                ("sys.watch", "src", "return function() if ecs.get('c', 'count') then ecs.set('c', 'seen', ecs.get('c', 'count')) end end"),
            ] {
                w.set(e, c, v).unwrap();
            }
        }
        h
    }

    #[test]
    fn an_operation_runs_then_the_systems_run() {
        let h = host();
        h.run("clerk", "bump", &[("n", "3")]).unwrap();
        h.run("clerk", "bump", &[("n", "4")]).unwrap();
        let w = h.world.borrow();
        assert_eq!(w.own("c", "count"), Some("7"));
        assert_eq!(w.own("c", "seen"), Some("7"));
        let last = w.journal.last().unwrap();
        assert_eq!((last.role.as_str(), last.op.as_str(), last.comp.as_str()), ("system", "sys.watch", "seen"));
    }

    #[test]
    fn a_failure_anywhere_undoes_the_operation_and_its_systems() {
        let h = host();
        h.run("clerk", "bump", &[("n", "1")]).unwrap();
        assert_eq!(h.run("clerk", "bump", &[("n", "5"), ("note", "fail")]), Err("asked_to_fail".to_string()));
        let w = h.world.borrow();
        assert_eq!(w.own("c", "count"), Some("1"));
        assert!(w.journal.last().unwrap().value.starts_with("rollback"));
        assert!(verify(&w.journal, &w.head()).is_ok());
    }

    #[test]
    fn requests_are_checked_before_anything_runs() {
        let h = host();
        assert_eq!(h.check("dev", "bump", &[("n", "1")]).unwrap_err(), "forbidden: bump for dev");
        assert!(h.check("boss", "bump", &[]).unwrap_err().starts_with("unknown_role"));
        assert!(h.check("clerk", "nope", &[]).unwrap_err().starts_with("unknown_op"));
        assert_eq!(h.check("clerk", "bump", &[]).unwrap_err(), "missing_param: n");
        assert_eq!(h.check("clerk", "bump", &[("n", "x")]).unwrap_err(), "bad_int: n");
        assert_eq!(h.check("clerk", "bump", &[("n", "1"), ("x", "1")]).unwrap_err(), "unknown_param: x");
        assert_eq!(h.check("clerk", "bump", &[("n", "1"), ("n", "2")]).unwrap_err(), "duplicate_param: n");
        assert_eq!(h.check("clerk", "bump", &[("n", "1"), ("note", "")]).unwrap_err(), "bad_text: note");
        let _ = h.run("dev", "bump", &[("n", "1")]);
        let w = h.world.borrow();
        let last = w.journal.last().unwrap();
        assert_eq!((last.role.as_str(), last.value.as_str()), ("dev", "refused: forbidden: bump for dev"));
    }

    #[test]
    fn the_rights_of_a_component_hold_inside_code_too() {
        let h = host();
        // the operation runs as clerk; clerk may not write "seen", whatever the code says
        h.world.borrow_mut().current_role = String::from("dev");
        h.world.borrow_mut().set("op.bump", "src", "return function(p) ecs.set('c', 'seen', 1) end").unwrap();
        assert_eq!(h.run("clerk", "bump", &[("n", "1")]), Err("no_right: seen for clerk".to_string()));
    }

    #[test]
    fn code_is_data_the_developer_changes() {
        let h = host();
        h.world.borrow_mut().current_role = String::from("dev");
        h.world.borrow_mut().set("op.bump", "src", "return function(p) ecs.set('c', 'count', p.n * 10) end").unwrap();
        h.run("clerk", "bump", &[("n", "2")]).unwrap();
        assert_eq!(h.world.borrow().own("c", "count"), Some("20"));
    }

    #[test]
    fn no_state_survives_outside_the_components() {
        let h = host();
        h.world.borrow_mut().current_role = String::from("dev");
        h.world
            .borrow_mut()
            .set("op.bump", "src", "hidden = (hidden or 0) + 1 string.hidden = (string.hidden or 0) + 1 return function(p) ecs.set('c', 'count', hidden * 100 + string.hidden) end")
            .unwrap();
        h.run("clerk", "bump", &[("n", "1")]).unwrap();
        h.run("clerk", "bump", &[("n", "1")]).unwrap();
        assert_eq!(h.world.borrow().own("c", "count"), Some("101"));
        assert_eq!(h.probe("return tostring(getmetatable)"), Ok("nil".to_string()));
    }

    #[test]
    fn the_top_of_a_code_chunk_cannot_write() {
        let h = host();
        h.world.borrow_mut().current_role = String::from("dev");
        h.world.borrow_mut().set("op.bump", "src", "ecs.set('c', 'count', 99) return function(p) end").unwrap();
        assert!(h.run("clerk", "bump", &[("n", "1")]).unwrap_err().contains("readonly"));
    }

    #[test]
    fn broken_code_is_refused_when_written_from_lua() {
        let h = host();
        h.world.borrow_mut().current_role = String::from("dev");
        let r = h.probe("ecs.set('op.bump', 'src', 'return function(') return 'x'");
        assert!(r.unwrap_err().starts_with("bad_code"));
    }

    #[test]
    fn params_are_parsed_strictly() {
        assert_eq!(parse_params("a:id b:text?").unwrap().len(), 2);
        assert!(parse_params("a:number").is_err());
        assert!(parse_params("a:id a:id").is_err());
        assert!(parse_params("A:id").is_err());
        assert!(parse_params("").unwrap().is_empty());
    }

    #[test]
    fn a_program_adds_its_own_api() {
        let h = host();
        let twice = h.lua().create_function(|_, n: i64| Ok(n * 2)).unwrap();
        h.add_to_ecs("twice", twice).unwrap();
        let t = h.lua().create_table().unwrap();
        t.set("name", "kiosk").unwrap();
        h.add_global("app", Value::Table(t)).unwrap();
        assert_eq!(h.probe("return tostring(ecs.twice(21)) .. ' ' .. app.name"), Ok("42 kiosk".to_string()));
        // a run cannot change what the next run gets
        assert_eq!(h.probe("app.name = 'x' ecs.twice = nil return 'ok'"), Ok("ok".to_string()));
        assert_eq!(h.probe("return tostring(ecs.twice(1)) .. ' ' .. app.name"), Ok("2 kiosk".to_string()));
        assert!(h.add_global("string", Value::Nil).is_err());
        let f = h.lua().create_function(|_, ()| Ok(())).unwrap();
        assert!(h.add_to_ecs("set", f).is_err());
    }

    #[test]
    fn no_finalizer_can_be_planted() {
        let h = host();
        let r = h.probe("setmetatable({}, {__gc = function() end}) return 'x'");
        assert!(r.unwrap_err().contains("no_gc"));
        assert_eq!(h.probe("return tostring(getmetatable == nil and setmetatable({}, {__index = {a = 1}}).a)"), Ok("1".to_string()));
    }

    #[test]
    fn sandbox_has_no_system_access() {
        let h = host();
        for src in [
            "return tostring(os)",
            "return tostring(io)",
            "return tostring(dofile)",
            "return tostring(load)",
            "return tostring(require)",
            "return tostring(debug)",
            "return tostring(pcall)",
            "return tostring(xpcall)",
            "return tostring(collectgarbage)",
            "return tostring(string.find)",
            "return tostring(string.gsub)",
            "return tostring(('x').gsub)",
        ] {
            assert_eq!(h.probe(src), Ok("nil".to_string()), "{}", src);
        }
        assert_eq!(h.probe("return tostring(string.find_plain('hello', 'll'))"), Ok("3".to_string()));
    }

    #[test]
    fn endless_loops_expensive_calls_and_memory_hogs_are_stopped() {
        let h = host();
        assert!(h.probe("while true do end return 'x'").unwrap_err().contains("time_limit"));
        let t = Instant::now();
        let r = h.probe("for i = 1, 4000 do local s = string.rep('x', 1000000) end return 'done'");
        assert!(r.unwrap_err().contains("time_limit"));
        assert!(t.elapsed() < Duration::from_secs(2), "took {:?}", t.elapsed());
        assert!(h.probe("return #string.rep('x', 2000000)").unwrap_err().contains("rep_too_large"));
        let r = h.probe("local t = {} for i = 1, 40 do t[i] = string.rep('x', 1000000) .. i end return 'x'");
        assert!(r.unwrap_err().contains("memory"));
        // the host keeps working
        h.run("clerk", "bump", &[("n", "1")]).unwrap();
    }

    #[test]
    fn code_can_call_code_and_read_the_journal() {
        let h = host();
        {
            let mut w = h.world.borrow_mut();
            w.set("lib.double", "src", "return function(x) return x * 2 end").unwrap();
            w.set("view.v", "src", "return function() return {d = ecs.call('lib.double', 21), last = ecs.journal(1)[1].entity} end").unwrap();
            w.set("lib.loop", "src", "return function() return ecs.call('lib.loop') end").unwrap();
        }
        let v = h.view("v", None).unwrap();
        assert_eq!(v["d"], 42);
        assert_eq!(v["last"], "lib.loop");
        h.world.borrow_mut().set("view.l", "src", "return function() return ecs.call('lib.loop') end").unwrap();
        assert!(h.view("l", None).unwrap_err().contains("call_too_deep"));
        // a called function inside a view cannot write either
        h.world.borrow_mut().set("lib.w", "src", "return function() ecs.set('c', 'count', 1) end").unwrap();
        h.world.borrow_mut().set("view.w", "src", "return function() ecs.call('lib.w') return 1 end").unwrap();
        assert!(h.view("w", None).unwrap_err().contains("readonly"));
    }
}
