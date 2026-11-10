// the driver: lua of its own that runs a world from outside, for a try and for the tests of
// a project (test/*.lua). it steps the world, holds keys, reads and sets values, draws a
// frame as text. its functions:
//   step([n [, dt]])   n ticks (1), dt seconds (1/60)
//   get(e, c)          a value: a number for num and int, a boolean for bool, else text
//   set(e, c, v)       as the developer (every right allows it)
//   keys(k, ...)       the keys held from now on; keys() lets them all go
//   press(k)           a key that goes down on the next tick
//   events()           what the world told the display since the last call (quit, sounds)
//   mouse(x, y, down)  mic(level 0..1)  glyph(w)   the width of a character of text, as a part of its size (0.6)
//   draw()             the frame of the drawings, as text lines
//   run(role, op, {k = v})   an operation      view(name [, arg])   a view, as a table
//   ents(c)            the names of the entities with c      tick()   ticks so far
//   ms()               milliseconds since the driver began
//   print(...)         what the answer shows: numbers with 4 decimals at most, tables as lua
use crate::host::{clip, from_lua, new_sandbox, reason_at, reason_of, Budget};
use crate::ecs::Kind;
use crate::project::{as_core, explain, App, Project, TEST_DIR};
use crate::screen::{self, Input};
use mlua::{Lua, LuaSerdeExt, MultiValue, Table, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

// how much an answer shows
pub const OUT_MAX: usize = 6000;
// how long one try or one test may take
pub const DRIVER_TIME: Duration = Duration::from_secs(20);

pub struct Driver {
    lua: Lua,
    budget: Rc<Budget>,
    out: Rc<RefCell<String>>,
    files: BTreeMap<String, String>,
}

// a number for a person: whole numbers as they are, else 4 decimals at most
pub fn short_num(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    if x == x.trunc() && x.abs() < 1e15 {
        return format!("{}", x as i64);
    }
    let s = format!("{:.4}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        String::from("0")
    } else {
        s.to_string()
    }
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && s.chars().all(|x| x.is_ascii_alphanumeric() || x == '_')
}

// a lua value as lua text, short: tables to depth 4, 50 items each
pub fn show(v: &Value, depth: usize) -> String {
    match v {
        Value::Nil => String::from("nil"),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => short_num(*n),
        Value::String(s) => match depth {
            0 => s.to_string_lossy().to_string(),
            _ => format!("{:?}", s.to_string_lossy()),
        },
        Value::Table(t) => {
            if depth >= 4 {
                return String::from("{...}");
            }
            let n = t.raw_len();
            let mut parts = Vec::new();
            for i in 1..=n.min(50) {
                parts.push(show(&t.raw_get::<Value>(i).unwrap_or(Value::Nil), depth + 1));
            }
            if n > 50 {
                parts.push(format!("... {} in all", n));
            }
            let mut keyed: Vec<(String, String)> = Vec::new();
            for (k, x) in t.pairs::<Value, Value>().flatten() {
                let skip = matches!(k, Value::Integer(i) if i >= 1 && (i as usize) <= n);
                if skip {
                    continue;
                }
                let key = match &k {
                    Value::String(s) if is_ident(&s.to_string_lossy()) => s.to_string_lossy().to_string(),
                    other => format!("[{}]", show(other, depth + 1)),
                };
                keyed.push((key, show(&x, depth + 1)));
            }
            keyed.sort();
            let total = keyed.len();
            for (k, x) in keyed.into_iter().take(50) {
                parts.push(format!("{} = {}", k, x));
            }
            if total > 50 {
                parts.push(format!("... {} keys", total));
            }
            format!("{{{}}}", parts.join(", "))
        }
        Value::Function(_) => String::from("function"),
        _ => String::from("?"),
    }
}

fn append(out: &RefCell<String>, line: &str) {
    let mut o = out.borrow_mut();
    if o.len() >= OUT_MAX {
        return;
    }
    if o.len() + line.len() + 1 > OUT_MAX {
        let room = OUT_MAX.saturating_sub(o.len()).max(1);
        o.push_str(&clip(line, room));
        o.push_str("\n... (cut)\n");
        return;
    }
    o.push_str(line);
    o.push('\n');
}

fn fail(msg: String) -> mlua::Error {
    mlua::Error::runtime(msg)
}

impl Driver {
    pub fn new(app: &App) -> Result<Driver, String> {
        Driver::with_time(app, DRIVER_TIME)
    }

    // the same with another time limit
    pub fn with_time(app: &App, time: Duration) -> Result<Driver, String> {
        let (lua, budget) = new_sandbox().map_err(|e| reason_of(&e))?;
        budget.set_limits(u32::MAX, time);
        let out = Rc::new(RefCell::new(String::new()));
        let held = Rc::new(RefCell::new(Input::default()));
        let start = Instant::now();
        let deadline = start + time;
        Driver::install(&lua, app, &out, &held, start, deadline, time).map_err(|e| reason_of(&e))?;
        Ok(Driver { lua, budget, out, files: app.files.clone() })
    }

    fn install(lua: &Lua, app: &App, out: &Rc<RefCell<String>>, held: &Rc<RefCell<Input>>, start: Instant, deadline: Instant, time: Duration) -> mlua::Result<()> {
        let g = lua.globals();
        let files: Rc<BTreeMap<String, String>> = Rc::new(app.files.clone());

        let (h, s, f, k) = (Rc::clone(&app.host), Rc::clone(&app.screen), Rc::clone(&files), Rc::clone(held));
        let sv = Rc::clone(&app.saves);
        // what the world told the display (sounds, quit), kept for events()
        let told: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let tl = Rc::clone(&told);
        g.set(
            "step",
            lua.create_function(move |_, (n, dt): (Option<i64>, Option<f64>)| {
                let n = n.unwrap_or(1).clamp(0, 10_000_000);
                let dt = dt.unwrap_or(1.0 / 60.0);
                for _ in 0..n {
                    if Instant::now() > deadline {
                        return Err(fail(format!("time_limit: the driver may take {} ms", time.as_millis())));
                    }
                    {
                        let mut held = k.borrow_mut();
                        *s.input.borrow_mut() = Input { down: held.down.clone(), pressed: std::mem::take(&mut held.pressed), mouse: held.mouse, glyph: held.glyph, mic: held.mic };
                    }
                    let r = h.tick(dt);
                    screen::after_tick(&s);
                    crate::saves::after_tick(&h, &sv);
                    {
                        let mut t = tl.borrow_mut();
                        t.extend(s.take_events());
                        let over = t.len().saturating_sub(1000);
                        t.drain(..over);
                    }
                    let failed = sv.take_errors();
                    if !failed.is_empty() {
                        return Err(fail(format!("tick {}: {}", h.ticks(), failed.join("; "))));
                    }
                    if let Err(e) = r {
                        return Err(fail(format!("tick {}: {}", h.ticks(), explain(&f, &e))));
                    }
                }
                Ok(())
            })?,
        )?;

        let h = Rc::clone(&app.host);
        g.set(
            "get",
            lua.create_function(move |lua, (e, c): (String, String)| {
                let w = h.world.borrow();
                let v = match w.value_text(&e, &c) {
                    Some(v) => v,
                    None => return Ok(Value::Nil),
                };
                Ok(match w.kind_of(&c) {
                    Some(Kind::Num) => Value::Number(v.parse::<f64>().unwrap_or(0.0)),
                    Some(Kind::Int) => Value::Integer(v.parse::<i64>().unwrap_or(0)),
                    Some(Kind::Bool) => Value::Boolean(v == "true"),
                    _ => Value::String(lua.create_string(&v)?),
                })
            })?,
        )?;

        let h = Rc::clone(&app.host);
        g.set(
            "set",
            lua.create_function(move |_, (e, c, v): (String, String, Value)| {
                let text = from_lua(v)?;
                as_core(&h, |w| if text.is_empty() { w.del(&e, &c) } else { w.set(&e, &c, &text) }).map_err(fail)
            })?,
        )?;

        let k = Rc::clone(held);
        g.set(
            "keys",
            lua.create_function(move |_, list: MultiValue| {
                let mut held = k.borrow_mut();
                held.down.clear();
                for v in list {
                    if let Value::String(s) = v {
                        held.down.insert(s.to_str()?.to_ascii_lowercase());
                    }
                }
                Ok(())
            })?,
        )?;

        let k = Rc::clone(held);
        g.set(
            "press",
            lua.create_function(move |_, key: String| {
                k.borrow_mut().pressed.insert(key.to_ascii_lowercase());
                Ok(())
            })?,
        )?;

        let k = Rc::clone(held);
        g.set(
            "mouse",
            lua.create_function(move |_, (x, y, d): (f64, f64, Option<bool>)| {
                // the button going down is a click, as on the page: no press('mouse') needed
                let mut held = k.borrow_mut();
                let down = d.unwrap_or(false);
                if down && !held.mouse.2 {
                    held.pressed.insert(String::from("mouse"));
                }
                held.mouse = (x, y, down);
                Ok(())
            })?,
        )?;

        let k = Rc::clone(held);
        g.set(
            "mic",
            lua.create_function(move |_, level: f64| {
                k.borrow_mut().mic = level.clamp(0.0, 1.0);
                Ok(())
            })?,
        )?;

        let k = Rc::clone(held);
        g.set(
            "glyph",
            lua.create_function(move |_, w: f64| {
                k.borrow_mut().glyph = w;
                Ok(())
            })?,
        )?;

        let (h, s, f) = (Rc::clone(&app.host), Rc::clone(&app.screen), Rc::clone(&files));
        g.set("draw", lua.create_function(move |_, ()| screen::draw(&h, &s).map_err(|e| fail(explain(&f, &e))))?)?;

        let (h, f) = (Rc::clone(&app.host), Rc::clone(&files));
        g.set(
            "run",
            lua.create_function(move |_, (role, op, args): (String, String, Option<Table>)| {
                let mut pairs: Vec<(String, String)> = Vec::new();
                if let Some(t) = args {
                    for (k, v) in t.pairs::<String, Value>().flatten() {
                        pairs.push((k, from_lua(v)?));
                    }
                }
                let refs: Vec<(&str, &str)> = pairs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                h.run(&role, &op, &refs).map_err(|e| fail(explain(&f, &e)))
            })?,
        )?;

        let (h, f) = (Rc::clone(&app.host), Rc::clone(&files));
        g.set(
            "view",
            lua.create_function(move |lua, (name, arg): (String, Option<String>)| {
                let j = h.view(&name, arg.as_deref()).map_err(|e| fail(explain(&f, &e)))?;
                lua.to_value(&j)
            })?,
        )?;

        let h = Rc::clone(&app.host);
        g.set("ents", lua.create_function(move |_, c: String| Ok(h.world.borrow().with(&c, None)))?)?;

        let h = Rc::clone(&app.host);
        g.set("tick", lua.create_function(move |_, ()| Ok(h.ticks()))?)?;

        g.set("events", lua.create_function(move |_, ()| Ok(std::mem::take(&mut *told.borrow_mut())))?)?;

        g.set("ms", lua.create_function(move |_, ()| Ok(start.elapsed().as_secs_f64() * 1000.0))?)?;

        // driver code is not a code file of the world: the tables of the world say so
        for (name, hint) in [
            ("ecs", "the driver has no ecs: use get(e, c), set(e, c, v), ents(c), step(); ecs works in the code files (sys/, lib/, ...)"),
            ("input", "the driver has no input: it gives the input with keys(k), press(k), mouse(x, y, down); input works in the code files"),
            ("gfx", "the driver has no gfx: draw() gives the frame as text; gfx works in draw/ files"),
        ] {
            let t = lua.create_table()?;
            let m = lua.create_table()?;
            m.set("__index", lua.create_function(move |_, (_, _): (Value, Value)| -> mlua::Result<Value> { Err(fail(String::from(hint))) })?)?;
            t.set_metatable(Some(m))?;
            g.set(name, t)?;
        }

        let o = Rc::clone(out);
        g.set(
            "print",
            lua.create_function(move |_, list: MultiValue| {
                let parts: Vec<String> = list.iter().map(|v| show(v, 0)).collect();
                append(&o, &parts.join("\t"));
                Ok(())
            })?,
        )?;
        Ok(())
    }

    // runs driver code; the answer is what it printed and what it returned. on an error the
    // answer so far comes with it
    pub fn run(&self, name: &str, code: &str) -> Result<String, String> {
        self.budget.arm();
        let r = self.lua.load(code).set_name(name).eval::<MultiValue>();
        match r {
            Ok(values) => {
                let parts: Vec<String> = values.iter().map(|v| show(v, 0)).collect();
                if !parts.is_empty() {
                    append(&self.out, &parts.join("\t"));
                }
                Ok(self.out.borrow().trim_end().to_string())
            }
            Err(e) => {
                let msg = explain(&self.files, &reason_at(&e)).replace(&format!("[string \"{}\"]", name), name);
                let so_far = self.out.borrow().trim_end().to_string();
                Err(if so_far.is_empty() { msg } else { format!("{}\nerror: {}", so_far, msg) })
            }
        }
    }
}

// the tests of a project, each on a fresh world: test/<name>.lua, or only the one named.
// (name, Ok(what it printed) or Err(why it failed), milliseconds)
// one test: its name, what it printed or why it failed, milliseconds
pub type TestResult = (String, Result<String, String>, f64);

pub fn run_tests(project: &Project, only: Option<&str>) -> Result<Vec<TestResult>, String> {
    let (files, _) = project.scan()?;
    let mut out = Vec::new();
    for (path, code) in &files {
        let name = match path.strip_prefix(&format!("{}/", TEST_DIR)).and_then(|f| f.strip_suffix(".lua")) {
            Some(n) => n,
            None => continue,
        };
        if only.is_some_and(|o| o != name) {
            continue;
        }
        let t = Instant::now();
        let r = match project.fresh() {
            Ok((app, problems)) if problems.is_empty() => Driver::new(&app).and_then(|d| d.run(path, code)),
            Ok((_, problems)) => Err(format!("the world has problems: {}", problems.join("; "))),
            Err(e) => Err(e),
        };
        out.push((name.to_string(), r, t.elapsed().as_secs_f64() * 1000.0));
    }
    if out.is_empty() {
        return Err(match only {
            Some(n) => format!("no test/{}.lua", clip(n, 40)),
            None => String::from("no tests: write test/<name>.lua"),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::build;

    fn app() -> App {
        let mut files = BTreeMap::new();
        files.insert(String::from("world.lua"), String::from("return { types = { y = 'num', hits = 'int' }, entities = { pad = { y = 10, hits = 0 } } }"));
        files.insert(
            String::from("sys/pad.lua"),
            String::from("return function(dt)\n  if input.down('up') then ecs.set('pad', 'y', ecs.get('pad', 'y') - 60 * dt) end\n  if input.pressed('space') then ecs.set('pad', 'hits', ecs.get('pad', 'hits') + 1) end\nend"),
        );
        files.insert(String::from("draw/pad.lua"), String::from("return function() gfx.size(100, 50) gfx.rect(2, ecs.get('pad', 'y'), 2, 8, '#fff') end"));
        files.insert(String::from("sys/bad.lua"), String::from("return function(dt)\n  if ecs.get('pad', 'hits') > 1 then\n    error('two hits')\n  end\nend"));
        build(&files).unwrap().0
    }

    #[test]
    fn the_driver_steps_holds_keys_and_reads_the_frame() {
        let a = app();
        let d = Driver::new(&a).unwrap();
        let out = d.run("try", "keys('up') step(30) keys() press('space') step(5) print(get('pad', 'y'), get('pad', 'hits')) return draw()").unwrap();
        assert_eq!(out, "-20\t1\nsize 100 50\nrect 2 -20 2 8 #fff");
        let err = Driver::new(&a).unwrap().run("try", "press('space') step() print('one') press('space') step()").unwrap_err();
        assert!(err.starts_with("one\nerror: try:1: tick 37: sys/bad.lua:3: two hits"), "{}", err);
        // an error of a rust function names the line too: in the driver and in the world
        let err = Driver::new(&a).unwrap().run("try", "local x = 1\nget(nil, 'y')").unwrap_err();
        assert!(err.starts_with("try:2: bad argument"), "{}", err);
        let err = Driver::new(&app()).unwrap().run("try", "set('sys.pad', 'src', 'return function(dt)\\n  ecs.set(nil, 1, 2)\\nend') step()").unwrap_err();
        assert!(err.contains("tick 1: sys/pad.lua:2: bad_column"), "{}", err);
    }

    #[test]
    fn values_are_shown_short() {
        let a = app();
        let d = Driver::new(&a).unwrap();
        assert_eq!(d.run("try", "return 1/3, {1, 2, a = {b = 'x'}}, nil").unwrap(), "0.3333\t{1, 2, a = {b = \"x\"}}\tnil");
        let d = Driver::with_time(&a, Duration::from_millis(200)).unwrap();
        assert!(d.run("try", "while true do end").unwrap_err().contains("time_limit"));
        let d = Driver::with_time(&a, Duration::from_millis(200)).unwrap();
        assert!(d.run("try", "step(100000000)").unwrap_err().contains("time_limit"));
    }

    fn menu_app() -> App {
        let mut files = BTreeMap::new();
        files.insert(String::from("world.lua"), String::from("return { types = { sel = 'num', chosen = 'num' }, entities = { s = { sel = 1, chosen = 0 } } }"));
        files.insert(
            String::from("sys/menu.lua"),
            String::from("return function(dt)\n  local ui = ecs.call('lib.ui')\n  local list = ui.menu({'Play', 'Options', 'Quit'}, 400, 200, {size = 20})\n  local sel, chosen = ui.update_menu(list, ecs.get('s', 'sel'))\n  ecs.set('s', 'sel', sel)\n  if chosen then ecs.set('s', 'chosen', chosen) end\nend"),
        );
        files.insert(
            String::from("draw/menu.lua"),
            String::from("return function()\n  local ui = ecs.call('lib.ui')\n  local list = ui.menu({'Play', 'Options', 'Quit'}, 400, 200, {size = 20})\n  ui.draw_menu(list, ecs.get('s', 'sel'))\n  local w, h = ui.label('Title', 400, 50, {align = 'center', size = 40})\n  ui.paragraph('one two three four', 10, 300, 100, {size = 10})\nend"),
        );
        let (app, _, problems) = build(&files).unwrap();
        assert!(problems.is_empty(), "{:?}", problems);
        app
    }

    #[test]
    fn the_mouse_going_down_is_a_click_and_the_driver_names_what_it_lacks() {
        let mut files = BTreeMap::new();
        files.insert(String::from("world.lua"), String::from("return { types = { n = 'num' }, entities = { p = { n = 0 } } }"));
        files.insert(String::from("sys/c.lua"), String::from("return function() if input.pressed('mouse') then ecs.set('p', 'n', ecs.get('p', 'n') + 1) end end"));
        let (app, _, problems) = crate::project::build(&files).unwrap();
        assert!(problems.is_empty(), "{:?}", problems);
        // down, held, up, down again: two clicks
        let code = "mouse(5, 5, true) step(3) mouse(5, 5, false) step() mouse(5, 5, true) step() return get('p', 'n')";
        assert_eq!(Driver::new(&app).unwrap().run("t", code).unwrap().trim(), "2");
        // the driver is not a code file: it says what to use instead
        let e = Driver::new(&app).unwrap().run("t", "return ecs.get('p', 'n')").unwrap_err();
        assert!(e.contains("the driver has no ecs") && e.contains("get(e, c)"), "{}", e);
    }

    #[test]
    fn a_test_sees_what_the_world_told_the_display() {
        let mut files = BTreeMap::new();
        files.insert(String::from("sys/q.lua"), String::from("return function() if input.pressed('escape') then ecs.quit() end end"));
        let (app, _, problems) = crate::project::build(&files).unwrap();
        assert!(problems.is_empty(), "{:?}", problems);
        let out = Driver::new(&app).unwrap().run("t", "step(3) local a = #events() press('escape') step(2) local e = events() return a, #e, e[1], #events()").unwrap();
        assert_eq!(out.trim(), "0\t1\tquit\t0", "{}", out);
    }

    #[test]
    fn ui_buttons_know_their_size_and_their_clicks() {
        let a = menu_app();
        let frame = Driver::new(&a).unwrap().run("t", "step() return draw()").unwrap();
        // the widest text is Options: 7 * 20 * 0.6 = 84 wide, 20 * 1.25 = 25 high, padding 14
        assert!(frame.contains("rect 344 200 112 39 #3f3f66"), "{}", frame);
        assert!(frame.contains("text 376 207 20 #ffffff Play"), "{}", frame);
        assert!(frame.contains("text 358 260 20 #ffffff Options"), "{}", frame);
        // a title centered at 400: 5 * 40 * 0.6 = 120 wide
        assert!(frame.contains("text 340 50 40 #ffffff Title"), "{}", frame);
        // a paragraph of width 100: 10 * 0.6 = 6 a character, 16 characters fit
        assert!(frame.contains("text 10 300 10 #ffffff one two three") && frame.contains("text 10 312.5 10 #ffffff four"), "{}", frame);
        // another display: the same buttons follow the real width of a character
        let wide = Driver::new(&menu_app()).unwrap().run("t", "glyph(0.5) step() return draw()").unwrap();
        assert!(wide.contains("rect 351 200 98 39"), "{}", wide);
        // a click on the second button chooses it
        let r = Driver::new(&menu_app()).unwrap().run("t", "mouse(400, 270, false) step() mouse(400, 270, true) press('mouse') step() return get('s', 'sel'), get('s', 'chosen')").unwrap();
        assert_eq!(r, "2\t2");
        // a click beside the buttons chooses nothing
        let r = Driver::new(&menu_app()).unwrap().run("t", "mouse(50, 50, true) press('mouse') step() return get('s', 'chosen')").unwrap();
        assert_eq!(r, "0");
        // the keys: down, down, up, enter
        let r = Driver::new(&menu_app()).unwrap().run("t", "press('down') step() press('down') step() press('up') step() press('enter') step() return get('s', 'sel'), get('s', 'chosen')").unwrap();
        assert_eq!(r, "2\t2");
    }
}
