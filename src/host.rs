// lua host: one world, its own lua sandbox, and the code of the world that lives in the
// component "src" of its entities:
//   op.<name>                  an operation: what a role may do (host.run)
//   sys.<name>, system = true  a rule: runs after every operation
//   anything with tick = true  a tick: runs on host.tick(dt), with dt
//   view.<name>                read-only, gives back a table
//   any other src              shared code, reached with ecs.call
// every world is equal: its spec (ecs.rs) says whether it keeps a journal, checks roles,
// undoes a failed run, and whether its lua remembers between runs. a program makes as many
// worlds (hosts) as it needs and drops them when it wants; one world gives a result to
// another through link.rs

use crate::ecs::{valid_id, valid_text, Kind, Spec, World, CODE_MAX};
use mlua::ffi;
use mlua::state::{GcGenParams, GcMode};
use mlua::{Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, Value, VmState};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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

    // both limits at once: instructions (in hundreds, one hook is HOOK_EVERY instructions)
    // and wall time. a game world can count instructions per tick and keep the wall time only
    // as a guard against one expensive library call
    pub fn set_limits(&self, hooks: u32, time: Duration) {
        self.hooks.set(hooks.max(1));
        self.time.set(time);
    }
}

// compiled code, so code that did not change is not compiled again on every run. the key is
// the entity, the text is compared before use (the developer changes code while the program
// runs). only the compiled chunk is kept: every run still gets a fresh environment
const CHUNKS_MAX: usize = 4096;

pub struct Chunks {
    map: HashMap<String, (String, Function)>,
}

impl Chunks {
    pub fn new() -> Chunks {
        Chunks { map: HashMap::new() }
    }
}

impl Default for Chunks {
    fn default() -> Self {
        Chunks::new()
    }
}

// the chunk of an entity's code with the given environment. set_environment must succeed:
// a chunk left with the shared globals would let one run leave state for the next
pub fn chunk(lua: &Lua, chunks: &RefCell<Chunks>, name: &str, src: &str, env: Table) -> mlua::Result<Function> {
    let cached = match chunks.borrow().map.get(name) {
        Some((text, f)) if text == src => Some(f.clone()),
        _ => None,
    };
    let f = match cached {
        Some(f) => f,
        None => {
            let f = lua.load(src).set_name(name).into_function()?;
            let mut c = chunks.borrow_mut();
            if c.map.len() >= CHUNKS_MAX {
                c.map.clear();
            }
            c.map.insert(name.to_string(), (src.to_string(), f.clone()));
            f
        }
    };
    if !f.set_environment(env)? {
        return Err(mlua::Error::runtime("no_env: the code has no environment"));
    }
    Ok(f)
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

// a rust system of the program: runs over the world's columns when lua calls ecs.run(name,
// arg). it is the program's own code: it writes past rights, undo and limits
pub type System = Box<dyn FnMut(&mut World, f64)>;

// a result waiting for the program (ecs.commit): the pairs, sorted
type Pending = Rc<RefCell<Option<Vec<(String, String)>>>>;

// the lua of a world whose spec says keep_lua: one environment for every run, and the
// function each entity's code returned, kept until the code changes
#[derive(Default)]
struct Kept {
    env: Option<Table>,
    funcs: HashMap<String, (String, Function)>,
}

const KEPT_MAX: usize = 4096;

// what the functions of the table ecs share
#[derive(Clone)]
struct Ctx {
    world: Rc<RefCell<World>>,
    base: Rc<RefCell<Vec<(String, Value)>>>,
    chunks: Rc<RefCell<Chunks>>,
    kept: Rc<RefCell<Kept>>,
    systems: Rc<RefCell<HashMap<String, System>>>,
    result: Pending,
    ticks: Rc<Cell<u64>>,
}

pub struct Host {
    lua: Lua,
    pub world: Rc<RefCell<World>>,
    pub budget: Rc<Budget>,
    // the globals every run starts from; in a world without keep_lua each run gets fresh
    // copies, so nothing a script keeps outside the components survives the run
    base: Rc<RefCell<Vec<(String, Value)>>>,
    // what the program built on lust does when an operation is refused or fails (after
    // the run is undone): the role and the reason. None: nothing more than the journal note
    refused: RefCell<Option<Rc<RefusalHook>>>,
    ctx: Ctx,
}

// the hook of a refusal: the host, the role, the reason
pub type RefusalHook = dyn Fn(&Host, &str, &str);

// a value of the world as lua sees it: numbers, booleans and lists where the kind says so
fn to_lua(lua: &Lua, kind: Option<Kind>, v: &str) -> mlua::Result<Value> {
    Ok(match kind {
        Some(Kind::Int) => Value::Integer(v.parse::<i64>().unwrap_or(0)),
        Some(Kind::Num) => Value::Number(v.parse::<f64>().unwrap_or(0.0)),
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
        Value::Number(f) => crate::ecs::fmt_num(f),
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

// ---------- entities, components and numbers from lua ----------

fn number(v: &Value) -> mlua::Result<f64> {
    match v {
        Value::Integer(i) => Ok(*i as f64),
        Value::Number(f) => Ok(*f),
        Value::Boolean(b) => Ok(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => s.to_str()?.trim().parse::<f64>().map_err(|_| fail(String::from("not_a_number"))),
        _ => Err(fail(String::from("not_a_number"))),
    }
}

// an entity given by name or by number: its name, or None for a number of an entity that is
// gone (or a name that does not exist yet, which only a write may make)
fn ent_opt(w: &World, v: &Value) -> mlua::Result<Option<String>> {
    match v {
        Value::String(s) => Ok(Some(s.to_str()?.to_string())),
        Value::Integer(id) => Ok(w.index_of(*id).and_then(|i| w.name_at(i)).map(|s| s.to_string())),
        _ => Err(fail(String::from("bad_entity"))),
    }
}

fn ent_name(w: &World, v: &Value) -> mlua::Result<String> {
    ent_opt(w, v)?.ok_or_else(|| fail(String::from("dead_entity")))
}

// the index of an entity that exists, by name or number
fn ent_index(w: &World, v: &Value) -> mlua::Result<Option<u32>> {
    match v {
        Value::String(s) => Ok(w.index_by_name(&s.to_str()?)),
        Value::Integer(id) => Ok(w.index_of(*id)),
        _ => Err(fail(String::from("bad_entity"))),
    }
}

// a num component by name or by column number (ecs.col); None for a text component
fn num_col(w: &World, v: &Value) -> mlua::Result<Option<usize>> {
    match v {
        Value::String(s) => Ok(w.col(&s.to_str()?)),
        Value::Integer(i) if *i >= 0 => w.live_col(*i as usize).map(Some).ok_or_else(|| fail(String::from("bad_column"))),
        _ => Err(fail(String::from("bad_column"))),
    }
}

// a num component that must be one
fn need_col(w: &World, v: &Value) -> mlua::Result<usize> {
    num_col(w, v)?.ok_or_else(|| fail(String::from("bad_column: not a num component")))
}

fn comp_name(v: &Value) -> mlua::Result<String> {
    match v {
        Value::String(s) => Ok(s.to_str()?.to_string()),
        _ => Err(fail(String::from("bad_component"))),
    }
}

// ---------- many values across in one step ----------
// mlua guards every single table write when the state has a memory limit, which costs
// tens of nanoseconds per value. these three do a whole column in one guarded call
// (Lua::exec_raw): inside, only lua api calls that cannot raise an error, except the
// allocation of a table, which the guard catches. nothing owned is held across them

// n, then the table of ids, then one table per column, as lua values
fn new_tables(lua: &Lua, ids: &[i64], cols: &[Vec<f64>]) -> mlua::Result<mlua::MultiValue> {
    let n = ids.len() as std::os::raw::c_int;
    unsafe {
        lua.exec_raw::<mlua::MultiValue>((), |state| {
            ffi::lua_checkstack(state, cols.len() as std::os::raw::c_int + 4);
            ffi::lua_pushinteger(state, n as ffi::lua_Integer);
            ffi::lua_createtable(state, n, 0);
            for (i, v) in ids.iter().enumerate() {
                ffi::lua_pushinteger(state, *v);
                ffi::lua_rawseti(state, -2, (i + 1) as ffi::lua_Integer);
            }
            for col in cols {
                ffi::lua_createtable(state, n, 0);
                for (i, v) in col.iter().enumerate() {
                    ffi::lua_pushnumber(state, *v);
                    ffi::lua_rawseti(state, -2, (i + 1) as ffi::lua_Integer);
                }
            }
        })
    }
}

fn fill_tables(lua: &Lua, ids: Table, vals: Table, v: &[f64], i: &[i64]) -> mlua::Result<()> {
    unsafe {
        lua.exec_raw::<()>((ids, vals), |state| {
            ffi::lua_checkstack(state, 4);
            // the arguments: 1 = ids, 2 = vals
            for (k, x) in v.iter().enumerate() {
                ffi::lua_pushnumber(state, *x);
                ffi::lua_rawseti(state, 2, (k + 1) as ffi::lua_Integer);
            }
            for (k, x) in i.iter().enumerate() {
                ffi::lua_pushinteger(state, *x);
                ffi::lua_rawseti(state, 1, (k + 1) as ffi::lua_Integer);
            }
            ffi::lua_settop(state, 0);
        })
    }
}

// the first n entries (or all, up to the first nil) of two tables; anything but numbers
// is an error after the call
fn take_tables(lua: &Lua, ids: Table, vals: Table, n: Option<usize>, max: usize) -> mlua::Result<(Vec<i64>, Vec<f64>)> {
    let mut idv: Vec<i64> = Vec::new();
    let mut vv: Vec<f64> = Vec::new();
    let mut bad = false;
    let limit = n.unwrap_or(max).min(max);
    unsafe {
        lua.exec_raw::<()>((ids, vals), |state| {
            ffi::lua_checkstack(state, 4);
            let mut k: usize = 0;
            while k < limit {
                let ti = ffi::lua_rawgeti(state, 1, (k + 1) as ffi::lua_Integer);
                let tv = ffi::lua_rawgeti(state, 2, (k + 1) as ffi::lua_Integer);
                if ti == ffi::LUA_TNIL || tv == ffi::LUA_TNIL {
                    ffi::lua_settop(state, 2);
                    break;
                }
                let mut ok_i: std::os::raw::c_int = 0;
                let mut ok_v: std::os::raw::c_int = 0;
                let id = ffi::lua_tointegerx(state, -2, &mut ok_i);
                let v = ffi::lua_tonumberx(state, -1, &mut ok_v);
                ffi::lua_settop(state, 2);
                if ok_i == 0 || ok_v == 0 || ti != ffi::LUA_TNUMBER || tv != ffi::LUA_TNUMBER {
                    bad = true;
                    break;
                }
                idv.push(id);
                vv.push(v);
                k += 1;
            }
            ffi::lua_settop(state, 0);
        })?;
    }
    if bad || (n.is_some() && idv.len() < limit) {
        return Err(fail(String::from("bad_list: ids and vals must be numbers")));
    }
    Ok((idv, vv))
}

// ---------- the code of an entity ----------

// the environment of a world with keep_lua: made once from the base
fn kept_env(lua: &Lua, ctx: &Ctx) -> mlua::Result<Table> {
    if let Some(env) = &ctx.kept.borrow().env {
        return Ok(env.clone());
    }
    let env = fresh_env(lua, &ctx.base.borrow())?;
    ctx.kept.borrow_mut().env = Some(env.clone());
    Ok(env)
}

// the function an entity's code returns. without keep_lua the chunk runs in a fresh
// environment every time, read-only: only the function it returns may change the world.
// with keep_lua the chunk is the load of that code: it runs once (again when the code
// changes), with the rights of the run that first needs it, so it may spawn the first
// entities; its function is kept
fn function_of(lua: &Lua, ctx: &Ctx, entity: &str) -> Result<Function, String> {
    // the code may come from a prefab: screen blocks share the code of ui.lib
    let src = ctx.world.borrow().value(entity, "src").map(|s| s.to_string()).ok_or_else(|| format!("no_code: {}", entity))?;
    let keep = ctx.world.borrow().spec.keep_lua;
    if keep {
        if let Some((text, f)) = ctx.kept.borrow().funcs.get(entity) {
            if *text == src {
                return Ok(f.clone());
            }
        }
    }
    let env = if keep { kept_env(lua, ctx) } else { fresh_env(lua, &ctx.base.borrow()) }.map_err(|e| reason_of(&e))?;
    let was = ctx.world.borrow().read_only;
    if !keep {
        ctx.world.borrow_mut().read_only = true;
    }
    let r = chunk(lua, &ctx.chunks, entity, &src, env).and_then(|c| c.call::<Value>(()));
    ctx.world.borrow_mut().read_only = was;
    match r {
        Ok(Value::Function(f)) => {
            if keep {
                let mut k = ctx.kept.borrow_mut();
                if k.funcs.len() >= KEPT_MAX {
                    k.funcs.clear();
                }
                k.funcs.insert(entity.to_string(), (src, f.clone()));
            }
            Ok(f)
        }
        Ok(_) => Err(format!("bad_code: {} returns no function", entity)),
        Err(e) => Err(format!("bad_code: {}: {}", entity, clip(&reason_of(&e), 80))),
    }
}

// ---------- the table ecs: the only way a script reaches its world ----------

fn make_api(lua: &Lua, ctx: &Ctx) -> mlua::Result<Table> {
    let api = lua.create_table()?;

    // ecs.get(e, c) -> value, from   (own value, or the one of the prefab chain). e is a
    // name or a number (ecs.id); c a name or, for a num component, a column number (ecs.col)
    let w = Rc::clone(&ctx.world);
    api.set(
        "get",
        lua.create_function(move |lua, (e, c): (Value, Value)| {
            let w = w.borrow();
            if let Some(col) = num_col(&w, &c)? {
                if w.col_hidden(col) {
                    return Ok((Value::Nil, Value::Nil));
                }
                let v = ent_index(&w, &e)?.and_then(|i| w.num_at(i, col));
                return Ok((v.map_or(Value::Nil, Value::Number), Value::Nil));
            }
            let c = comp_name(&c)?;
            let Some(e) = ent_opt(&w, &e)? else { return Ok((Value::Nil, Value::Nil)) };
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
    let w = Rc::clone(&ctx.world);
    api.set(
        "own",
        lua.create_function(move |lua, (e, c): (Value, Value)| {
            let w = w.borrow();
            if let Some(col) = num_col(&w, &c)? {
                if w.col_hidden(col) {
                    return Ok(Value::Nil);
                }
                let v = ent_index(&w, &e)?.and_then(|i| w.column(col).get(i));
                return Ok(v.map_or(Value::Nil, Value::Number));
            }
            let c = comp_name(&c)?;
            let Some(e) = ent_opt(&w, &e)? else { return Ok(Value::Nil) };
            if w.hidden(&c) {
                return Ok(Value::Nil);
            }
            match w.own(&e, &c) {
                None => Ok(Value::Nil),
                Some(v) => to_lua(lua, w.kind_of(&c), v),
            }
        })?,
    )?;

    // ecs.set(e, c, v): raises on any refusal; the host then undoes the whole run (when the
    // world's spec says undo)
    let w = Rc::clone(&ctx.world);
    api.set(
        "set",
        lua.create_function(move |lua, (e, c, v): (Value, Value, Value)| {
            let col = num_col(&w.borrow(), &c)?;
            if let Some(col) = col {
                let x = number(&v)?;
                let mut w = w.borrow_mut();
                return match &e {
                    Value::Integer(id) => {
                        let i = w.index_of(*id).ok_or_else(|| fail(String::from("dead_entity")))?;
                        w.set_num_at(i, col, x).map_err(fail)
                    }
                    _ => {
                        let name = ent_name(&w, &e)?;
                        w.set_num(&name, col, x).map_err(fail)
                    }
                };
            }
            let name = ent_name(&w.borrow(), &e)?;
            let c = comp_name(&c)?;
            let text = from_lua(v)?;
            if w.borrow().kind_of(&c) == Some(Kind::Code) {
                // code is checked when it is written, not when it first runs
                lua.load(&text).set_name(&name).into_function().map_err(|err| fail(format!("bad_code: {}", clip(&reason_of(&err), 80))))?;
            }
            w.borrow_mut().set(&name, &c, &text).map_err(fail)
        })?,
    )?;

    // ecs.del(e, c): removes an own value
    let w = Rc::clone(&ctx.world);
    api.set(
        "del",
        lua.create_function(move |_, (e, c): (Value, Value)| {
            let name = ent_name(&w.borrow(), &e)?;
            let c = match num_col(&w.borrow(), &c)? {
                Some(col) => w.borrow().column(col).name().to_string(),
                None => comp_name(&c)?,
            };
            w.borrow_mut().del(&name, &c).map_err(fail)
        })?,
    )?;

    // ecs.kill(e): removes an entity
    let w = Rc::clone(&ctx.world);
    api.set(
        "kill",
        lua.create_function(move |_, e: Value| {
            let name = ent_name(&w.borrow(), &e)?;
            w.borrow_mut().kill(&name).map_err(fail)
        })?,
    )?;

    // ecs.instantiate(prefab, id): id and its children become instances of prefab and its children
    let w = Rc::clone(&ctx.world);
    api.set(
        "instantiate",
        lua.create_function(move |_, (prefab, id): (String, String)| w.borrow_mut().instantiate(&prefab, &id, 0).map_err(fail))?,
    )?;

    // ecs.spawn(id, {comp = value, ...}) or ecs.spawn({comp = value, ...}) -> the number of
    // the new entity. without a name it is called "#<n>". is_a and parent are set first
    let w = Rc::clone(&ctx.world);
    api.set(
        "spawn",
        lua.create_function(move |lua, (a, b): (Value, Option<Table>)| {
            let (name, comps) = match (a, b) {
                (Value::Table(t), None) => (w.borrow_mut().new_anon(), t),
                (Value::String(s), Some(t)) => {
                    let s = s.to_str()?.to_string();
                    if w.borrow().exists(&s) {
                        return Err(fail(format!("exists: {}", clip(&s, 40))));
                    }
                    (s, t)
                }
                _ => return Err(fail(String::from("bad_spawn: spawn(id, {...}) or spawn({...})"))),
            };
            let mut pairs: Vec<(String, Option<usize>, Value)> = Vec::new();
            for pair in comps.pairs::<Value, Value>() {
                let (k, v) = pair?;
                let col = num_col(&w.borrow(), &k)?;
                let c = match col {
                    Some(col) => w.borrow().column(col).name().to_string(),
                    None => comp_name(&k)?,
                };
                pairs.push((c, col, v));
            }
            if pairs.is_empty() {
                return Err(fail(String::from("empty_entity")));
            }
            pairs.sort_by(|x, y| (x.0 != "is_a", x.0 != "parent", &x.0).cmp(&(y.0 != "is_a", y.0 != "parent", &y.0)));
            for (c, col, v) in pairs {
                if let Some(col) = col {
                    let x = number(&v)?;
                    w.borrow_mut().set_num(&name, col, x).map_err(fail)?;
                } else {
                    let text = from_lua(v)?;
                    if w.borrow().kind_of(&c) == Some(Kind::Code) {
                        lua.load(&text).set_name(&name).into_function().map_err(|err| fail(format!("bad_code: {}", clip(&reason_of(&err), 80))))?;
                    }
                    w.borrow_mut().set(&name, &c, &text).map_err(fail)?;
                }
            }
            Ok(w.borrow().id(&name))
        })?,
    )?;

    // ecs.with(c [, v]) -> sorted list of the names of entities with an own value of c
    // (equal to v)
    let w = Rc::clone(&ctx.world);
    api.set(
        "with",
        lua.create_function(move |_, (c, v): (Value, Option<Value>)| {
            let c = match num_col(&w.borrow(), &c)? {
                Some(col) => w.borrow().column(col).name().to_string(),
                None => comp_name(&c)?,
            };
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

    let w = Rc::clone(&ctx.world);
    api.set(
        "children",
        lua.create_function(move |_, e: Value| {
            let w = w.borrow();
            Ok(ent_opt(&w, &e)?.map(|e| w.children(&e)).unwrap_or_default())
        })?,
    )?;

    let w = Rc::clone(&ctx.world);
    api.set(
        "exists",
        lua.create_function(move |_, e: Value| {
            let w = w.borrow();
            Ok(ent_opt(&w, &e)?.is_some_and(|e| w.exists(&e)))
        })?,
    )?;

    // ecs.comps(e) -> the names of its own components
    let w = Rc::clone(&ctx.world);
    api.set(
        "comps",
        lua.create_function(move |_, e: Value| {
            let w = w.borrow();
            Ok(ent_opt(&w, &e)?.map(|e| w.own_comps(&e)).unwrap_or_default())
        })?,
    )?;

    // ecs.kind(c) -> the kind of a component type, or nil
    let w = Rc::clone(&ctx.world);
    api.set("kind", lua.create_function(move |_, c: String| Ok(w.borrow().kind_of(&c).map(|k| k.as_str())))?)?;

    // ecs.id(e) -> the number of an entity (faster than its name for num components)
    let w = Rc::clone(&ctx.world);
    api.set(
        "id",
        lua.create_function(move |_, e: Value| {
            let w = w.borrow();
            Ok(ent_index(&w, &e)?.map(|i| w.id_at(i)))
        })?,
    )?;

    // ecs.name(e) -> the name of an entity ("#<n>" when it was made without one)
    let w = Rc::clone(&ctx.world);
    api.set(
        "name",
        lua.create_function(move |_, e: Value| {
            let w = w.borrow();
            Ok(ent_index(&w, &e)?.and_then(|i| w.name_at(i)).map(|s| s.to_string()))
        })?,
    )?;

    // ecs.col(c) -> the number of a num component (faster than its name)
    let w = Rc::clone(&ctx.world);
    api.set(
        "col",
        lua.create_function(move |_, c: Value| {
            let w = w.borrow();
            need_col(&w, &c)
        })?,
    )?;

    // ecs.count() -> entities; ecs.count(c) -> entities with an own value of c
    let w = Rc::clone(&ctx.world);
    api.set(
        "count",
        lua.create_function(move |_, c: Option<Value>| {
            let w = w.borrow();
            match c {
                None | Some(Value::Nil) => Ok(w.count()),
                Some(c) => match num_col(&w, &c)? {
                    Some(col) => Ok(w.column(col).len()),
                    None => Ok(w.with(&comp_name(&c)?, None).len()),
                },
            }
        })?,
    )?;

    // ecs.query(c, c, ...) -> n, ids, vals, vals, ...: the entities that have all these num
    // components (own values), with one new lua table per component, in the same order, in
    // one call. the fastest way to bring many values into lua
    let w = Rc::clone(&ctx.world);
    api.set(
        "query",
        lua.create_function(move |lua, cols: mlua::Variadic<Value>| {
            if cols.is_empty() || cols.len() > 16 {
                return Err(fail(String::from("bad_query: 1 to 16 components")));
            }
            let w = w.borrow();
            let mut ids: Vec<usize> = Vec::new();
            for c in cols.iter() {
                ids.push(need_col(&w, c)?);
            }
            if ids.iter().any(|&c| w.col_hidden(c)) {
                drop(w);
                return new_tables(lua, &[], &vec![Vec::new(); cols.len()]);
            }
            let (e, v) = w.query(&ids);
            drop(w);
            new_tables(lua, &e, &v)
        })?,
    )?;

    // ecs.read(c, ids, vals) -> n: one num component into two tables lua keeps from tick to
    // tick, so no new garbage is made; entries after n are left from an earlier tick
    let w = Rc::clone(&ctx.world);
    api.set(
        "read",
        lua.create_function(move |lua, (c, ids, vals): (Value, Table, Table)| {
            let w = w.borrow();
            let col = need_col(&w, &c)?;
            if w.col_hidden(col) {
                return Ok(0);
            }
            let (e, v) = w.query(&[col]);
            drop(w);
            let v = v.into_iter().next().unwrap_or_default();
            fill_tables(lua, ids, vals, &v, &e)?;
            Ok(v.len())
        })?,
    )?;

    // ecs.write(c, ids, vals [, n]): the values back, in one call (rights and undo hold)
    let w = Rc::clone(&ctx.world);
    api.set(
        "write",
        lua.create_function(move |lua, (c, ids, vals, n): (Value, Table, Table, Option<usize>)| {
            let col = need_col(&w.borrow(), &c)?;
            let max = w.borrow().spec.entities_max;
            let (idv, vv) = take_tables(lua, ids, vals, n, max)?;
            let mut w = w.borrow_mut();
            for (id, v) in idv.iter().zip(vv.iter()) {
                let i = w.index_of(*id).ok_or_else(|| fail(String::from("dead_entity")))?;
                w.set_num_at(i, col, *v).map_err(fail)?;
            }
            Ok(())
        })?,
    )?;

    // whole num components at once
    let w = Rc::clone(&ctx.world);
    api.set(
        "add",
        lua.create_function(move |_, (dst, src, k): (Value, Value, Option<f64>)| {
            let mut w = w.borrow_mut();
            let d = need_col(&w, &dst)?;
            let s = need_col(&w, &src)?;
            w.add_cols(d, s, k.unwrap_or(1.0)).map_err(fail)
        })?,
    )?;

    let w = Rc::clone(&ctx.world);
    api.set(
        "scale",
        lua.create_function(move |_, (c, k): (Value, f64)| {
            let mut w = w.borrow_mut();
            let c = need_col(&w, &c)?;
            if !k.is_finite() {
                return Err(fail(String::from("not_finite")));
            }
            w.map_col(c, |v| v * k).map_err(fail)
        })?,
    )?;

    let w = Rc::clone(&ctx.world);
    api.set(
        "fill",
        lua.create_function(move |_, (c, x): (Value, f64)| {
            let mut w = w.borrow_mut();
            let c = need_col(&w, &c)?;
            if !x.is_finite() {
                return Err(fail(String::from("not_finite")));
            }
            w.map_col(c, |_| x).map_err(fail)
        })?,
    )?;

    let w = Rc::clone(&ctx.world);
    api.set(
        "clamp",
        lua.create_function(move |_, (c, lo, hi): (Value, f64, f64)| {
            let mut w = w.borrow_mut();
            let c = need_col(&w, &c)?;
            if !lo.is_finite() || !hi.is_finite() || lo > hi {
                return Err(fail(String::from("bad_range")));
            }
            w.map_col(c, |v| v.clamp(lo, hi)).map_err(fail)
        })?,
    )?;

    // ecs.sum(c) -> sum, n
    let w = Rc::clone(&ctx.world);
    api.set(
        "sum",
        lua.create_function(move |_, c: Value| {
            let w = w.borrow();
            let c = need_col(&w, &c)?;
            if w.col_hidden(c) {
                return Ok((0.0, 0));
            }
            Ok((w.column(c).values().iter().sum::<f64>(), w.column(c).len()))
        })?,
    )?;

    // ecs.run(name [, arg]): a rust system the program added (Host::add_system)
    let w = Rc::clone(&ctx.world);
    let sy = Rc::clone(&ctx.systems);
    api.set(
        "run",
        lua.create_function(move |_, (name, arg): (mlua::LuaString, Option<f64>)| {
            let name = name.to_str()?;
            if w.borrow().read_only {
                return Err(fail(String::from("readonly")));
            }
            let mut systems = sy.borrow_mut();
            let f = systems.get_mut(&*name).ok_or_else(|| fail(format!("no_system: {}", clip(&name, 40))))?;
            f(&mut w.borrow_mut(), arg.unwrap_or(0.0));
            Ok(())
        })?,
    )?;

    // ecs.commit({key = value, ...}): a result for the program, which may give it to another
    // world as one operation (link::commit_to). one at a time: a second commit before the
    // program took the first is refused
    let r = Rc::clone(&ctx.result);
    api.set(
        "commit",
        lua.create_function(move |_, t: Table| {
            if r.borrow().is_some() {
                return Err(fail(String::from("result_pending")));
            }
            let mut out: Vec<(String, String)> = Vec::new();
            for pair in t.pairs::<String, Value>() {
                let (k, v) = pair?;
                if !valid_id(&k) || out.len() >= RESULT_MAX {
                    return Err(fail(format!("bad_result: {}", clip(&k, 40))));
                }
                let text = match v {
                    Value::String(s) => s.to_str()?.to_string(),
                    Value::Integer(i) => i.to_string(),
                    Value::Number(f) if f.fract() == 0.0 && f.abs() < 1e15 => (f as i64).to_string(),
                    Value::Number(f) if f.is_finite() => crate::ecs::fmt_num(f),
                    Value::Boolean(b) => b.to_string(),
                    _ => return Err(fail(format!("bad_result: {}", clip(&k, 40)))),
                };
                if !valid_text(&text) {
                    return Err(fail(format!("bad_result: {}", clip(&k, 40))));
                }
                out.push((k, text));
            }
            out.sort();
            *r.borrow_mut() = Some(out);
            Ok(())
        })?,
    )?;

    // ecs.tick() -> how many ticks the world has had
    let t = Rc::clone(&ctx.ticks);
    api.set("tick", lua.create_function(move |_, ()| Ok(t.get()))?)?;

    // ecs.call(entity, arg) -> what the code of another entity returns. code is shared
    // this way (a view uses another view, a screen block uses a view). the called code
    // runs with the rights of the run that calls it; the top of its chunk is read-only
    let cx = ctx.clone();
    let depth = Rc::new(Cell::new(0u32));
    api.set(
        "call",
        lua.create_function(move |lua, (e, arg): (String, Value)| {
            if depth.get() >= CALL_DEPTH_MAX {
                return Err(fail(String::from("call_too_deep")));
            }
            let f = function_of(lua, &cx, &e).map_err(fail)?;
            depth.set(depth.get() + 1);
            let r = f.call::<Value>(arg);
            depth.set(depth.get() - 1);
            r
        })?,
    )?;

    // ecs.history(entity, n) -> the last n changes of one entity (n at most 50), newest
    // first: {seq, role, op, comp, value}; what a local-only value was is not shown.
    // empty in a world without a journal
    let w = Rc::clone(&ctx.world);
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
    let w = Rc::clone(&ctx.world);
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
    let w = Rc::clone(&ctx.world);
    api.set("role", lua.create_function(move |_, ()| Ok(w.borrow().current_role.clone()))?)?;

    Ok(api)
}

// the most keys of one result
pub const RESULT_MAX: usize = 64;

impl Host {
    // a ledger: journal, roles, undo, a fresh lua for every run (Spec::ledger)
    pub fn new() -> mlua::Result<Host> {
        Host::with_world(World::new())
    }

    // a new world of the given spec (Spec::game for a game that runs)
    pub fn with_spec(spec: Spec) -> mlua::Result<Host> {
        Host::with_world(World::with_spec(spec))
    }

    // a host around a world that already exists, for example one replayed from disk or
    // loaded from a snapshot; its spec decides
    pub fn with_world(world: World) -> mlua::Result<Host> {
        let (lua, budget) = new_sandbox()?;
        lua.set_memory_limit(world.spec.memory)?;
        // a game loop makes much short-lived garbage: the generational collector takes it
        // in short steps
        if world.spec.keep_lua {
            lua.gc_set_mode(GcMode::Generational(GcGenParams::default()));
        }
        let ctx = Ctx {
            world: Rc::new(RefCell::new(world)),
            base: Rc::new(RefCell::new(Vec::new())),
            chunks: Rc::new(RefCell::new(Chunks::new())),
            kept: Rc::new(RefCell::new(Kept::default())),
            systems: Rc::new(RefCell::new(HashMap::new())),
            result: Rc::new(RefCell::new(None)),
            ticks: Rc::new(Cell::new(0)),
        };
        let api = make_api(&lua, &ctx)?;
        lua.globals().set("ecs", api)?;
        for pair in lua.globals().pairs::<String, Value>() {
            let (k, v) = pair?;
            if k != "_G" {
                ctx.base.borrow_mut().push((k, v));
            }
        }
        Ok(Host { lua, world: Rc::clone(&ctx.world), budget, base: Rc::clone(&ctx.base), refused: RefCell::new(None), ctx })
    }

    // ---------- the program's own api: what a program built on lust adds to lua ----------
    // a game engine adds its drawing and input, an erp its pages. what is added is seen by
    // every later run; in a world without keep_lua a table is copied fresh for each run, as
    // the libraries are, so a script cannot leave anything in it. an added function that
    // changes the world should do it through World::set, so the rights and the journal hold

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
        base.push((name.to_string(), value.clone()));
        if let Some(env) = &self.ctx.kept.borrow().env {
            env.set(name, value).map_err(|e| reason_of(&e))?;
        }
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
        ecs.raw_set(name, f.clone()).map_err(|e| reason_of(&e))?;
        if let Some(env) = &self.ctx.kept.borrow().env {
            let kept_ecs: Table = env.get("ecs").map_err(|e| reason_of(&e))?;
            kept_ecs.raw_set(name, f).map_err(|e| reason_of(&e))?;
        }
        Ok(())
    }

    // a rust system of the program, run from lua as ecs.run(name, arg)
    pub fn add_system(&self, name: &str, f: System) -> Result<(), String> {
        if !valid_id(name) {
            return Err(format!("bad_name: {}", clip(name, 40)));
        }
        self.ctx.systems.borrow_mut().insert(name.to_string(), f);
        Ok(())
    }

    // the result lua committed, if any; taking it lets lua commit again
    pub fn take_result(&self) -> Option<Vec<(String, String)>> {
        self.ctx.result.borrow_mut().take()
    }

    // how many ticks the world has had
    pub fn ticks(&self) -> u64 {
        self.ctx.ticks.get()
    }

    // a step of lua's garbage collector, for the end of a frame with time to spare
    pub fn gc_step(&self) -> Result<bool, String> {
        self.lua.gc_step().map_err(|e| reason_of(&e))
    }

    // the memory lua uses now, in bytes
    pub fn memory(&self) -> usize {
        self.lua.used_memory()
    }

    // a fresh environment: copies of the base functions and fresh copies of the library tables
    fn env(&self) -> mlua::Result<Table> {
        fresh_env(&self.lua, &self.base.borrow())
    }

    // the function an entity's code returns (see function_of)
    fn code_of(&self, entity: &str) -> Result<Function, String> {
        self.budget.arm();
        function_of(&self.lua, &self.ctx, entity)
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

    // in a world without roles anyone may run any operation
    fn may_run(&self, role: &str, op: &str) -> bool {
        let w = self.world.borrow();
        !w.spec.roles || w.value(&format!("op.{}", op), "roles").is_some_and(|r| r.split(' ').any(|x| x == role))
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
        let roles = self.world.borrow().spec.roles;
        if roles && !self.role_exists(role) {
            return Err(format!("unknown_role: {}", clip(role, 40)));
        }
        if !valid_id(role) {
            return Err(format!("bad_role: {}", clip(role, 40)));
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

    // runs one operation as a role, then the systems; all or none (when the spec says undo)
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
        let m = self.world.borrow_mut().begin();
        let r = self.run_inner(role, op, &info, args);
        match &r {
            Ok(()) => self.world.borrow_mut().commit(m),
            Err(reason) => {
                {
                    let mut w = self.world.borrow_mut();
                    w.current_role = journal_role.clone();
                    w.current_op = journal_op;
                    w.rollback(m, &clip(reason, REASON_MAX));
                }
                self.tell_refused(&journal_role, reason);
            }
        }
        r
    }

    // an operation that came from outside the person's own hands: a confirmed proposal
    // of an AI ("mcp 12"), a result of another world ("world battle"), a step of a card
    // pack. the journal gets a note before ("from <source>") and after ("end <source>
    // ok|failed"), so what came from where can be found again
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
        {
            let mut w = self.world.borrow_mut();
            w.current_role = role.to_string();
            w.current_op = op.to_string();
        }
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

    // the entities with an own value true of a bool component, by ord, then by name
    fn ordered(&self, flag: &str) -> Vec<String> {
        let w = self.world.borrow();
        let mut s = w.with(flag, Some("true"));
        s.sort_by_key(|e| (w.value(e, "ord").and_then(|o| o.parse::<i64>().ok()).unwrap_or(0), e.clone()));
        s
    }

    fn systems(&self) -> Result<(), String> {
        for s in self.ordered("system") {
            {
                let mut w = self.world.borrow_mut();
                w.current_role = String::from("system");
                w.current_op = s.clone();
            }
            let f = self.code_of(&s)?;
            self.budget.arm();
            f.call::<()>(()).map_err(|e| format!("{}: {}", s, reason_of(&e)))?;
        }
        Ok(())
    }

    // one tick of the world: every entity whose component tick is true runs its code with
    // dt, as the role "system", in the order of ord. dt is whatever the program says: a
    // fixed step, or the time since the last frame (clock.rs). all or none when the spec
    // says undo
    pub fn tick(&self, dt: f64) -> Result<(), String> {
        if !dt.is_finite() || dt < 0.0 {
            return Err(String::from("bad_dt"));
        }
        let list = self.ordered("tick");
        let m = self.world.borrow_mut().begin();
        let mut r = Ok(());
        for s in &list {
            {
                let mut w = self.world.borrow_mut();
                w.current_role = String::from("system");
                w.current_op = s.clone();
            }
            let f = match self.code_of(s) {
                Ok(f) => f,
                Err(e) => {
                    r = Err(e);
                    break;
                }
            };
            self.budget.arm();
            if let Err(e) = f.call::<()>(dt) {
                r = Err(format!("{}: {}", s, reason_of(&e)));
                break;
            }
        }
        match &r {
            Ok(()) => self.world.borrow_mut().commit(m),
            Err(reason) => self.world.borrow_mut().rollback(m, &clip(reason, REASON_MAX)),
        }
        self.ctx.ticks.set(self.ctx.ticks.get() + 1);
        r
    }

    // runs read-only code and gives back what it returns, as json
    pub fn read_only_call(&self, entity: &str, arg: Option<&str>) -> Result<serde_json::Value, String> {
        self.read_only_call_with(entity, arg)
    }

    // the same with any arguments lua takes (a tuple for several)
    pub fn read_only_call_with<A: mlua::IntoLuaMulti>(&self, entity: &str, args: A) -> Result<serde_json::Value, String> {
        // read-only before the code is loaded: with keep_lua the top of a chunk runs as its
        // load, and a view must not write there either
        let was = self.world.borrow().read_only;
        self.world.borrow_mut().read_only = true;
        let r = self.code_of(entity).and_then(|f| {
            self.budget.arm();
            f.call::<Value>(args).map_err(|e| reason_of(&e))
        });
        self.world.borrow_mut().read_only = was;
        let v = r?;
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

    // runs a snippet inside the same sandbox (always a fresh environment) and gives back
    // the text it returns: a probe of the sandbox, or a quick question to the world
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
    fn compiled_code_is_kept_but_every_run_is_fresh() {
        let h = host();
        let a = "n = (n or 0) + 1 return function(p) ecs.set('c', 'count', 10 + n) end";
        h.world.borrow_mut().current_role = String::from("dev");
        h.world.borrow_mut().set("op.bump", "src", a).unwrap();
        h.run("clerk", "bump", &[("n", "1")]).unwrap();
        h.run("clerk", "bump", &[("n", "1")]).unwrap();
        // the chunk was compiled once, but the second run did not see the global of the first
        assert_eq!(h.world.borrow().own("c", "count"), Some("11"));
        assert_eq!(h.ctx.chunks.borrow().map.get("op.bump").map(|x| x.0.as_str()), Some(a));
        // changed code is compiled again
        let b = "return function(p) ecs.set('c', 'count', 7) end";
        h.world.borrow_mut().current_role = String::from("dev");
        h.world.borrow_mut().set("op.bump", "src", b).unwrap();
        h.run("clerk", "bump", &[("n", "1")]).unwrap();
        assert_eq!(h.world.borrow().own("c", "count"), Some("7"));
        assert_eq!(h.ctx.chunks.borrow().map.get("op.bump").map(|x| x.0.as_str()), Some(b));
    }

    #[test]
    fn a_nested_call_of_the_same_code_does_not_take_the_environment_of_the_outer_one() {
        let h = host();
        {
            let mut w = h.world.borrow_mut();
            w.set("lib.f", "src", "tag = (tag or 0) + 1 mine = {} return function(d) if d > 0 then ecs.call('lib.f', d - 1) end mine[#mine + 1] = d return tostring(tag) .. ':' .. #mine end").unwrap();
            w.set("view.v", "src", "return function() return ecs.call('lib.f', 3) end").unwrap();
        }
        // every level has its own environment: tag 1 and one entry in its own table
        assert_eq!(h.view("v", None).unwrap(), "1:1");
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

    // ---------- worlds of other specs ----------

    // a game world: no journal, no roles, lua remembers; types and ticks set by the program
    fn game(src: &str) -> Host {
        let h = Host::with_spec(Spec::game()).unwrap();
        {
            let mut w = h.world.borrow_mut();
            for (e, c, v) in [
                ("src", "kind", "code"),
                ("tick", "kind", "bool"),
                ("params", "kind", "text"),
                ("x", "kind", "num"),
                ("vx", "kind", "num"),
                ("hp", "kind", "num"),
                ("label", "kind", "text"),
                ("sys.main", "tick", "true"),
            ] {
                w.set(e, c, v).unwrap();
            }
            w.set("sys.main", "src", src).unwrap();
        }
        h
    }

    #[test]
    fn a_game_world_ticks_with_any_dt_and_lua_remembers() {
        let h = game("n = 0 total = 0 return function(dt) n = n + 1 total = total + dt ecs.set('clock', 'x', total) ecs.set('clock', 'hp', n) end");
        for dt in [0.016, 0.017, 0.0331] {
            h.tick(dt).unwrap();
        }
        let w = h.world.borrow();
        assert_eq!(w.num("clock", "hp"), Some(3.0));
        assert!((w.num("clock", "x").unwrap() - 0.0661).abs() < 1e-12);
        assert!(w.journal.is_empty());
        drop(w);
        assert_eq!(h.ticks(), 3);
        assert_eq!(h.tick(f64::NAN), Err(String::from("bad_dt")));
        assert_eq!(h.tick(-1.0), Err(String::from("bad_dt")));
    }

    #[test]
    fn nameless_entities_by_number_and_whole_columns() {
        let h = game("
            X, VX = ecs.col('x'), ecs.col('vx')
            for i = 1, 4 do ecs.spawn({[X] = i, vx = 10}) end
            ball = ecs.spawn('ball', {x = 0, vx = 1, label = 'the ball'})
            return function(dt)
                ecs.add(X, VX, dt)
                ecs.set(ball, X, ecs.get(ball, X) + 100)
            end");
        h.tick(0.5).unwrap();
        let w = h.world.borrow();
        let x = w.col("x").unwrap();
        let mut xs = w.column(x).values().to_vec();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(xs, vec![6.0, 7.0, 8.0, 9.0, 100.5]);
        assert_eq!(w.value("ball", "label"), Some("the ball"));
        assert!(w.exists("#1") && w.exists("#4"));
        drop(w);
        assert_eq!(h.probe("return ecs.name(ecs.id('ball')) .. ' ' .. tostring(ecs.count('x'))"), Ok(String::from("ball 5")));
    }

    #[test]
    fn query_read_and_write_move_columns_in_one_call() {
        let h = game("
            a = ecs.spawn({x = 1, vx = 10})
            b = ecs.spawn({x = 2})
            c = ecs.spawn({x = 3, vx = 30})
            ecs.kill(a)
            ids, vals = {}, {}
            return function(dt)
                local n, qi, xs, vxs = ecs.query('vx', 'x')
                for i = 1, n do xs[i] = xs[i] + vxs[i] end
                ecs.write('x', qi, xs, n)
                local m = ecs.read('x', ids, vals)
                for i = 1, m do vals[i] = vals[i] * 2 end
                ecs.write('x', ids, vals, m)
                ecs.commit({n = n, m = m, first = qi[1] == c, dead = ecs.exists(a)})
            end");
        h.tick(0.0).unwrap();
        let r = h.take_result().unwrap();
        assert_eq!(r, vec![
            (String::from("dead"), String::from("false")),
            (String::from("first"), String::from("true")),
            (String::from("m"), String::from("2")),
            (String::from("n"), String::from("1")),
        ]);
        let w = h.world.borrow();
        assert_eq!(w.num("#3", "x"), Some(66.0));
        assert_eq!(w.num("#2", "x"), Some(4.0));
    }

    #[test]
    fn rust_systems_and_one_result_at_a_time() {
        let h = game("return function(dt) ecs.run('double') ecs.commit({left = 3}) end");
        h.add_system("double", Box::new(|w: &mut World, _| {
            if let Some(x) = w.col("x") {
                for v in w.column_mut(x).values_mut() {
                    *v *= 2.0;
                }
            }
        }))
        .unwrap();
        h.world.borrow_mut().set("a", "x", "1.5").unwrap();
        h.tick(0.0).unwrap();
        assert!(h.tick(0.0).unwrap_err().contains("result_pending"));
        assert_eq!(h.take_result(), Some(vec![(String::from("left"), String::from("3"))]));
        assert_eq!(h.world.borrow().num("a", "x"), Some(6.0));
        let h = game("return function(dt) ecs.run('nothing') end");
        assert!(h.tick(0.0).unwrap_err().contains("no_system"));
    }

    #[test]
    fn a_bad_list_or_a_bad_number_is_refused() {
        let h = game("e = ecs.spawn({x = 1}) return function(dt) ecs.write('x', {e}, {'no'}) end");
        assert!(h.tick(0.0).unwrap_err().contains("bad_list"));
        let h = game("e = ecs.spawn({x = 1}) return function(dt) ecs.write('x', {e, e}, {1}, 2) end");
        assert!(h.tick(0.0).unwrap_err().contains("bad_list"));
        let h = game("e = ecs.spawn({x = 1}) return function(dt) ecs.kill(e) ecs.write('x', {e}, {1}) end");
        assert!(h.tick(0.0).unwrap_err().contains("dead_entity"));
        let h = game("return function(dt) ecs.spawn({x = 0/0}) end");
        assert!(h.tick(0.0).unwrap_err().contains("not_finite"));
        let h = game("return function(dt) ecs.query('label') end");
        assert!(h.tick(0.0).unwrap_err().contains("not a num component"));
    }

    #[test]
    fn a_tick_has_a_budget_and_the_sandbox_holds_in_a_game_world() {
        let h = game("return function(dt) while true do end end");
        assert!(h.tick(0.0).unwrap_err().contains("time_limit"));
        let h = game("return function(dt) return io.open('x') end");
        assert!(h.tick(0.0).is_err());
        let h = game("setmetatable({}, {__gc = function() end}) return function() end");
        assert!(h.tick(0.0).unwrap_err().contains("no_gc"));
        // the program's api reaches a world whose lua is already running
        let h = game("return function(dt) ecs.set('a', 'hp', twice(21)) end");
        h.tick(0.0).unwrap_err();
        let f = h.lua().create_function(|_, n: i64| Ok(n * 2)).unwrap();
        h.add_global("twice", Value::Function(f)).unwrap();
        h.tick(0.0).unwrap();
        assert_eq!(h.world.borrow().num("a", "hp"), Some(42.0));
    }

    #[test]
    fn in_a_ledger_num_values_have_rights_and_are_undone() {
        let h = host();
        {
            let mut w = h.world.borrow_mut();
            w.current_role = String::from("dev");
            w.set("pos", "kind", "num").unwrap();
            w.set("pos", "write", "clerk").unwrap();
            w.set("op.bump", "src", "return function(p) local id = ecs.spawn({pos = 1.5}) ecs.set('c', 'pos', p.n) ecs.add('pos', 'pos', 1) if p.note == 'fail' then error('asked_to_fail', 0) end end").unwrap();
        }
        h.run("clerk", "bump", &[("n", "2")]).unwrap();
        let before = h.world.borrow().all_values();
        let journal = h.world.borrow().journal.len();
        assert_eq!(h.run("clerk", "bump", &[("n", "5"), ("note", "fail")]), Err(String::from("asked_to_fail")));
        assert_eq!(h.world.borrow().all_values(), before);
        assert_eq!(h.world.borrow().journal.len(), journal + 1);
        assert_eq!(h.world.borrow().num("c", "pos"), Some(4.0));
        // the system role may not write pos
        h.world.borrow_mut().current_role = String::from("dev");
        h.world.borrow_mut().set("sys.watch", "src", "return function() ecs.set('c', 'pos', 1) end").unwrap();
        assert!(h.run("clerk", "bump", &[("n", "1")]).unwrap_err().contains("no_right: pos for system"));
    }

    #[test]
    fn a_result_of_one_world_is_one_operation_of_another() {
        let ledger = host();
        {
            let mut w = ledger.world.borrow_mut();
            w.current_role = String::from("dev");
            w.set("role.battle", "desc", "ends battles").unwrap();
            w.set("count", "write", "dev clerk system battle").unwrap();
            w.set("c", "count", "10").unwrap();
            w.set("op.result", "roles", "battle").unwrap();
            w.set("op.result", "params", "left:int").unwrap();
            w.set("op.result", "src", "return function(p) ecs.set('c', 'count', p.left) end").unwrap();
        }
        let mut seed = crate::link::Seed::new();
        let start = seed.read(&ledger.world.borrow(), "c", "count").unwrap();
        let battle = game(&format!("left = {} return function(dt) left = left - 3 if left < 3 then ecs.commit({{left = left}}) end end", start));
        let mut result = None;
        while result.is_none() {
            battle.tick(0.1).unwrap();
            result = battle.take_result();
        }
        let n = ledger.world.borrow().journal.len();
        crate::link::commit_to(&ledger, "battle", "battle", "result", &seed, &result.unwrap()).unwrap();
        let w = ledger.world.borrow();
        // from, the value, the system that watches count, end
        assert_eq!(w.journal.len() - n, 4);
        assert_eq!(w.own("c", "count"), Some("1"));
        drop(w);
        // the ledger changed meanwhile: the same result is refused
        assert!(crate::link::commit_to(&ledger, "battle", "battle", "result", &seed, &[(String::from("left"), String::from("1"))]).unwrap_err().starts_with("stale: c.count"));
        // and the other way: a ledger may feed a game world too, both are worlds
        let mut back = crate::link::Seed::new();
        back.read(&battle.world.borrow(), "nothing", "x");
        let g = game("return function(dt) end");
        {
            let mut w = g.world.borrow_mut();
            w.set("op.feed", "params", "n:int").unwrap();
            w.set("op.feed", "src", "return function(p) ecs.set('food', 'hp', p.n) end").unwrap();
        }
        crate::link::commit_to(&g, "ledger", "anyone", "feed", &back, &[(String::from("n"), String::from("7"))]).unwrap();
        assert_eq!(g.world.borrow().num("food", "hp"), Some(7.0));
    }

    #[test]
    fn a_failed_tick_is_undone_in_a_world_with_undo() {
        let mut spec = Spec::game();
        spec.undo = true;
        let h = Host::with_spec(spec).unwrap();
        {
            let mut w = h.world.borrow_mut();
            for (e, c, v) in [("src", "kind", "code"), ("tick", "kind", "bool"), ("x", "kind", "num"), ("sys.a", "tick", "true"), ("sys.a", "ord", "1"), ("sys.b", "tick", "true"), ("sys.b", "ord", "2")] {
                if e == "sys.a" && c == "ord" {
                    w.set("ord", "kind", "int").unwrap_or(());
                }
                w.set(e, c, v).unwrap();
            }
            w.set("sys.a", "src", "return function(dt) ecs.set('a', 'x', (ecs.get('a', 'x') or 0) + 1) end").unwrap();
            w.set("sys.b", "src", "return function(dt) if ecs.get('a', 'x') > 1 then error('two', 0) end end").unwrap();
        }
        h.tick(0.1).unwrap();
        assert_eq!(h.tick(0.1), Err(String::from("sys.b: two")));
        assert_eq!(h.world.borrow().num("a", "x"), Some(1.0));
    }

    #[test]
    fn a_view_cannot_write_at_its_load_in_a_world_that_keeps_lua() {
        let h = game("return function(dt) end");
        {
            let mut w = h.world.borrow_mut();
            w.set("view.v", "src", "ecs.set('a', 'hp', 1) return function() return 1 end").unwrap();
            w.set("view.ok", "src", "n = (n or 0) + 1 return function() n = n + 1 return n end").unwrap();
        }
        assert!(h.view("v", None).unwrap_err().contains("readonly"));
        assert!(!h.world.borrow().exists("a"));
        // a view's load runs once, its globals live on
        assert_eq!(h.view("ok", None).unwrap(), 2);
        assert_eq!(h.view("ok", None).unwrap(), 3);
        // a nameless entity that was never given cannot be made by a write
        assert!(h.probe("ecs.set('#99', 'hp', 1) return 'x'").unwrap_err().contains("dead_entity"));
    }
}
