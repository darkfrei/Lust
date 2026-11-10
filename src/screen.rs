// the screen of a world: what its draw code draws (a list of commands, as text) and what
// the person presses (keys, the mouse). there is no window here: a display module (web.rs,
// or another one) shows the frame and gives the input back through an Exchange. lua sees
// two tables:
//   gfx    gfx.size(w, h), gfx.clear(color), gfx.rect(x, y, w, h, color),
//          gfx.circle(x, y, r, color), gfx.line(x1, y1, x2, y2, width, color),
//          gfx.text(x, y, size, color, text): y is the top of the text; only while draw
//          code runs. gfx.measure(text, size) -> w, h of a text, anywhere
//   input  input.down(key), input.pressed(key) (went down since the last tick),
//          input.mouse() -> x, y, down
//          ecs.quit() ends the game (an event, see Exchange::events)
//          gfx.mic(true) asks the display for the microphone, input.mic() -> loudness 0..1
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
// text is monospace: a character is this much of the size wide until the display tells the
// real figure, and a line of text is this much of the size high. gfx.text puts the top of
// that box at y
pub const GLYPH_DEFAULT: f64 = 0.6;
pub const LINE: f64 = 1.25;

// what the person does, as a display reports it
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    // the keys held now
    pub down: BTreeSet<String>,
    // the keys that went down since the last tick
    pub pressed: BTreeSet<String>,
    // x, y in the units of the frame, and whether a button is down
    pub mouse: (f64, f64, bool),
    // the width of one character as a part of the text size; 0 when the display did not say
    pub glyph: f64,
    // how loud the microphone hears, 0 to 1 (only a display that was asked for it, see gfx.mic)
    pub mic: f64,
}

fn key_ok(k: &str) -> bool {
    !k.is_empty() && k.len() <= 20 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn key_set(list: &str) -> BTreeSet<String> {
    list.split(',').map(|k| k.trim().to_ascii_lowercase()).filter(|k| key_ok(k)).take(64).collect()
}

impl Input {
    // the text a display sends: "down=up,space;hit=space;mouse=12.5,3,1;cw=0.6"
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
                "cw" => out.glyph = v.trim().parse::<f64>().ok().filter(|x| (0.3..=1.2).contains(x)).unwrap_or(0.0),
                "mic" => out.mic = v.trim().parse::<f64>().ok().filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0)).unwrap_or(0.0),
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
    pub glyph: f64,
    pub mic: f64,
    // when a page last asked for a frame (it is shown), last said it is there (also hidden),
    // and said it was closed; the program pauses and ends by these (src/bin/lust.rs)
    pub seen: Option<std::time::Instant>,
    pub alive: Option<std::time::Instant>,
    pub bye: Option<std::time::Instant>,
    // what the world tells the display once (a sound, the end of the game): number, when,
    // text. a frame may be missed or taken twice, an event is done exactly once: the display
    // says the number of the last event it has (ack) and gets those after it
    pub events: std::collections::VecDeque<(u64, std::time::Instant, String)>,
    pub last_event: u64,
    // the highest number a display said it has
    pub acked: u64,
}

// how many events wait for the displays at most, and for how long
pub const EVENTS_KEPT: usize = 64;
pub const EVENT_AGE: std::time::Duration = std::time::Duration::from_secs(5);
// how many events the code of one tick may make, and how long one is
pub const EVENTS_TICK_MAX: usize = 16;
pub const EVENT_MAX: usize = 200;

// the number a display says it has: "ack=12" in its input; None for "ack=new" or none
pub fn ack_of(text: &str) -> Option<u64> {
    text.split(';').find_map(|part| part.trim().strip_prefix("ack=")).and_then(|v| v.trim().parse::<u64>().ok())
}

impl Exchange {
    // a display reports the input (the text of Input::parse)
    pub fn put_input(&mut self, text: &str) {
        let i = Input::parse(text);
        self.down = i.down;
        self.hits.extend(i.pressed);
        self.mouse = i.mouse;
        if i.glyph > 0.0 {
            self.glyph = i.glyph;
        }
        self.mic = i.mic;
        let now = std::time::Instant::now();
        self.seen = Some(now);
        self.alive = Some(now);
        self.bye = None;
    }

    // a hidden page says it is still there
    pub fn ping(&mut self) {
        self.alive = Some(std::time::Instant::now());
    }

    // a page was closed (another page may still be open: its next frame takes this back)
    pub fn closed(&mut self) {
        self.bye = Some(std::time::Instant::now());
    }

    // the loop takes the input for one tick: what went down is taken only once
    pub fn take_input(&mut self) -> Input {
        Input { down: self.down.clone(), pressed: std::mem::take(&mut self.hits), mouse: self.mouse, glyph: self.glyph, mic: self.mic }
    }

    fn prune(&mut self) {
        let now = std::time::Instant::now();
        while self.events.len() > EVENTS_KEPT || self.events.front().is_some_and(|(_, t, _)| now.duration_since(*t) > EVENT_AGE) {
            self.events.pop_front();
        }
    }

    // the world tells the displays something once; gives the number of the event
    pub fn push_event(&mut self, text: &str) -> u64 {
        self.last_event += 1;
        let line: String = text.chars().filter(|c| !c.is_control()).take(EVENT_MAX).collect();
        self.events.push_back((self.last_event, std::time::Instant::now(), line));
        self.prune();
        self.last_event
    }

    // the events a display has not had: those after its ack. a display that is new (None),
    // or that knows a number this program never gave (it outlived an older program), gets
    // none and starts from the last number
    pub fn events_after(&mut self, ack: Option<u64>) -> (Vec<(u64, String)>, u64) {
        self.prune();
        match ack {
            Some(a) if a <= self.last_event => {
                self.acked = self.acked.max(a);
                (self.events.iter().filter(|(n, _, _)| *n > a).map(|(n, _, t)| (*n, t.clone())).collect(), self.last_event)
            }
            _ => (Vec::new(), self.last_event),
        }
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
    // what the code of this tick tells the display once (see Exchange::events)
    events: RefCell<Vec<String>>,
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
        Screen { body: RefCell::new(String::new()), cmds: Cell::new(0), drawing: Cell::new(false), size: Cell::new(DEFAULT_SIZE), input: RefCell::new(Input::default()), events: RefCell::new(Vec::new()) }
    }

    // the code of a tick tells the display something once. not from a drawing: a drawing
    // only reads and may run more than once for one tick
    pub fn event(&self, text: &str) -> mlua::Result<()> {
        if self.drawing.get() {
            return Err(fail(String::from("event_in_draw: draw files only read; do this in a sys/ or op/ file")));
        }
        let mut e = self.events.borrow_mut();
        if e.len() >= EVENTS_TICK_MAX {
            return Err(fail(format!("too_many_events: a tick makes at most {}", EVENTS_TICK_MAX)));
        }
        e.push(text.chars().filter(|c| !c.is_control()).take(EVENT_MAX).collect());
        Ok(())
    }

    // the events since the last time; the caller hands them to the display (or to a test)
    pub fn take_events(&self) -> Vec<String> {
        std::mem::take(&mut *self.events.borrow_mut())
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

    // the size of a text as the display will draw it: width, height
    pub fn measure(&self, text: &str, size: f64) -> (f64, f64) {
        let g = self.input.borrow().glyph;
        let g = if g > 0.0 { g } else { GLYPH_DEFAULT };
        (text.chars().count() as f64 * size * g, size * LINE)
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

// a text of a lua value: a string, a number or a boolean; one line
fn text_of(v: Value) -> mlua::Result<String> {
    let text = match v {
        Value::String(s) => s.to_str()?.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => num(n),
        Value::Boolean(b) => b.to_string(),
        _ => String::new(),
    };
    Ok(text.chars().map(|ch| if ch == '\n' || ch == '\r' { ' ' } else { ch }).collect())
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
    // gfx.mic(true): ask the display to listen to the microphone (the person is asked by the
    // browser and sees that it is on); input.mic() then gives the loudness, 0 to 1. only the
    // loudness reaches the world, never the sound
    let sc = Rc::clone(s);
    t.set("mic", lua.create_function(move |_, on: bool| if on { sc.push(String::from("mic on")) } else { Ok(()) })?)?;
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
            let text = text_of(text)?;
            sc.push(format!("text {} {} {} {} {}", num(x), num(y), num(size), color(c)?, clip(&text, TEXT_MAX)))
        })?,
    )?;
    let sc = Rc::clone(s);
    t.set(
        "measure",
        lua.create_function(move |_, (text, size): (Value, f64)| {
            if !(size.is_finite() && size > 0.0 && size <= 1000.0) {
                return Err(fail(String::from("bad_size: the size of a text is 1..1000")));
            }
            let text = clip(&text_of(text)?, TEXT_MAX);
            Ok(sc.measure(&text, size))
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
    let sc = Rc::clone(s);
    t.set("mic", lua.create_function(move |_, ()| Ok(sc.input.borrow().mic))?)?;
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
    // ecs.quit(): the game ends itself (the event quit; the program decides what that means)
    let sc = Rc::clone(&s);
    let quit = lua.create_function(move |_, ()| sc.event("quit")).map_err(|e| e.to_string())?;
    host.add_to_ecs("quit", quit)?;
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
    fn a_text_has_a_size_that_follows_the_display() {
        let (h, s) = host();
        {
            let mut w = h.world.borrow_mut();
            w.set("sys.a", "tick", "true").unwrap();
            w.set("x", "kind", "num").unwrap();
            w.set("sys.a", "src", "return function() local w, hh = gfx.measure('abcd', 10) ecs.set('p', 'x', w) ecs.set('q', 'x', hh) end").unwrap();
        }
        h.tick(0.01).unwrap();
        assert_eq!(h.world.borrow().value_text("p", "x").as_deref(), Some("24"));
        assert_eq!(h.world.borrow().value_text("q", "x").as_deref(), Some("12.5"));
        // the display says its figure: a character is 0.5 of the size
        let mut ex = Exchange::default();
        ex.put_input("cw=0.5");
        *s.input.borrow_mut() = ex.take_input();
        h.tick(0.01).unwrap();
        assert_eq!(h.world.borrow().value_text("p", "x").as_deref(), Some("20"));
        ex.put_input("cw=7");
        assert_eq!(ex.glyph, 0.5);
        assert_eq!(Input::parse("cw=nope").glyph, 0.0);
        assert_eq!(Input::parse("mic=2").mic, 1.0);
        assert_eq!(Input::parse("mic=nope").mic, 0.0);
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

    #[test]
    fn an_event_reaches_a_display_exactly_once() {
        let mut ex = Exchange::default();
        // a new display gets nothing old and learns where to start
        assert_eq!(ex.push_event("sound a 1 1"), 1);
        assert_eq!(ex.events_after(None), (vec![], 1));
        assert_eq!(ex.events_after(ack_of("down=;ack=new")), (vec![], 1));
        ex.push_event("quit");
        ex.push_event("sound\nb");
        // asking twice with the same ack gives the same events (a lost answer is asked again)
        let want = vec![(2, String::from("quit")), (3, String::from("soundb"))];
        assert_eq!(ex.events_after(ack_of("ack=1;down=up")), (want.clone(), 3));
        assert_eq!(ex.events_after(Some(1)), (want, 3));
        // once the display says it has them, they do not come again
        assert_eq!(ex.events_after(Some(3)), (vec![], 3));
        assert_eq!(ex.acked, 3);
        // a display that knows a number this program never gave starts anew
        assert_eq!(ex.events_after(Some(99)), (vec![], 3));
        assert_eq!(ex.acked, 3);
        // the queue has an end
        for _ in 0..200 {
            ex.push_event("x");
        }
        assert_eq!(ex.events.len(), EVENTS_KEPT);
        assert_eq!(ex.events_after(Some(3)).0.len(), EVENTS_KEPT);
        // and old events go away
        ex.events.iter_mut().for_each(|e| e.1 -= EVENT_AGE + std::time::Duration::from_secs(1));
        assert_eq!(ex.events_after(Some(3)), (vec![], 203));
    }

    #[test]
    fn the_code_of_a_tick_makes_events_and_a_drawing_cannot() {
        let host = Host::with_spec(Spec::game()).unwrap();
        let s = attach(&host).unwrap();
        {
            let mut w = host.world.borrow_mut();
            for t in ["src", "tick", "draw"] {
                w.set(t, "kind", if t == "src" { "code" } else { "bool" }).unwrap();
            }
            w.set("sys.a", "src", "return function() ecs.quit() end").unwrap();
            w.set("sys.a", "tick", "true").unwrap();
        }
        host.tick(0.016).unwrap();
        assert_eq!(s.take_events(), vec![String::from("quit")]);
        assert!(s.take_events().is_empty());
        // more than a tick may make
        for _ in 0..EVENTS_TICK_MAX {
            s.event("x").unwrap();
        }
        assert!(s.event("x").unwrap_err().to_string().contains("too_many_events"));
        s.take_events();
        // a drawing only reads
        {
            let mut w = host.world.borrow_mut();
            w.set("draw.a", "src", "return function() ecs.quit() end").unwrap();
            w.set("draw.a", "draw", "true").unwrap();
        }
        let e = draw(&host, &s).unwrap_err();
        assert!(e.contains("event_in_draw"), "{}", e);
        assert!(s.take_events().is_empty());
    }
}
