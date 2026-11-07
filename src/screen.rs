// the screen of a world: what its draw code draws (a list of commands, as text) and what
// the person presses (keys, the mouse). there is no window here: a display module (web.rs,
// or another one) shows the frame and gives the input back through an Exchange. lua sees
// two tables:
//   gfx    gfx.size(w, h), gfx.clear(color), gfx.rect(x, y, w, h, color),
//          gfx.circle(x, y, r, color), gfx.line(x1, y1, x2, y2, width, color),
//          gfx.text(x, y, size, color, text); only while draw code runs
//   input  input.down(key), input.pressed(key) (went down since the last tick),
//          input.mouse() -> x, y, down
// the frame, one command per line:
//   size w h | clear c | rect x y w h c | circle x y r c | line x1 y1 x2 y2 width c |
//   text x y size c the text
use crate::host::{clip, Host};
use mlua::{Lua, Table, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

// how many commands one frame may hold
pub const CMDS_MAX: usize = 20_000;
// how long one text of a frame may be
pub const TEXT_MAX: usize = 200;
pub const DEFAULT_SIZE: (f64, f64) = (800.0, 600.0);

// what the person does, as a display reports it
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    // the keys held now
    pub down: BTreeSet<String>,
    // the keys that went down since the last tick
    pub pressed: BTreeSet<String>,
    // x, y in the units of the frame, and whether a button is down
    pub mouse: (f64, f64, bool),
}

fn key_ok(k: &str) -> bool {
    !k.is_empty() && k.len() <= 20 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn key_set(list: &str) -> BTreeSet<String> {
    list.split(',').map(|k| k.trim().to_ascii_lowercase()).filter(|k| key_ok(k)).take(64).collect()
}

impl Input {
    // the text a display sends: "down=up,space;hit=space;mouse=12.5,3,1"
    pub fn parse(text: &str) -> Input {
        let mut out = Input::default();
        for part in text.split(';') {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            match k.trim() {
                "down" => out.down = key_set(v),
                "hit" => out.pressed = key_set(v),
                "mouse" => {
                    let n: Vec<&str> = v.split(',').collect();
                    let num = |i: usize| n.get(i).and_then(|s| s.trim().parse::<f64>().ok()).filter(|x| x.is_finite()).unwrap_or(0.0);
                    out.mouse = (num(0), num(1), n.get(2).map(|s| s.trim()) == Some("1"));
                }
                _ => {}
            }
        }
        out
    }
}

// what a display and the loop of the program share: the last frame and the input since.
// the display thread holds it behind a mutex; nothing of lua is in it
#[derive(Clone, Debug, Default)]
pub struct Exchange {
    pub frame: String,
    // empty while the world runs, else why it stopped
    pub status: String,
    pub seq: u64,
    pub down: BTreeSet<String>,
    pub hits: BTreeSet<String>,
    pub mouse: (f64, f64, bool),
}

impl Exchange {
    // a display reports the input (the text of Input::parse)
    pub fn put_input(&mut self, text: &str) {
        let i = Input::parse(text);
        self.down = i.down;
        self.hits.extend(i.pressed);
        self.mouse = i.mouse;
    }

    // the loop takes the input for one tick: what went down is taken only once
    pub fn take_input(&mut self) -> Input {
        Input { down: self.down.clone(), pressed: std::mem::take(&mut self.hits), mouse: self.mouse }
    }

    pub fn put_frame(&mut self, frame: String, status: &str) {
        self.frame = frame;
        self.status = status.to_string();
        self.seq += 1;
    }
}

pub struct Screen {
    body: RefCell<String>,
    cmds: Cell<usize>,
    drawing: Cell<bool>,
    size: Cell<(f64, f64)>,
    pub input: RefCell<Input>,
}

// a number of a frame: two decimals at most, no trailing zeros
pub fn num(x: f64) -> String {
    if !x.is_finite() {
        return String::from("0");
    }
    let r = (x * 100.0).round() / 100.0;
    let s = format!("{:.2}", r);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        String::from("0")
    } else {
        s.to_string()
    }
}

fn color_ok(c: &str) -> bool {
    !c.is_empty() && c.len() <= 24 && c.chars().all(|x| x.is_ascii_alphanumeric() || x == '#')
}

fn fail(msg: String) -> mlua::Error {
    mlua::Error::runtime(msg)
}

impl Screen {
    fn new() -> Screen {
        Screen { body: RefCell::new(String::new()), cmds: Cell::new(0), drawing: Cell::new(false), size: Cell::new(DEFAULT_SIZE), input: RefCell::new(Input::default()) }
    }

    fn push(&self, line: String) -> mlua::Result<()> {
        if !self.drawing.get() {
            return Err(fail(String::from("gfx_outside_draw: gfx works only in draw/ files")));
        }
        if self.cmds.get() >= CMDS_MAX {
            return Err(fail(format!("too_many_cmds: a frame holds at most {}", CMDS_MAX)));
        }
        self.cmds.set(self.cmds.get() + 1);
        let mut b = self.body.borrow_mut();
        b.push_str(&line);
        b.push('\n');
        Ok(())
    }

    // a frame begins: nothing drawn yet
    fn begin(&self) {
        self.body.borrow_mut().clear();
        self.cmds.set(0);
        self.size.set(DEFAULT_SIZE);
    }

    fn frame(&self) -> String {
        let (w, h) = self.size.get();
        format!("size {} {}\n{}", num(w), num(h), self.body.borrow())
    }

    // the key of a lua argument, lowercase
    fn key(k: &str) -> String {
        k.to_ascii_lowercase()
    }
}

fn color(c: mlua::LuaString) -> mlua::Result<String> {
    let s = c.to_str()?.to_string();
    if color_ok(&s) {
        Ok(s)
    } else {
        Err(fail(format!("bad_color: {}", clip(&s, 24))))
    }
}

fn gfx_table(lua: &Lua, s: &Rc<Screen>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let sc = Rc::clone(s);
    t.set(
        "size",
        lua.create_function(move |_, (w, h): (f64, f64)| {
            if !sc.drawing.get() {
                return Err(fail(String::from("gfx_outside_draw: gfx works only in draw/ files")));
            }
            if !(w.is_finite() && h.is_finite() && w >= 1.0 && h >= 1.0 && w <= 10000.0 && h <= 10000.0) {
                return Err(fail(String::from("bad_size: 1..10000")));
            }
            sc.size.set((w, h));
            Ok(())
        })?,
    )?;
    let sc = Rc::clone(s);
    t.set("clear", lua.create_function(move |_, c: mlua::LuaString| sc.push(format!("clear {}", color(c)?)))?)?;
    let sc = Rc::clone(s);
    t.set(
        "rect",
        lua.create_function(move |_, (x, y, w, h, c): (f64, f64, f64, f64, mlua::LuaString)| sc.push(format!("rect {} {} {} {} {}", num(x), num(y), num(w), num(h), color(c)?)))?,
    )?;
    let sc = Rc::clone(s);
    t.set("circle", lua.create_function(move |_, (x, y, r, c): (f64, f64, f64, mlua::LuaString)| sc.push(format!("circle {} {} {} {}", num(x), num(y), num(r), color(c)?)))?)?;
    let sc = Rc::clone(s);
    t.set(
        "line",
        lua.create_function(move |_, (x1, y1, x2, y2, w, c): (f64, f64, f64, f64, f64, mlua::LuaString)| {
            sc.push(format!("line {} {} {} {} {} {}", num(x1), num(y1), num(x2), num(y2), num(w), color(c)?))
        })?,
    )?;
    let sc = Rc::clone(s);
    t.set(
        "text",
        lua.create_function(move |_, (x, y, size, c, text): (f64, f64, f64, mlua::LuaString, Value)| {
            let text = match text {
                Value::String(s) => s.to_str()?.to_string(),
                Value::Integer(i) => i.to_string(),
                Value::Number(n) => num(n),
                Value::Boolean(b) => b.to_string(),
                _ => String::new(),
            };
            let text: String = text.chars().map(|ch| if ch == '\n' || ch == '\r' { ' ' } else { ch }).collect();
            sc.push(format!("text {} {} {} {} {}", num(x), num(y), num(size), color(c)?, clip(&text, TEXT_MAX)))
        })?,
    )?;
    Ok(t)
}

fn input_table(lua: &Lua, s: &Rc<Screen>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let sc = Rc::clone(s);
    t.set("down", lua.create_function(move |_, k: String| Ok(sc.input.borrow().down.contains(&Screen::key(&k))))?)?;
    let sc = Rc::clone(s);
    t.set("pressed", lua.create_function(move |_, k: String| Ok(sc.input.borrow().pressed.contains(&Screen::key(&k))))?)?;
    let sc = Rc::clone(s);
    t.set(
        "mouse",
        lua.create_function(move |_, ()| {
            let (x, y, d) = sc.input.borrow().mouse;
            Ok((x, y, d))
        })?,
    )?;
    Ok(t)
}

// gives the world of a host the tables gfx and input. before any code of the world runs:
// a world that keeps its lua makes its environment once
pub fn attach(host: &Host) -> Result<Rc<Screen>, String> {
    let s = Rc::new(Screen::new());
    let lua = host.lua();
    let g = gfx_table(lua, &s).map_err(|e| e.to_string())?;
    let i = input_table(lua, &s).map_err(|e| e.to_string())?;
    host.add_global("gfx", Value::Table(g))?;
    host.add_global("input", Value::Table(i))?;
    Ok(s)
}

// the entities whose code draws: draw = true, by ord, then by name
pub fn drawers(host: &Host) -> Vec<String> {
    let w = host.world.borrow();
    let mut list = w.with("draw", Some("true"));
    list.sort_by_key(|e| (w.value(e, "ord").and_then(|o| o.parse::<i64>().ok()).unwrap_or(0), e.clone()));
    list
}

// one frame: every drawing runs read-only, in order. the error names the entity
pub fn draw(host: &Host, s: &Screen) -> Result<String, String> {
    s.begin();
    s.drawing.set(true);
    let mut r = Ok(());
    for e in drawers(host) {
        if let Err(msg) = host.read_only_call_with(&e, ()) {
            r = Err(format!("{}: {}", e, msg));
            break;
        }
    }
    s.drawing.set(false);
    r.map(|_| s.frame())
}

// the end of a tick: what went down counted once
pub fn after_tick(s: &Screen) {
    s.input.borrow_mut().pressed.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::Spec;

    fn host() -> (Host, Rc<Screen>) {
        let h = Host::with_spec(Spec::game()).unwrap();
        let s = attach(&h).unwrap();
        {
            let mut w = h.world.borrow_mut();
            for (e, c, v) in [("src", "kind", "code"), ("tick", "kind", "bool"), ("draw", "kind", "bool"), ("x", "kind", "num")] {
                w.set(e, c, v).unwrap();
            }
        }
        (h, s)
    }

    #[test]
    fn a_frame_is_text_and_numbers_are_short() {
        let (h, s) = host();
        {
            let mut w = h.world.borrow_mut();
            w.set("ball", "x", "12.3456").unwrap();
            w.set("draw.main", "draw", "true").unwrap();
            w.set("draw.main", "src", "return function() gfx.size(100, 60) gfx.clear('#000') gfx.rect(ecs.get('ball', 'x'), 2, 3, 4, 'white') gfx.text(1, 2, 8, '#fff', 'a\\nb') end").unwrap();
        }
        let f = draw(&h, &s).unwrap();
        assert_eq!(f, "size 100 60\nclear #000\nrect 12.35 2 3 4 white\ntext 1 2 8 #fff a b\n");
        assert_eq!(num(-0.001), "0");
        assert_eq!(num(3.0), "3");
    }

    #[test]
    fn gfx_only_in_draw_code_and_colors_are_checked() {
        let (h, s) = host();
        {
            let mut w = h.world.borrow_mut();
            w.set("sys.a", "tick", "true").unwrap();
            w.set("sys.a", "src", "return function() gfx.rect(0, 0, 1, 1, '#fff') end").unwrap();
        }
        assert!(h.tick(0.01).unwrap_err().contains("gfx_outside_draw"));
        h.world.borrow_mut().del("sys.a", "tick").unwrap();
        {
            let mut w = h.world.borrow_mut();
            w.set("draw.a", "draw", "true").unwrap();
            w.set("draw.a", "src", "return function() gfx.clear('red;x') end").unwrap();
        }
        assert!(draw(&h, &s).unwrap_err().contains("bad_color"));
    }

    #[test]
    fn input_reaches_the_world_and_pressed_counts_once() {
        let (h, s) = host();
        {
            let mut w = h.world.borrow_mut();
            w.set("p", "x", "0").unwrap();
            w.set("sys.a", "tick", "true").unwrap();
            w.set("sys.a", "src", "return function() local n = 0 if input.down('up') then n = n + 1 end if input.pressed('space') then n = n + 10 end local mx = input.mouse() ecs.set('p', 'x', ecs.get('p', 'x') + n + mx) end").unwrap();
        }
        let mut ex = Exchange::default();
        ex.put_input("down=Up,bad key;hit=space;mouse=100,2,1");
        *s.input.borrow_mut() = ex.take_input();
        h.tick(0.01).unwrap();
        after_tick(&s);
        h.tick(0.01).unwrap();
        // 1 + 10 + 100, then 1 + 100
        assert_eq!(h.world.borrow().value_text("p", "x").as_deref(), Some("212"));
        assert!(ex.take_input().pressed.is_empty());
    }
}
