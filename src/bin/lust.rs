// lust: the program that runs a project folder (lust::project). the program you build is
// the lua in that folder; this file never changes for it.
//   lust                     a double click: the games next to lust (the folders with a
//                            world.lua, in the folder games or beside lust) as a list of
//                            buttons in a window; with a world.lua beside lust it runs
//                            that folder. F1 in a game comes back to the list
//   lust <folder>            runs that folder (drop a folder on lust.exe)
//   lust run <folder> [--web] [--port 8730] [--fps 60] [--no-open]
//       a window of its own (feature native); --web, or a computer where no window opens,
//       shows the game in the browser instead
//   lust dev [<folder>] [--web] [--port 8730] [--fps 60] [--open] [--no-web]
//       an MCP server over stdio for an AI that builds games (lust::dev), and the live world
//       in a window (--web: in the browser at http://127.0.0.1:<port>; --no-web: nowhere,
//       the AI alone). without a folder the games are the folder games beside lust and the
//       AI opens the one the person names (lust_open); with the folder of a game it starts
//       there. the microphone stays on: the AI may wait for the words of the person
//       (lust_listen). closing the window only puts it into the taskbar. a second lust dev
//       (another AI client) shows no window of its own: it passes its messages to the first
//       (the hub, 127.0.0.1:8729), so there is one window, one microphone, one world
//   lust pack <folder>       the game of the folder as ONE file (folder/dist/<name>): a double
//                            click on it opens that game on its own menu (an AI does it with lust_pack)
//   lust test <folder>       the tests of the folder (test/*.lua); exit code 1 if one fails
//   lust hear <file.wav>     the words vosk hears in a sound file (a check of the voice)
// an MCP client (Claude Desktop, Claude Code) starts dev, for example:
//   "lust": { "command": "C:\\tools\\lust.exe", "args": ["dev"] }
// the folder of games is the permission: the AI writes lua files there and nowhere else.
// stdout belongs to the protocol only (dev); what the person should read goes to stderr
use lust::clock::Clock;
use lust::dev::{desk_tools, DevTools, GUIDE};
use lust::driver::run_tests;
use lust::launcher::{find_games, find_games_in, Launcher};
use lust::mcp::{handle_text, late_answer, Tool, Tools};
use lust::project::{Project, Session};
use lust::screen::{Exchange, Link};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: lust                      (the games next to lust, or the folder with a world.lua beside it)\n       lust <folder>             (run that folder)\n       lust run  <folder> [--web] [--port 8730] [--fps 60] [--no-open]\n       lust dev  [<folder>] [--web] [--port 8730] [--fps 60] [--open] [--no-web]\n       lust pack <folder>        (the game as one file to double click)\n       lust test <folder>\n       lust hear <file.wav>      (the words the voice recognizer hears in it)";
// how often the files on disk are compared with the live world
const SYNC_EVERY: Duration = Duration::from_millis(500);
// the world pauses when no page asked for a frame this long (the tab is hidden or closed)
const WATCH: Duration = Duration::from_secs(1);
// lust ends this long after the page said it was closed (a reload comes back sooner)
const BYE_GRACE: Duration = Duration::from_secs(3);
// lust ends when no page said anything this long (a hidden tab pings at least every minute)
const SILENT: Duration = Duration::from_secs(180);
// lust ends this long after the game asked to quit, even if no page heard it
const QUIT_GRACE: Duration = Duration::from_secs(1);
// how many ports after the asked one are tried when it is taken
const PORTS_TRIED: u16 = 20;
// the hub of lust dev: the first one listens here, a second one passes its messages through
const HUB_PORT: u16 = 8729;
const HUB_HELLO: &str = "lust-hub 1";
const HUB_OK: &str = "lust-hub ok";
// lust_listen waits this long by default, and at most (a client gives up on a call after a minute)
const LISTEN_DEFAULT: u64 = 45;
const LISTEN_MAX: u64 = 55;
// what the person said while no AI listened waits this long for the next lust_listen
const HEARD_KEPT: Duration = Duration::from_secs(90);
// a new game made by lust_open
const EMPTY_WORLD: &str = "return { types = {}, entities = {} }\n";

#[derive(Clone)]
struct Args {
    cmd: String,
    folder: Option<PathBuf>,
    port: u16,
    fps: f64,
    web: bool,
    open: bool,
    // a window of its own instead of the page (run, dev and a double click; --web: the page)
    window: bool,
}

fn args() -> Result<Args, String> {
    let list: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match list.first().map(|s| s.as_str()) {
        None => (String::from("start"), 0),
        Some("dev") | Some("run") | Some("test") | Some("pack") | Some("hear") => (list[0].clone(), 1),
        Some(x) if x.starts_with("--") => (String::from("start"), 0),
        Some(x) if Path::new(x).is_dir() => (String::from("run"), 0),
        Some(x) => return Err(format!("no folder or command {}\n{}", x, USAGE)),
    };
    let folder = list.get(rest).filter(|s| !s.starts_with("--")).map(PathBuf::from);
    if cmd != "start" && cmd != "dev" && folder.is_none() {
        return Err(USAGE.to_string());
    }
    let mut a = Args { cmd: cmd.clone(), folder, port: 8730, fps: 60.0, web: true, open: cmd != "dev", window: cfg!(feature = "native") };
    let mut i = if a.folder.is_some() { rest + 1 } else { rest };
    while i < list.len() {
        let next = list.get(i + 1);
        match list[i].as_str() {
            "--port" => {
                a.port = next.and_then(|x| x.parse().ok()).ok_or("--port needs a number")?;
                i += 1;
            }
            "--fps" => {
                a.fps = next.and_then(|x| x.parse::<f64>().ok()).filter(|f| *f >= 1.0 && *f <= 1000.0).ok_or("--fps needs 1..1000")?;
                i += 1;
            }
            "--no-web" => {
                a.web = false;
                a.window = false;
            }
            "--web" => a.window = false,
            "--open" => a.open = true,
            "--no-open" => a.open = false,
            other => return Err(format!("unknown option {}\n{}", other, USAGE)),
        }
        i += 1;
    }
    // the window shows the game, the page is not needed (--no-web: neither of them)
    if a.window {
        a.web = false;
    }
    Ok(a)
}

// ---------- the clients of lust dev ----------

// a message of a client of lust dev (0 is stdin; the others came through the hub)
enum Msg {
    Hello(usize),
    Line(usize, String),
    Gone(usize),
}

// where the answers to each client go
type Outs = Arc<Mutex<BTreeMap<usize, Box<dyn Write + Send>>>>;

// the lines of stdin, read by a thread of their own: the loop must not wait for them
fn stdin_lines(tx: Sender<Msg>) {
    thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => {
                    if tx.send(Msg::Line(0, l)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(Msg::Gone(0));
    });
}

fn send_to(outs: &Outs, client: usize, text: &str) {
    let mut o = outs.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(w) = o.get_mut(&client) {
        let _ = writeln!(w, "{}", text).and_then(|_| w.flush());
    }
}

fn hub_addr() -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, HUB_PORT))
}

// one client of the hub: it says hello, then each line is a message of MCP
fn hub_client(s: TcpStream, id: usize, tx: Sender<Msg>, outs: Outs) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let Ok(r) = s.try_clone() else { return };
    let mut r = BufReader::new(r);
    let mut first = String::new();
    if r.read_line(&mut first).is_err() || first.trim() != HUB_HELLO {
        return;
    }
    let _ = s.set_read_timeout(None);
    let mut w = s;
    if writeln!(w, "{}", HUB_OK).and_then(|_| w.flush()).is_err() {
        return;
    }
    outs.lock().unwrap_or_else(|p| p.into_inner()).insert(id, Box::new(w));
    if tx.send(Msg::Hello(id)).is_err() {
        return;
    }
    for line in r.lines() {
        match line {
            Ok(l) => {
                if tx.send(Msg::Line(id, l)).is_err() {
                    return;
                }
            }
            Err(_) => break,
        }
    }
    let _ = tx.send(Msg::Gone(id));
}

// the first lust dev: takes the clients that come through the hub
fn hub_serve(l: TcpListener, tx: Sender<Msg>, outs: Outs) {
    thread::spawn(move || {
        let mut next = 1;
        for s in l.incoming() {
            let Ok(s) = s else { continue };
            let (id, tx, outs) = (next, tx.clone(), Arc::clone(&outs));
            next += 1;
            thread::spawn(move || hub_client(s, id, tx, outs));
        }
    });
}

// a second lust dev finds the first; None when there is none (or the port is something else)
fn hub_join() -> Option<BufReader<TcpStream>> {
    let s = TcpStream::connect_timeout(&hub_addr(), Duration::from_secs(1)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    let mut w = s.try_clone().ok()?;
    writeln!(w, "{}", HUB_HELLO).and_then(|_| w.flush()).ok()?;
    let mut r = BufReader::new(s);
    let mut ok = String::new();
    r.read_line(&mut ok).ok()?;
    if ok.trim() != HUB_OK {
        return None;
    }
    r.get_ref().set_read_timeout(None).ok()?;
    Some(r)
}

// a second lust dev passes stdin to the first and its answers to stdout, until stdin ends
// (its client went away) or the first ends (then this one ends too, and the client sees it)
fn proxy(r: BufReader<TcpStream>) -> Result<(), String> {
    eprintln!("lust: another lust dev has the window; this one passes its messages to it");
    let mut w = r.get_ref().try_clone().map_err(|e| e.to_string())?;
    thread::spawn(move || {
        let mut out = std::io::stdout();
        for line in r.lines() {
            match line {
                Ok(l) => {
                    if writeln!(out, "{}", l).and_then(|_| out.flush()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        eprintln!("lust: the first lust dev ended");
        std::process::exit(0);
    });
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let l = line.map_err(|e| e.to_string())?;
        writeln!(w, "{}", l).and_then(|_| w.flush()).map_err(|e| e.to_string())?;
    }
    let _ = w.shutdown(std::net::Shutdown::Both);
    Ok(())
}

// ---------- the desk of lust dev: the tools over the open game ----------

// a name of a game the AI may make: a folder in games
fn game_name_ok(n: &str) -> bool {
    !n.is_empty() && n.len() <= 40 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

struct Desk<'a> {
    show: &'a mut Show,
    root: &'a Path,
    url: Option<String>,
}

impl Desk<'_> {
    fn open(&mut self, args: &Value) -> Result<String, String> {
        let name = args.get("name").and_then(|v| v.as_str()).map(|n| n.trim()).unwrap_or("");
        if name.is_empty() {
            let names: Vec<String> = find_games(self.root).into_iter().map(|g| g.name).collect();
            return Ok(if names.is_empty() {
                format!("no games yet in {}. lust_open(name) makes one", self.root.display())
            } else {
                format!("the games in {}: {}\nlust_open(name) opens one; a new name makes one", self.root.display(), names.join(", "))
            });
        }
        if !game_name_ok(name) {
            return Err(String::from("bad_name: 1 to 40 of a-z A-Z 0-9 _ -"));
        }
        let dir = self.root.join(name);
        let made = !dir.exists();
        if made {
            std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {}", dir.display(), e))?;
            std::fs::write(dir.join("world.lua"), EMPTY_WORLD).map_err(|e| format!("cannot make {}: {}", dir.display(), e))?;
        } else if !dir.is_dir() {
            return Err(format!("{} is a file, not a game", dir.display()));
        }
        let mut s = Session::open(&dir)?;
        s.url = self.url.clone();
        let mut out = format!("{} {} ({})", if made { "made and opened" } else { "opened" }, name, dir.display());
        for p in &s.problems {
            out.push('\n');
            out.push_str(p);
        }
        eprintln!("lust: running {}", dir.display());
        *self.show = Show::Game(Box::new(s));
        Ok(out)
    }
}

impl Tools for Desk<'_> {
    fn server(&self) -> (String, String, String) {
        (
            String::from("lust"),
            String::from(env!("CARGO_PKG_VERSION")),
            String::from("You build games for the person, each a folder of lua files. Read lust_guide once. lust_open opens the game the person names; lust_listen hears what the person says."),
        )
    }

    fn tools(&self) -> Vec<Tool> {
        desk_tools()
    }

    fn call(&mut self, name: &str, args: &Value) -> Result<String, String> {
        match name {
            "lust_guide" => Ok(GUIDE.to_string()),
            "lust_open" => self.open(args),
            // the loop answers it when the person has spoken (see Ear)
            "lust_listen" => Err(String::from("no voice here")),
            _ => match self.show {
                Show::Game(s) => DevTools { s }.call(name, args),
                Show::Menu(_) => Err(String::from("no game is open: lust_open(name) first (lust_open() lists the games)")),
            },
        }
    }
}

// ---------- the ear of lust dev: the words of the person for the AI ----------

#[derive(Default)]
struct Ear {
    // the clients that are there, and those that said hello (initialize)
    alive: BTreeSet<usize>,
    hello: BTreeSet<usize>,
    // the calls of lust_listen that wait: client, id of the call, until when
    pending: Vec<(usize, Value, Instant)>,
    // what the person said that no call has had yet
    queue: VecDeque<(String, Instant)>,
}

impl Ear {
    fn link(&self) -> Link {
        if !self.pending.is_empty() {
            Link::Listening
        } else if !self.hello.is_empty() {
            Link::Connected
        } else {
            Link::Waiting
        }
    }

    // one line of a client: lust_listen waits here, the rest goes to the desk
    fn line(&mut self, desk: &mut Desk, outs: &Outs, client: usize, line: &str, voice: Option<String>) {
        if line.trim().is_empty() {
            return;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
            if method == "initialize" {
                self.hello.insert(client);
            }
            if method == "notifications/cancelled" {
                if let Some(rid) = v.get("params").and_then(|p| p.get("requestId")) {
                    self.pending.retain(|(c, id, _)| !(*c == client && id == rid));
                }
            }
            let tool = v.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str());
            if let (Some(id), "tools/call", Some("lust_listen")) = (v.get("id").filter(|i| i.is_string() || i.is_number()), method, tool) {
                match voice {
                    Some(why) => send_to(outs, client, &late_answer(id.clone(), why, true)),
                    None => {
                        let secs = v.get("params").and_then(|p| p.get("arguments")).and_then(|a| a.get("seconds")).and_then(|s| s.as_u64()).unwrap_or(LISTEN_DEFAULT).clamp(5, LISTEN_MAX);
                        self.pending.push((client, id.clone(), Instant::now() + Duration::from_secs(secs)));
                    }
                }
                return;
            }
        }
        if let Some(text) = handle_text(desk, line) {
            send_to(outs, client, &text);
        }
    }

    // what the window heard goes to the calls that wait; a call that waited long enough
    // hears that nothing was said
    fn tick(&mut self, heard: Vec<String>, outs: &Outs, now: Instant) {
        for h in heard {
            eprintln!("lust: heard: {}", h);
            self.queue.push_back((h, now));
        }
        self.queue.retain(|(_, t)| now.duration_since(*t) < HEARD_KEPT);
        if !self.pending.is_empty() && !self.queue.is_empty() {
            let words: Vec<String> = self.queue.drain(..).map(|(h, _)| h).collect();
            let text = format!("the person said:\n{}", words.join("\n"));
            for (c, id, _) in self.pending.drain(..) {
                send_to(outs, c, &late_answer(id, text.clone(), false));
            }
        }
        let (done, wait): (Vec<_>, Vec<_>) = self.pending.drain(..).partition(|(_, _, until)| now >= *until);
        self.pending = wait;
        for (c, id, _) in done {
            send_to(outs, c, &late_answer(id, String::from("nothing was said; call lust_listen again to keep listening"), false));
        }
    }

    fn gone(&mut self, client: usize) {
        self.alive.remove(&client);
        self.hello.remove(&client);
        self.pending.retain(|(c, _, _)| *c != client);
    }
}

// the page in the default browser; nothing happens if there is none
fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("cmd").args(["/C", "start", "", url]).creation_flags(0x0800_0000).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    }
}

// the console window of a double click goes away (nothing else changes)
fn hide_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn FreeConsole() -> i32;
        }
        unsafe {
            FreeConsole();
        }
    }
}

// a frame that draws nothing (a new, empty game) says what to do instead of a black page
fn hint_if_empty(f: String, root: &Path) -> String {
    let drawn = f.lines().any(|l| !l.is_empty() && !l.starts_with("size ") && !l.starts_with("mic "));
    if drawn {
        return f;
    }
    let folder = root.display().to_string();
    format!(
        "size 800 600\nclear #14141f\ntext 40 220 28 #ffffff This game draws nothing yet.\ntext 40 270 18 #8888aa Ask the AI to build something, or add the file draw/main.lua\ntext 40 300 18 #8888aa in {}\n",
        folder.chars().filter(|c| !c.is_control()).take(80).collect::<String>()
    )
}

// the display on the asked port, or on the next free one
fn serve_near(port: u16, ex: &Arc<Mutex<Exchange>>) -> Result<SocketAddr, String> {
    let mut last = String::new();
    for p in port..=port.saturating_add(PORTS_TRIED) {
        match lust::web::serve(p, Arc::clone(ex)) {
            Ok(addr) => return Ok(addr),
            Err(e) => last = e,
        }
        if port == 0 {
            break;
        }
    }
    Err(last)
}

// where the games are looked for when lust is started without a folder
fn menu_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(exe) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        v.push(exe.join("games"));
        v.push(exe);
    }
    if let Ok(cwd) = std::env::current_dir() {
        v.push(cwd.join("games"));
        v.push(cwd.join("examples"));
    }
    v
}

fn make_menu(dirs: &[PathBuf], dev: bool) -> Result<Launcher, String> {
    let (games, at) = find_games_in(dirs);
    if let Some(d) = &at {
        eprintln!("lust: the games are in {}", d.display());
    }
    let looked: Vec<String> = dirs.iter().take(2).map(|d| d.display().to_string().replace('\\', "/")).collect();
    let hint = if dev {
        String::from("No games yet. Tell the AI the name of a new game.")
    } else {
        format!("No games found. Put a game folder (with a world.lua inside) into the folder games next to lust. Looked in {}.", looked.join(" and "))
    };
    Launcher::new(games, &hint)
}

// lust dev: the folder of the games, and what is shown first. a folder of a game opens that
// game (its games are the folder around it); a folder of games, or none (the folder games
// beside lust), shows the list
fn dev_start(folder: &Option<PathBuf>) -> Result<(PathBuf, Show), String> {
    match folder {
        Some(f) if f.join("world.lua").is_file() || find_games(f).is_empty() => {
            let root = f.parent().map(|p| p.to_path_buf()).filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| PathBuf::from("."));
            Ok((root, Show::Game(Box::new(Session::open(f)?))))
        }
        Some(f) => Ok((f.clone(), Show::Menu(make_menu(std::slice::from_ref(f), true)?))),
        None => {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let root = exe.parent().map(|d| d.join("games")).ok_or("no folder of lust")?;
            std::fs::create_dir_all(&root).map_err(|e| format!("cannot make {}: {}", root.display(), e))?;
            Ok((root.clone(), Show::Menu(make_menu(&[root], true)?)))
        }
    }
}

// the name the window shows in its title
fn title_of(show: &Show) -> String {
    match show {
        Show::Menu(_) => String::from("games"),
        Show::Game(s) => s.project.root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
    }
}

// what the page shows now: the list of games, or one game
enum Show {
    Menu(Launcher),
    Game(Box<Session>),
}

// the loop: messages of the AI between the frames, a frame every 1/fps s, the files on disk
// every SYNC_EVERY. dev ends when its last client went away (stdin closed, and no client of
// the hub is left)
fn live(a: &Args, ex: Arc<Mutex<Exchange>>, hub: Option<TcpListener>) -> Result<(), String> {
    let dev = a.cmd == "dev";
    // something shows the game: the page or the window
    let shown = a.web || a.window;
    let dirs = menu_dirs();
    // a folder with a world.lua beside lust is the game; else the list of games
    // a game glued to this very file (lust::pack) wins: a double click opens that game
    let packed = if a.folder.is_none() && !dev { lust::pack::embedded()? } else { None };
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).filter(|d| d.join("world.lua").is_file());
    let (root, mut show, can_return) = if dev {
        let (root, show) = dev_start(&a.folder)?;
        (Some(root), show, true)
    } else {
        match (&a.folder, packed.as_ref().or(beside.as_ref())) {
            (Some(f), _) => (None, Show::Game(Box::new(Session::open(f)?)), false),
            (None, Some(b)) => (None, Show::Game(Box::new(Session::open(b)?)), false),
            (None, None) => (None, Show::Menu(make_menu(&dirs, false)?), true),
        }
    };
    // the list F1 goes back to
    let list: Vec<PathBuf> = match &root {
        Some(r) => vec![r.clone()],
        None => dirs.clone(),
    };
    if let Show::Game(s) = &show {
        for p in &s.problems {
            eprintln!("lust: {}", p);
        }
    }
    if a.window && packed.is_some() && !dev {
        hide_console();
    }
    let mut url = None;
    if a.web {
        let addr = serve_near(a.port, &ex)?;
        let u = format!("http://{}", addr);
        eprintln!("lust: the page is at {}", u);
        if !dev {
            eprintln!("lust: it opens in your browser. close this window to stop.");
        }
        if let Show::Game(s) = &mut show {
            s.url = Some(u.clone());
        }
        if a.open {
            open_browser(&u);
        }
        // a packed game has no use for the black window: the page says all there is
        if packed.is_some() && !dev {
            hide_console();
        }
        url = Some(u);
    }
    // lust dev: the clients (stdin, and those of the hub) and the words of the person
    let outs: Outs = Arc::new(Mutex::new(BTreeMap::new()));
    let mut ear = Ear::default();
    let rx = if dev {
        let (tx, rx) = channel();
        outs.lock().unwrap_or_else(|p| p.into_inner()).insert(0, Box::new(std::io::stdout()));
        ear.alive.insert(0);
        stdin_lines(tx.clone());
        if let Some(l) = hub {
            hub_serve(l, tx, Arc::clone(&outs));
        }
        let mut e = ex.lock().unwrap_or_else(|p| p.into_inner());
        e.link = Link::Waiting;
        e.listen = a.window;
        Some(rx)
    } else {
        None
    };
    let frame = Duration::from_secs_f64(1.0 / a.fps);
    let mut clock = Clock::variable(0.1);
    let (mut last, mut next, mut synced) = (Instant::now(), Instant::now() + frame, Instant::now());
    // the number of the event quit and when it was made, once the game asked to end
    let mut quit: Option<(u64, Instant)> = None;
    loop {
        let wait = next.saturating_duration_since(Instant::now());
        match &rx {
            Some(r) => match r.recv_timeout(wait) {
                Ok(Msg::Line(c, line)) => {
                    // the voice: none without a window, or when the window found no recognizer
                    let voice = if !a.window {
                        Some(String::from("no voice: lust dev shows no window here (--web or --no-web); the voice needs the window"))
                    } else {
                        Some(ex.lock().unwrap_or_else(|p| p.into_inner()).voice.clone()).filter(|v| v.starts_with("no voice"))
                    };
                    let mut desk = Desk { show: &mut show, root: root.as_deref().unwrap_or(Path::new(".")), url: url.clone() };
                    ear.line(&mut desk, &outs, c, &line, voice);
                    continue;
                }
                Ok(Msg::Hello(c)) => {
                    eprintln!("lust: another AI client came through the hub");
                    ear.alive.insert(c);
                    continue;
                }
                Ok(Msg::Gone(c)) => {
                    ear.gone(c);
                    outs.lock().unwrap_or_else(|p| p.into_inner()).remove(&c);
                    if ear.alive.is_empty() {
                        return Ok(());
                    }
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            },
            None => thread::sleep(wait),
        }
        let now = Instant::now();
        let (n, dt) = clock.advance((now - last).as_secs_f64());
        last = now;
        let (seen, alive, bye) = {
            let e = ex.lock().unwrap_or_else(|p| p.into_inner());
            (e.seen, e.alive, e.bye)
        };
        if dev {
            let heard = std::mem::take(&mut ex.lock().unwrap_or_else(|p| p.into_inner()).heard);
            ear.tick(heard, &outs, now);
            let mut e = ex.lock().unwrap_or_else(|p| p.into_inner());
            e.link = ear.link();
            let t = title_of(&show);
            if e.title != t {
                e.title = t;
            }
        }
        // the game asked to end: when the page has heard it, or a little later anyway
        if let Some((n, at)) = quit {
            let acked = ex.lock().unwrap_or_else(|p| p.into_inner()).acked;
            if !shown || acked >= n || now.duration_since(at) > QUIT_GRACE {
                eprintln!("lust: the game ended");
                return Ok(());
            }
        }
        // a closed window ends lust at once (the window of dev is never closed, only put into
        // the taskbar). a closed page ends run and a double click a little later (a reload
        // comes back); dev with the page ends with its AI client only
        if a.window && !dev && bye.is_some() {
            eprintln!("lust: the window was closed");
            return Ok(());
        }
        if a.web && !dev {
            if bye.is_some_and(|b| now.duration_since(b) >= BYE_GRACE) {
                eprintln!("lust: the page was closed");
                return Ok(());
            }
            if alive.is_some_and(|t| now.duration_since(t) > SILENT) {
                eprintln!("lust: no page for {} s", SILENT.as_secs());
                return Ok(());
            }
        }
        // nobody looks: the game waits (in dev the AI may look through its tools)
        let watched = !shown || dev || seen.is_some_and(|t| now.duration_since(t) < WATCH);
        if n > 0 && watched {
            let input = ex.lock().unwrap_or_else(|p| p.into_inner()).take_input();
            // F1 in a game that was started from the list: back to the list
            if can_return && matches!(show, Show::Game(_)) && input.pressed.contains("f1") {
                match make_menu(&list, dev) {
                    Ok(m) => show = Show::Menu(m),
                    Err(e) => eprintln!("lust: {}", e),
                }
            }
            let (f, status) = match &mut show {
                Show::Menu(m) => (m.frame(dt, input), String::new()),
                Show::Game(s) => {
                    let f = hint_if_empty(s.frame(dt, input), &s.project.root);
                    (f, s.paused.clone().or_else(|| s.problems.first().cloned()).unwrap_or_default())
                }
            };
            ex.lock().unwrap_or_else(|p| p.into_inner()).put_frame(f, &status);
            // what the world tells the display once: sounds later, now the end of the game
            let events = match &mut show {
                Show::Menu(m) => m.take_events(),
                Show::Game(s) => s.app.screen.take_events(),
            };
            for e in events {
                if e != "quit" {
                    ex.lock().unwrap_or_else(|p| p.into_inner()).push_event(&e);
                } else if dev {
                    // the AI goes on working: the world waits instead of the program ending
                    if let Show::Game(s) = &mut show {
                        s.paused = Some(String::from("the game asked to quit (ecs.quit); lust_restart starts it again"));
                    }
                } else if quit.is_none() {
                    let n = ex.lock().unwrap_or_else(|p| p.into_inner()).push_event(&e);
                    quit = Some((n, now));
                }
            }
            let picked = match &mut show {
                Show::Menu(m) => m.take_pick().map(|g| g.path.clone()),
                Show::Game(_) => None,
            };
            if let Some(path) = picked {
                match Session::open(&path) {
                    Ok(mut s) => {
                        eprintln!("lust: running {}", path.display());
                        s.url = url.clone();
                        show = Show::Game(Box::new(s));
                    }
                    Err(e) => eprintln!("lust: {}", e),
                }
            }
        }
        if now - synced >= SYNC_EVERY {
            synced = now;
            if let Show::Game(s) = &mut show {
                for l in s.sync() {
                    eprintln!("lust: {}", l);
                }
            }
        }
        next += frame;
        if next < now {
            next = now + frame;
        }
    }
}

// the live program: in a window of its own when it can, else with the page. a second lust
// dev passes its messages to the first one instead (one window, one microphone)
fn start(a: Args) -> Result<(), String> {
    let mut hub = None;
    if a.cmd == "dev" {
        match TcpListener::bind(hub_addr()) {
            Ok(l) => hub = Some(l),
            Err(_) => {
                if let Some(r) = hub_join() {
                    return proxy(r);
                }
                eprintln!("lust: the hub port {} is taken by something else; this lust dev works alone", HUB_PORT);
            }
        }
    }
    #[cfg(feature = "native")]
    if a.window {
        let ex = Arc::new(Mutex::new(Exchange::default()));
        let (b, e) = (a.clone(), Arc::clone(&ex));
        let title = a.folder.as_ref().and_then(|f| f.file_name()).map(|n| format!("lust: {}", n.to_string_lossy())).unwrap_or_else(|| String::from("lust"));
        // the hub goes to whichever loop runs: in the window, or with the page when no window opens
        let shared = Arc::new(Mutex::new(hub.take()));
        let given = Arc::clone(&shared);
        let take = move || given.lock().unwrap_or_else(|p| p.into_inner()).take();
        match lust::native::run(&title, a.cmd == "dev", ex, Box::new(move || live(&b, e, take()))) {
            Ok(r) => return r,
            Err(why) => {
                eprintln!("lust: no window here ({}); the game opens in the browser", why);
                let mut b = a;
                b.window = false;
                b.web = true;
                let h = shared.lock().unwrap_or_else(|p| p.into_inner()).take();
                return live(&b, Arc::new(Mutex::new(Exchange::default())), h);
            }
        }
    }
    live(&a, Arc::new(Mutex::new(Exchange::default())), hub)
}

// lust hear <file.wav>: the words the voice recognizer hears in a sound file
fn hear(a: &Args) -> Result<bool, String> {
    let file = a.folder.clone().ok_or(USAGE)?;
    #[cfg(feature = "native")]
    {
        let phrases = lust::native::voice::hear_file(&file)?;
        for p in &phrases {
            println!("{}", p);
        }
        Ok(!phrases.is_empty())
    }
    #[cfg(not(feature = "native"))]
    {
        Err(format!("{}: this lust is built without the window (feature native), so without the voice", file.display()))
    }
}

// lust pack <folder>: the game of the folder as one file
fn pack(a: &Args) -> Result<(), String> {
    let folder = a.folder.clone().ok_or(USAGE)?;
    let program = std::env::current_exe().map_err(|e| e.to_string())?;
    let (to, size) = lust::pack::pack(&folder, &program)?;
    println!("packed {} ({} KB)", to.display(), size / 1024);
    Ok(())
}

fn test(a: &Args) -> Result<bool, String> {
    let folder = a.folder.clone().ok_or(USAGE)?;
    let p = Project::open(&folder)?;
    let results = run_tests(&p, None)?;
    let mut ok = true;
    for (name, r, ms) in results {
        match r {
            Ok(out) => {
                println!("ok {} ({:.0} ms)", name, ms);
                if !out.is_empty() {
                    println!("{}", out);
                }
            }
            Err(e) => {
                ok = false;
                println!("FAIL {}: {}", name, e);
            }
        }
    }
    Ok(ok)
}

fn main() {
    let r = args().and_then(|a| match a.cmd.as_str() {
        "dev" | "run" | "start" => start(a).map(|_| true),
        "test" => test(&a),
        "pack" => pack(&a).map(|_| true),
        "hear" => hear(&a),
        _ => Err(String::from(USAGE)),
    });
    match r {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("lust: {}", e);
            // a window opened by a double click closes at once: let the person read the error
            #[cfg(windows)]
            {
                eprintln!("press enter to close");
                let mut s = String::new();
                let _ = std::io::stdin().read_line(&mut s);
            }
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // a client of the tests: what lust answers it is kept
    #[derive(Clone, Default)]
    struct Pipe(Arc<Mutex<Vec<u8>>>);

    impl Write for Pipe {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Pipe {
        fn take(&self) -> Vec<Value> {
            let text = String::from_utf8(std::mem::take(&mut *self.0.lock().unwrap())).unwrap();
            text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
        }
    }

    fn said(v: &Value) -> String {
        v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string()
    }

    #[test]
    fn the_ai_waits_for_the_words_of_the_person_and_opens_games_by_name() {
        let root = std::env::temp_dir().join(format!("lust_desk_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut show = Show::Menu(make_menu(std::slice::from_ref(&root), true).unwrap());
        let (one, two) = (Pipe::default(), Pipe::default());
        let outs: Outs = Arc::new(Mutex::new(BTreeMap::new()));
        outs.lock().unwrap().insert(0, Box::new(one.clone()));
        outs.lock().unwrap().insert(1, Box::new(two.clone()));
        let mut ear = Ear::default();
        let mut desk = Desk { show: &mut show, root: &root, url: None };
        let call = |id: u64, name: &str, args: &str| format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"tools/call\",\"params\":{{\"name\":\"{}\",\"arguments\":{}}}}}", id, name, args);
        assert_eq!(ear.link(), Link::Waiting);
        ear.line(&mut desk, &outs, 0, "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}", None);
        assert_eq!(ear.link(), Link::Connected);
        // no game yet: the tools of a game say what to do; a new name makes a game
        ear.line(&mut desk, &outs, 0, &call(2, "lust_files", "{}"), None);
        ear.line(&mut desk, &outs, 0, &call(3, "lust_open", "{\"name\":\"../up\"}"), None);
        ear.line(&mut desk, &outs, 0, &call(4, "lust_open", "{\"name\":\"bomber\"}"), None);
        ear.line(&mut desk, &outs, 0, &call(5, "lust_open", "{}"), None);
        let a = one.take();
        assert!(said(&a[1]).contains("lust_open"), "{:?}", a[1]);
        assert!(said(&a[2]).starts_with("bad_name"));
        assert!(said(&a[3]).starts_with("made and opened bomber"));
        assert!(said(&a[4]).contains("bomber"));
        assert!(root.join("bomber/world.lua").is_file() && !root.join("up").exists());
        ear.line(&mut desk, &outs, 0, &call(6, "lust_files", "{}"), None);
        assert!(said(&one.take()[0]).contains("world.lua 1"));
        // the voice: an error at once when there is none
        ear.line(&mut desk, &outs, 0, &call(7, "lust_listen", "{}"), Some(String::from("no voice: test")));
        let a = one.take();
        assert_eq!((said(&a[0]).as_str(), a[0]["result"]["isError"].as_bool()), ("no voice: test", Some(true)));
        // a call waits; what the person says goes to it, and only to the client that asked
        let t = Instant::now();
        ear.line(&mut desk, &outs, 1, &call(8, "lust_listen", "{\"seconds\":10}"), None);
        assert_eq!(ear.link(), Link::Listening);
        ear.tick(Vec::new(), &outs, t);
        assert!(two.take().is_empty(), "nothing said yet: the call waits");
        ear.tick(vec![String::from("make the walls grey")], &outs, t + Duration::from_secs(1));
        let b = two.take();
        assert_eq!((b[0]["id"].as_u64(), said(&b[0]).as_str()), (Some(8), "the person said:\nmake the walls grey"));
        assert!(one.take().is_empty());
        assert_eq!(ear.link(), Link::Connected);
        // said while nobody listens: it waits for the next call (a while)
        ear.tick(vec![String::from("add a player")], &outs, t + Duration::from_secs(2));
        ear.line(&mut desk, &outs, 0, &call(9, "lust_listen", "{}"), None);
        ear.tick(Vec::new(), &outs, t + Duration::from_secs(3));
        assert_eq!(said(&one.take()[0]), "the person said:\nadd a player");
        // a call that waited long enough hears that nothing was said
        ear.line(&mut desk, &outs, 0, &call(10, "lust_listen", "{\"seconds\":5}"), None);
        ear.tick(Vec::new(), &outs, Instant::now() + Duration::from_secs(6));
        assert!(said(&one.take()[0]).starts_with("nothing was said"));
        // a cancelled call and a client that went away are forgotten
        ear.line(&mut desk, &outs, 0, &call(11, "lust_listen", "{}"), None);
        ear.line(&mut desk, &outs, 0, "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":11}}", None);
        ear.line(&mut desk, &outs, 1, &call(12, "lust_listen", "{}"), None);
        ear.alive.insert(1);
        ear.gone(1);
        assert!(ear.pending.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_game_folder_or_a_folder_of_games() {
        let root = std::env::temp_dir().join(format!("lust_start_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pong")).unwrap();
        std::fs::write(root.join("pong/world.lua"), EMPTY_WORLD).unwrap();
        let (r, show) = dev_start(&Some(root.join("pong"))).unwrap();
        assert!(r == root && matches!(show, Show::Game(_)));
        assert_eq!(title_of(&show), "pong");
        let (r, show) = dev_start(&Some(root.clone())).unwrap();
        assert!(r == root && matches!(show, Show::Menu(_)));
        assert!(game_name_ok("bomber_man-2") && !game_name_ok("") && !game_name_ok("a/b") && !game_name_ok(".."));
        let _ = std::fs::remove_dir_all(&root);
    }
}

