// lust: the program that runs a project folder (lust::project). the program you build is
// the lua in that folder; this file never changes for it.
//   lust                     a double click: the games next to lust (the folders with a
//                            world.lua, in the folder games or beside lust) as a list of
//                            buttons in the browser; with a world.lua beside lust it runs
//                            that folder. F1 in a game comes back to the list
//   lust <folder>            runs that folder (drop a folder on lust.exe)
//   lust run <folder> [--port 8730] [--fps 60] [--no-open]
//   lust dev <folder> [--port 8730] [--fps 60] [--open] [--no-web]
//       an MCP server over stdio for an AI that builds in that folder (lust::dev), and the
//       live world in the browser at http://127.0.0.1:<port>
//   lust pack <folder>       the game of the folder as ONE file (folder/dist/<name>): a double
//                            click on it opens that game on its own menu (an AI does it with lust_pack)
//   lust test <folder>       the tests of the folder (test/*.lua); exit code 1 if one fails
// an MCP client (Claude Desktop, Claude Code) starts dev, for example:
//   "lust": { "command": "C:\\tools\\lust.exe", "args": ["dev", "D:\\games\\pong"] }
// the folder is the permission: the AI writes lua files there and nowhere else.
// stdout belongs to the protocol only (dev); what the person should read goes to stderr
use lust::clock::Clock;
use lust::dev::DevTools;
use lust::driver::run_tests;
use lust::launcher::{find_games_in, Launcher};
use lust::mcp::handle_text;
use lust::project::{Project, Session};
use lust::screen::Exchange;
use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: lust                      (the games next to lust, or the folder with a world.lua beside it)\n       lust <folder>             (run that folder)\n       lust run  <folder> [--port 8730] [--fps 60] [--no-open]\n       lust dev  <folder> [--port 8730] [--fps 60] [--open] [--no-web]\n       lust pack <folder>        (the game as one file to double click)\n       lust test <folder>";
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

struct Args {
    cmd: String,
    folder: Option<PathBuf>,
    port: u16,
    fps: f64,
    web: bool,
    open: bool,
}

fn args() -> Result<Args, String> {
    let list: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match list.first().map(|s| s.as_str()) {
        None => (String::from("start"), 0),
        Some("dev") | Some("run") | Some("test") | Some("pack") => (list[0].clone(), 1),
        Some(x) if x.starts_with("--") => (String::from("start"), 0),
        Some(x) if Path::new(x).is_dir() => (String::from("run"), 0),
        Some(x) => return Err(format!("no folder or command {}\n{}", x, USAGE)),
    };
    let folder = list.get(rest).filter(|s| !s.starts_with("--")).map(PathBuf::from);
    if cmd != "start" && folder.is_none() {
        return Err(USAGE.to_string());
    }
    let mut a = Args { cmd: cmd.clone(), folder, port: 8730, fps: 60.0, web: true, open: cmd != "dev" };
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
            "--no-web" => a.web = false,
            "--open" => a.open = true,
            "--no-open" => a.open = false,
            other => return Err(format!("unknown option {}\n{}", other, USAGE)),
        }
        i += 1;
    }
    Ok(a)
}

// the lines of stdin, read by a thread of their own: the loop must not wait for them
fn stdin_lines() -> Receiver<String> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });
    rx
}

fn answer(s: &mut Session, line: &str) -> Result<(), String> {
    if line.trim().is_empty() {
        return Ok(());
    }
    let a = {
        let mut tools = DevTools { s };
        handle_text(&mut tools, line)
    };
    if let Some(text) = a {
        let mut out = std::io::stdout();
        writeln!(out, "{}", text).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
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

fn make_menu(dirs: &[PathBuf]) -> Result<Launcher, String> {
    let (games, at) = find_games_in(dirs);
    if let Some(d) = &at {
        eprintln!("lust: the games are in {}", d.display());
    }
    let looked: Vec<String> = dirs.iter().take(2).map(|d| d.display().to_string().replace('\\', "/")).collect();
    let hint = format!("No games found. Put a game folder (with a world.lua inside) into the folder games next to lust. Looked in {}.", looked.join(" and "));
    Launcher::new(games, &hint)
}

// what the page shows now: the list of games, or one game
enum Show {
    Menu(Launcher),
    Game(Box<Session>),
}

// the loop: messages of the AI between the frames, a frame every 1/fps s, the files on disk
// every SYNC_EVERY. dev ends when stdin closes (the client went away)
fn live(a: &Args) -> Result<(), String> {
    let dev = a.cmd == "dev";
    let dirs = menu_dirs();
    // a folder with a world.lua beside lust is the game; else the list of games
    // a game glued to this very file (lust::pack) wins: a double click opens that game
    let packed = if a.folder.is_none() { lust::pack::embedded()? } else { None };
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).filter(|d| d.join("world.lua").is_file());
    let (mut show, can_return) = match (&a.folder, packed.as_ref().or(beside.as_ref())) {
        (Some(f), _) => (Show::Game(Box::new(Session::open(f)?)), false),
        (None, Some(b)) => (Show::Game(Box::new(Session::open(b)?)), false),
        (None, None) => (Show::Menu(make_menu(&dirs)?), true),
    };
    if let Show::Game(s) = &show {
        for p in &s.problems {
            eprintln!("lust: {}", p);
        }
    }
    let ex = Arc::new(Mutex::new(Exchange::default()));
    if a.web {
        let addr = serve_near(a.port, &ex)?;
        let url = format!("http://{}", addr);
        eprintln!("lust: the page is at {}", url);
        if !dev {
            eprintln!("lust: it opens in your browser. close this window to stop.");
        }
        if let Show::Game(s) = &mut show {
            s.url = Some(url.clone());
        }
        if a.open {
            open_browser(&url);
        }
        // a packed game has no use for the black window: the page says all there is
        if packed.is_some() && !dev {
            hide_console();
        }
    }
    let rx = if dev { Some(stdin_lines()) } else { None };
    let frame = Duration::from_secs_f64(1.0 / a.fps);
    let mut clock = Clock::variable(0.1);
    let (mut last, mut next, mut synced) = (Instant::now(), Instant::now() + frame, Instant::now());
    // the number of the event quit and when it was made, once the game asked to end
    let mut quit: Option<(u64, Instant)> = None;
    loop {
        let wait = next.saturating_duration_since(Instant::now());
        match &rx {
            Some(r) => match r.recv_timeout(wait) {
                Ok(line) => {
                    if let Show::Game(s) = &mut show {
                        answer(s, &line)?;
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
        // the game asked to end: when the page has heard it, or a little later anyway
        if let Some((n, at)) = quit {
            let acked = ex.lock().unwrap_or_else(|p| p.into_inner()).acked;
            if !a.web || acked >= n || now.duration_since(at) > QUIT_GRACE {
                eprintln!("lust: the game ended");
                return Ok(());
            }
        }
        // run and a double click end with the page; dev ends with its AI client
        if a.web && !dev {
            if bye.is_some_and(|b| now.duration_since(b) > BYE_GRACE) {
                eprintln!("lust: the page was closed");
                return Ok(());
            }
            if alive.is_some_and(|t| now.duration_since(t) > SILENT) {
                eprintln!("lust: no page for {} s", SILENT.as_secs());
                return Ok(());
            }
        }
        // nobody looks: the game waits (in dev the AI may look through its tools)
        let watched = !a.web || dev || seen.is_some_and(|t| now.duration_since(t) < WATCH);
        if n > 0 && watched {
            let input = ex.lock().unwrap_or_else(|p| p.into_inner()).take_input();
            // F1 in a game that was started from the list: back to the list
            if can_return && matches!(show, Show::Game(_)) && input.pressed.contains("f1") {
                match make_menu(&dirs) {
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
                    Ok(s) => {
                        eprintln!("lust: running {}", path.display());
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
        "dev" | "run" | "start" => live(&a).map(|_| true),
        "test" => test(&a),
        "pack" => pack(&a).map(|_| true),
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
