// a display in a window of its own, without a browser: the window shows the frames of a world
// (screen.rs) and gives the keys and the mouse back through the same Exchange as the page of
// web.rs, with the same text (screen::Input::parse), so the world cannot tell them apart.
// winit makes the window and reads the keys; pixels puts a picture into it through the video
// card (DirectX 12 or Vulkan on Windows), which is what the overlay of Steam needs; the
// picture itself is drawn by the processor (draw.rs); cpal hears the microphone when the
// world asks for it (mic.rs), and only its loudness goes on. in lust dev the microphone stays
// on and its words go to the AI (voice.rs, vosk), and a squircle in the corner shows both:
// the ring how the AI is linked, the inside how loud the person speaks. the window of lust dev
// is hidden until the AI works on a game (Exchange::show), and a closed one is only hidden
// again (the microphone off): the AI goes on, and its next work shows the window. leave it out with the
// feature "native".
// the window runs on the main thread (winit wants that); the world runs on a thread of its own.
// every window has its own Exchange: today lust opens one, a second one is another Win
use crate::screen::{Exchange, Link};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub mod draw;
pub mod mic;
pub mod voice;

use draw::{Canvas, Font, Place};
use pixels::{Pixels, SurfaceTexture};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, PhysicalKey};
use winit::window::{Window, WindowId};

// a window with nothing to show says it is there this often (a minimized window draws nothing,
// and the program must not think it was closed)
const PING_EVERY: Duration = Duration::from_secs(5);
// a window draws when the world made a new frame, and at least this often (the input goes
// to the world with a drawing); between, it looks for a new frame this often. so a window
// costs nothing when the video card does not wait for the screen
const DRAW_AT_LEAST: Duration = Duration::from_millis(100);
const LOOK_EVERY: Duration = Duration::from_millis(2);
// the color of the status line (why the world stopped), as on the page
const STATUS_COLOR: [u8; 4] = [0xff, 0x77, 0x77, 255];
const NOTE_COLOR: [u8; 4] = [0xdd, 0xdd, 0xdd, 255];
const WORDS_COLOR: [u8; 4] = [0xff, 0xff, 0xff, 255];
const TROUBLE_COLOR: [u8; 4] = [0xff, 0xaa, 0x55, 255];
// a phrase the person said stays under the sign this long
const SAID_FOR: Duration = Duration::from_secs(6);

// ---------- the keys ----------

// the name of a key for the world, as the page of web.rs gives it: a few keys by their
// meaning, a letter or a digit by its place on the keyboard (KeyR is r in any layout), else
// the name of the key in lowercase. named: the meaning of a key that has a name (winit
// NamedKey, "ArrowUp"), code: its place ("KeyR", "Digit1"), ch: the character it types
pub fn key_name(named: Option<&str>, code: Option<&str>, ch: Option<&str>) -> Option<String> {
    let fixed = match named.unwrap_or("") {
        "ArrowUp" => Some("up"),
        "ArrowDown" => Some("down"),
        "ArrowLeft" => Some("left"),
        "ArrowRight" => Some("right"),
        "Space" => Some("space"),
        "Enter" => Some("enter"),
        "Escape" => Some("escape"),
        "Shift" => Some("shift"),
        "Control" => Some("ctrl"),
        "Alt" => Some("alt"),
        "Tab" => Some("tab"),
        "Backspace" => Some("backspace"),
        "F1" => Some("f1"),
        _ => None,
    };
    if let Some(f) = fixed {
        return Some(f.to_string());
    }
    if ch == Some(" ") {
        return Some(String::from("space"));
    }
    let code = code.unwrap_or("");
    if let Some(l) = code.strip_prefix("Key").filter(|l| l.len() == 1 && l.chars().all(|c| c.is_ascii_uppercase())) {
        return Some(l.to_ascii_lowercase());
    }
    if let Some(d) = code.strip_prefix("Digit").filter(|d| d.len() == 1 && d.chars().all(|c| c.is_ascii_digit())) {
        return Some(d.to_string());
    }
    match named {
        // the browser calls this key Meta
        Some("Super") | Some("Meta") => Some(String::from("meta")),
        Some(n) => Some(n.to_ascii_lowercase()),
        None => ch.map(|c| c.to_lowercase()),
    }
}

fn key_of(e: &KeyEvent) -> Option<String> {
    let named = match &e.logical_key {
        Key::Named(n) => Some(format!("{:?}", n)),
        _ => None,
    };
    let code = match e.physical_key {
        PhysicalKey::Code(c) => Some(format!("{:?}", c)),
        _ => None,
    };
    let ch = match &e.logical_key {
        Key::Character(s) => Some(s.to_string()),
        _ => None,
    };
    key_name(named.as_deref(), code.as_deref(), ch.as_deref())
}

// ---------- what a window knows of the person ----------

// the keys, the mouse and the last event, kept between frames as the page keeps them
#[derive(Default)]
pub struct Person {
    down: BTreeSet<String>,
    hits: BTreeSet<String>,
    // the mouse in pixels of the window, and whether a button is down
    mouse: (f64, f64, bool),
    // the number of the last event the window has; None until the first frame
    ack: Option<u64>,
    // how loud the microphone hears, 0..1 (0 while the world does not ask for it)
    pub mic: f32,
}

impl Person {
    pub fn key(&mut self, name: String, pressed: bool) {
        if pressed {
            if !self.down.contains(&name) {
                self.hits.insert(name.clone());
            }
            self.down.insert(name);
        } else {
            self.down.remove(&name);
        }
    }

    pub fn button(&mut self, pressed: bool) {
        self.mouse.2 = pressed;
        if pressed {
            self.hits.insert(String::from("mouse"));
        }
    }

    // the window lost the focus: no key is held any more (the page does the same)
    pub fn blur(&mut self) {
        self.down.clear();
        self.mouse.2 = false;
    }

    // the input as the page sends it; the mouse in the units of the frame
    pub fn text(&mut self, pl: &Place, glyph: f32) -> String {
        let mx = (self.mouse.0 - pl.x as f64) / pl.k.max(1e-6) as f64;
        let my = (self.mouse.1 - pl.y as f64) / pl.k.max(1e-6) as f64;
        let list = |s: &BTreeSet<String>| s.iter().cloned().collect::<Vec<_>>().join(",");
        let t = format!(
            "down={};hit={};mouse={:.1},{:.1},{};cw={:.4};mic={:.3};ack={}",
            list(&self.down),
            list(&self.hits),
            mx,
            my,
            if self.mouse.2 { 1 } else { 0 },
            glyph,
            self.mic,
            self.ack.map(|a| a.to_string()).unwrap_or_else(|| String::from("new"))
        );
        self.hits.clear();
        t
    }
}

// one frame through the exchange, as one POST /frame of the page: the input in; the frame,
// the status and the events out. gives the frame, the status and whether the game asked to
// quit. the event quit is acknowledged at once (the program waits for that to end)
pub fn trade(ex: &Mutex<Exchange>, person: &mut Person, pl: &Place, glyph: f32) -> (String, String, bool) {
    let mut e = ex.lock().unwrap_or_else(|p| p.into_inner());
    let input = person.text(pl, glyph);
    e.put_input(&input);
    let (events, last) = e.events_after(person.ack);
    let mut quit = false;
    match person.ack {
        // a new window starts at the last event: what came before is not for it
        None => person.ack = Some(last),
        Some(_) => {
            for (n, text) in events {
                person.ack = Some(n);
                if text.split(' ').next() == Some("quit") {
                    quit = true;
                }
            }
        }
    }
    // the program learns at once what the window has
    e.events_after(person.ack);
    (e.frame.clone(), e.status.clone(), quit)
}

// ---------- the windows ----------

struct Win {
    window: Arc<Window>,
    pixels: Pixels<'static>,
    ex: Arc<Mutex<Exchange>>,
    person: Person,
    // where the last frame lay (for the mouse) and its size (the window takes it once)
    place: Place,
    sized: bool,
    ended: bool,
    // the number of the frame of the world last asked for, and when
    seq: u64,
    asked: Instant,
    mic: mic::Mic,
    // lust dev: the words of the person (started when the program asks for it), the last
    // phrase and when it was said, the words being said now, and why there are no words
    hearing: Option<voice::Hearing>,
    said: Option<(String, Instant)>,
    partial: String,
    voice: String,
    title: String,
    // lust dev: hidden (not shown yet, or closed by the person), and the number of the last
    // request to show it that was served
    hidden: bool,
    shown_seq: u64,
}

// what the sign in the corner shows
pub struct Sign {
    pub link: Link,
    // the microphone: None while off, else its level 0..1
    pub level: Option<f32>,
    // a line under the sign: the words of the person, or a trouble (true)
    pub words: String,
    pub trouble: bool,
    // the note beside it ("AI listening", "microphone on", a failed microphone)
    pub note: String,
}

// the color of the ring of the sign: how the AI is linked
pub fn ring_color(link: Link) -> [u8; 4] {
    match link {
        Link::None | Link::Waiting => [0x88, 0x88, 0x88, 255],
        Link::Connected => [0x44, 0xcc, 0x44, 255],
        Link::Listening => [0x44, 0xcc, 0xff, 255],
    }
}

// the note beside the sign
pub fn link_note(link: Link) -> &'static str {
    match link {
        Link::None => "",
        Link::Waiting => "waiting for the AI",
        Link::Connected => "AI connected",
        Link::Listening => "AI listening",
    }
}

// a picture in the corners of the window, in its pixels: the status (why the world stopped)
// bottom left, and the sign top right: a squircle, its ring the AI, its inside the microphone
fn overlay(cv: &mut Canvas, font: &Font, scale: f32, status: &str, sign: Option<&Sign>) {
    let size = 14.0 * scale;
    if !status.is_empty() {
        let y = cv.h as f32 - 8.0 * scale - size * crate::screen::LINE as f32;
        cv.text(font, 8.0 * scale, y, size, STATUS_COLOR, status);
    }
    let Some(sg) = sign else { return };
    let r = 17.0 * scale;
    let (cx, cy) = (cv.w as f32 - 8.0 * scale - r, 8.0 * scale + r);
    let line = size * crate::screen::LINE as f32;
    let width = |t: &str, sz: f32| t.chars().count() as f32 * font.advance * sz;
    // the texts left of the sign, on a dark plate
    let words: String = sg.words.chars().take(60).collect();
    let wsize = 16.0 * scale;
    let tw = width(&sg.note, size).max(width(&words, wsize));
    if tw > 0.0 {
        let x = cx - r - 10.0 * scale - tw;
        cv.rect(x - 6.0 * scale, cy - r, tw + 10.0 * scale, 2.0 * r, [0, 0, 0, 166]);
        cv.text(font, cx - r - 10.0 * scale - width(&sg.note, size), cy - r + 2.0 * scale, size, NOTE_COLOR, &sg.note);
        if !words.is_empty() {
            let c = if sg.trouble { TROUBLE_COLOR } else { WORDS_COLOR };
            cv.text(font, cx - r - 10.0 * scale - width(&words, wsize), cy - r + 2.0 * scale + line, wsize, c, &words);
        }
    }
    cv.squircle(cx, cy, r, ring_color(sg.link));
    cv.squircle(cx, cy, r - 3.0 * scale, [0x1a, 0x1a, 0x1a, 255]);
    match sg.level {
        // the inside grows with the voice; a small heart shows that the microphone is on
        Some(l) => cv.squircle(cx, cy, (r - 6.0 * scale) * l.clamp(0.25, 1.0), [0xff, 0x4d, 0x4d, 255]),
        None => cv.line(cx - 5.0 * scale, cy, cx + 5.0 * scale, cy, 2.0 * scale, [0x66, 0x66, 0x66, 255]),
    }
}

// what the sign shows now; None when there is nothing to show (no AI, no microphone)
fn sign_of(mic: &mic::Mic, said: &Option<(String, Instant)>, partial: &str, voice: &str, link: Link, now: Instant) -> Option<Sign> {
    let level = if mic.state == mic::State::On { Some(mic.level) } else { None };
    let failed = match &mic.state {
        mic::State::Failed(e) => Some(format!("microphone: {}", e.chars().take(50).collect::<String>())),
        _ => None,
    };
    if link == Link::None && level.is_none() && failed.is_none() {
        return None;
    }
    let note = match (&failed, link) {
        (Some(f), _) => f.clone(),
        (None, Link::None) => String::from("microphone on"),
        (None, l) => link_note(l).to_string(),
    };
    let fresh = said.as_ref().filter(|(_, t)| now.duration_since(*t) < SAID_FOR).map(|(p, _)| p.clone());
    let (words, trouble) = if !partial.is_empty() {
        (format!("{}...", partial), false)
    } else if let Some(p) = fresh {
        (format!("\"{}\"", p), false)
    } else if voice.starts_with("no voice") {
        (voice.to_string(), true)
    } else if voice == "loading" {
        (String::from("loading the voice model..."), false)
    } else {
        (String::new(), false)
    };
    Some(Sign { link, level, words, trouble, note })
}

impl Win {
    fn open(el: &ActiveEventLoop, title: &str, ex: Arc<Mutex<Exchange>>, hidden: bool) -> Result<Win, String> {
        let attrs = Window::default_attributes().with_title(title).with_visible(!hidden).with_inner_size(LogicalSize::new(crate::screen::DEFAULT_SIZE.0, crate::screen::DEFAULT_SIZE.1));
        let window = Arc::new(el.create_window(attrs).map_err(|e| format!("no_window: {}", e))?);
        let size = window.inner_size();
        let surface = SurfaceTexture::new(size.width.max(1), size.height.max(1), Arc::clone(&window));
        let pixels = Pixels::new(size.width.max(1), size.height.max(1), surface).map_err(|e| format!("no_video: {}", e))?;
        let place = draw::place(size.width as usize, size.height as usize, 800.0, 600.0);
        Ok(Win {
            window,
            pixels,
            ex,
            person: Person::default(),
            place,
            sized: false,
            ended: false,
            seq: u64::MAX,
            asked: Instant::now(),
            mic: mic::Mic::default(),
            hearing: None,
            said: None,
            partial: String::new(),
            voice: String::new(),
            title: title.to_string(),
            hidden,
            shown_seq: 0,
        })
    }

    fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        let _ = self.pixels.resize_surface(w, h);
        let _ = self.pixels.resize_buffer(w, h);
    }

    fn redraw(&mut self, font: &Font) {
        let (frame, status, quit) = trade(&self.ex, &mut self.person, &self.place, font.advance);
        if quit {
            self.mic.stop();
            self.ended = true;
            self.window.set_visible(false);
            return;
        }
        // the window takes the size of the first frame once (the person may change it after)
        if !self.sized && !frame.is_empty() {
            self.sized = true;
            let (w, h) = draw::size_of(&frame);
            let _ = self.window.request_inner_size(LogicalSize::new(w as f64, h as f64));
        }
        let size = self.window.inner_size();
        let (w, h) = (size.width as usize, size.height as usize);
        if w == 0 || h == 0 || self.pixels.frame().len() != w * h * 4 {
            return;
        }
        let scale = self.window.scale_factor() as f32;
        let mut cv = Canvas::new(w, h, self.pixels.frame_mut());
        let (pl, said) = draw::frame(&mut cv, font, &frame);
        let now = Instant::now();
        let (link, listen, title) = {
            let e = self.ex.lock().unwrap_or_else(|p| p.into_inner());
            (e.link, e.listen, e.title.clone())
        };
        if !title.is_empty() && title != self.title {
            self.window.set_title(&format!("lust: {}", title));
            self.title = title;
        }
        // lust dev: the words of the person; the microphone sends its sound there from now on
        if listen && self.hearing.is_none() {
            let h = voice::Hearing::start();
            self.mic.words = Some(h.tx.clone());
            if self.mic.state == mic::State::On {
                self.mic.stop();
            }
            self.hearing = Some(h);
        }
        if let Some(h) = &self.hearing {
            let (phrases, partial, state) = h.take();
            if !phrases.is_empty() {
                let mut e = self.ex.lock().unwrap_or_else(|p| p.into_inner());
                for p in &phrases {
                    e.put_heard(p);
                }
            }
            if let Some(p) = phrases.last() {
                self.said = Some((p.clone(), now));
            }
            self.partial = partial;
            if state != self.voice {
                self.ex.lock().unwrap_or_else(|p| p.into_inner()).voice = state.clone();
                self.voice = state;
            }
        }
        // the microphone: on while the frames ask for it, and in lust dev; its level goes
        // with the next input
        self.mic.want(said.mic || listen, now);
        self.person.mic = self.mic.read();
        let sign = sign_of(&self.mic, &self.said, &self.partial, &self.voice, link, now);
        overlay(&mut cv, font, scale, &status, sign.as_ref());
        self.place = pl;
        if let Err(e) = self.pixels.render() {
            eprintln!("lust: the window cannot show the frame: {}", e);
        }
    }
}

struct App {
    title: String,
    font: Font,
    wins: Vec<Win>,
    // the exchange of the first window and the world, which starts once the window is there
    first: Option<Arc<Mutex<Exchange>>>,
    world: Option<Box<dyn FnOnce() -> Result<(), String> + Send>>,
    running: Option<JoinHandle<Result<(), String>>>,
    // why no window could be opened (lust then uses the browser)
    failed: Option<String>,
    pinged: Instant,
    // lust dev: closing the window only puts it into the taskbar (the AI goes on)
    keep: bool,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let Some(ex) = self.first.take() else { return };
        match Win::open(el, &self.title, ex, self.keep) {
            Ok(w) => {
                self.wins.push(w);
                if let Some(world) = self.world.take() {
                    self.running = Some(std::thread::spawn(world));
                }
            }
            Err(e) => {
                self.failed = Some(e);
                el.exit();
            }
        }
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, id: WindowId, ev: WindowEvent) {
        let font = &self.font;
        let Some(w) = self.wins.iter_mut().find(|w| w.window.id() == id) else { return };
        match ev {
            // lust dev: the person closes the window, the AI goes on; its next work shows it again
            WindowEvent::CloseRequested if self.keep => {
                w.mic.stop();
                w.hidden = true;
                w.window.set_visible(false);
            }
            WindowEvent::CloseRequested => {
                w.mic.stop();
                // the program ends when it hears that (src/bin/lust.rs); the window goes at once
                w.ex.lock().unwrap_or_else(|p| p.into_inner()).closed();
                w.ended = true;
                w.window.set_visible(false);
            }
            WindowEvent::Resized(s) => w.resize(s.width, s.height),
            WindowEvent::Focused(false) => w.person.blur(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(name) = key_of(&event) {
                    w.person.key(name, event.state == ElementState::Pressed);
                }
            }
            WindowEvent::CursorMoved { position: PhysicalPosition { x, y }, .. } => w.person.mouse = (x, y, w.person.mouse.2),
            WindowEvent::MouseInput { state, .. } => w.person.button(state == ElementState::Pressed),
            WindowEvent::RedrawRequested if !w.ended && !w.hidden => w.redraw(font),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // the world has ended (it asked to quit, the window was closed, an error): so has the window
        if self.running.as_ref().is_some_and(|t| t.is_finished()) {
            el.exit();
            return;
        }
        let now = Instant::now();
        if now.duration_since(self.pinged) >= PING_EVERY {
            self.pinged = now;
            for w in &self.wins {
                w.ex.lock().unwrap_or_else(|p| p.into_inner()).ping();
            }
        }
        // lust dev: the AI asked for the window (and the person may have closed it before)
        if self.keep {
            for w in self.wins.iter_mut().filter(|w| !w.ended) {
                let asked = w.ex.lock().unwrap_or_else(|p| p.into_inner()).show;
                if asked != w.shown_seq {
                    w.shown_seq = asked;
                    w.hidden = false;
                    w.window.set_visible(true);
                    w.window.set_minimized(false);
                    w.window.focus_window();
                }
            }
        }
        for w in self.wins.iter_mut().filter(|w| !w.ended && !w.hidden) {
            let seq = w.ex.lock().unwrap_or_else(|p| p.into_inner()).seq;
            if seq != w.seq || now.duration_since(w.asked) >= DRAW_AT_LEAST {
                w.seq = seq;
                w.asked = now;
                w.window.request_redraw();
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(now + LOOK_EVERY));
    }
}

// opens a window on the main thread and runs the world (a function that runs the loop of
// lust::bin with this exchange) on a thread of its own, until the world ends. keep: closing
// the window only puts it into the taskbar (lust dev). Err when no
// window can be opened (no video card, no screen): then the world never started, and the
// caller may use the browser. Ok holds what the world ended with
pub fn run(title: &str, keep: bool, ex: Arc<Mutex<Exchange>>, world: Box<dyn FnOnce() -> Result<(), String> + Send>) -> Result<Result<(), String>, String> {
    let el = EventLoop::new().map_err(|e| format!("no_window: {}", e))?;
    let mut app = App { title: title.to_string(), font: Font::builtin(), wins: Vec::new(), first: Some(ex), world: Some(world), running: None, failed: None, pinged: Instant::now(), keep };
    el.run_app(&mut app).map_err(|e| format!("no_window: {}", e))?;
    if let Some(e) = app.failed {
        return Err(e);
    }
    match app.running.take() {
        Some(t) => Ok(t.join().unwrap_or_else(|_| Err(String::from("the world stopped with a panic")))),
        None => Err(String::from("no_window: the window never opened")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_named_as_the_page_names_them() {
        assert_eq!(key_name(Some("ArrowUp"), Some("ArrowUp"), None).as_deref(), Some("up"));
        assert_eq!(key_name(Some("Space"), Some("Space"), Some(" ")).as_deref(), Some("space"));
        assert_eq!(key_name(Some("Control"), Some("ControlLeft"), None).as_deref(), Some("ctrl"));
        // the key of r in a russian layout types a cyrillic letter: it is still r
        assert_eq!(key_name(None, Some("KeyR"), Some("\u{43a}")).as_deref(), Some("r"));
        assert_eq!(key_name(None, Some("Digit7"), Some("7")).as_deref(), Some("7"));
        assert_eq!(key_name(Some("F5"), Some("F5"), None).as_deref(), Some("f5"));
        assert_eq!(key_name(Some("PageUp"), Some("PageUp"), None).as_deref(), Some("pageup"));
        assert_eq!(key_name(Some("Super"), Some("SuperLeft"), None).as_deref(), Some("meta"));
        assert_eq!(key_name(None, Some("Numpad1"), Some("1")).as_deref(), Some("1"));
    }

    #[test]
    fn a_window_trades_with_the_world_as_the_page_does() {
        let ex = Mutex::new(Exchange::default());
        ex.lock().unwrap().put_frame(String::from("size 100 50\nclear #000\n"), "");
        let mut p = Person::default();
        let pl = draw::place(200, 100, 100.0, 50.0);
        p.key(String::from("up"), true);
        p.mouse = (20.0, 10.0, false);
        p.button(true);
        let (f, status, quit) = trade(&ex, &mut p, &pl, 0.6);
        assert_eq!((f.as_str(), status.as_str(), quit), ("size 100 50\nclear #000\n", "", false));
        let i = ex.lock().unwrap().take_input();
        assert!(i.down.contains("up") && i.pressed.contains("up") && i.pressed.contains("mouse"));
        // the window is twice the frame: the mouse in the units of the frame
        assert_eq!(i.mouse, (10.0, 5.0, true));
        assert_eq!(i.glyph, 0.6);
        // a key held is not pressed again
        trade(&ex, &mut p, &pl, 0.6);
        assert!(ex.lock().unwrap().take_input().pressed.is_empty());
        // an event made before the window was there is not for it; quit after is, once
        ex.lock().unwrap().push_event("quit");
        assert!(!trade(&ex, &mut Person::default(), &pl, 0.6).2);
        let n = ex.lock().unwrap().push_event("quit");
        assert!(trade(&ex, &mut p, &pl, 0.6).2);
        assert_eq!(ex.lock().unwrap().acked, n, "the program knows at once that the window has it");
        assert!(!trade(&ex, &mut p, &pl, 0.6).2);
        // a window that lost the focus holds no key
        p.blur();
        trade(&ex, &mut p, &pl, 0.6);
        assert!(ex.lock().unwrap().take_input().down.is_empty());
    }
}
