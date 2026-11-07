// lust: the program that runs a project folder (lust::project). the program you build is
// the lua in that folder; this file never changes for it.
//   lust dev <folder> [--port 8730] [--fps 60] [--no-web]
//       an MCP server over stdio for an AI that builds in that folder (lust::dev), and the
//       live world in the browser at http://127.0.0.1:<port>
//   lust run <folder> [--port 8730] [--fps 60]
//       the world in the browser; files changed on disk are taken while it runs
//   lust test <folder>
//       the tests of the folder (test/*.lua); the exit code is 1 when one fails
// an MCP client (Claude Desktop, Claude Code) starts it, for example:
//   "lust": { "command": "C:\\tools\\lust.exe", "args": ["dev", "D:\\games\\pong"] }
// the folder is the permission: the AI writes lua files there and nowhere else.
// stdout belongs to the protocol only; what the person should read goes to stderr
use lust::clock::Clock;
use lust::dev::DevTools;
use lust::driver::run_tests;
use lust::mcp::handle_text;
use lust::project::{Project, Session};
use lust::screen::Exchange;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: lust dev <folder> [--port 8730] [--fps 60] [--no-web]\n       lust run <folder> [--port 8730] [--fps 60]\n       lust test <folder>";
// how often the files on disk are compared with the live world
const SYNC_EVERY: Duration = Duration::from_millis(500);

struct Args {
    cmd: String,
    folder: PathBuf,
    port: u16,
    fps: f64,
    web: bool,
}

fn args() -> Result<Args, String> {
    let list: Vec<String> = std::env::args().skip(1).collect();
    let cmd = list.first().cloned().ok_or(USAGE)?;
    let folder = list.get(1).map(PathBuf::from).ok_or(USAGE)?;
    let mut a = Args { cmd, folder, port: 8730, fps: 60.0, web: true };
    let mut i = 2;
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

// the loop: messages of the AI between the frames, a frame every 1/fps s, the files on disk
// every SYNC_EVERY. dev ends when stdin closes (the client went away)
fn live(a: &Args, dev: bool) -> Result<(), String> {
    let mut s = Session::open(&a.folder)?;
    for p in &s.problems {
        eprintln!("lust: {}", p);
    }
    let ex = Arc::new(Mutex::new(Exchange::default()));
    if a.web {
        let addr = lust::web::serve(a.port, Arc::clone(&ex))?;
        let url = format!("http://{}", addr);
        eprintln!("lust: the world of {} is at {}", a.folder.display(), url);
        s.url = Some(url);
    }
    let rx = if dev { Some(stdin_lines()) } else { None };
    let frame = Duration::from_secs_f64(1.0 / a.fps);
    let mut clock = Clock::variable(0.1);
    let (mut last, mut next, mut synced) = (Instant::now(), Instant::now() + frame, Instant::now());
    loop {
        let wait = next.saturating_duration_since(Instant::now());
        match &rx {
            Some(r) => match r.recv_timeout(wait) {
                Ok(line) => {
                    answer(&mut s, &line)?;
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
        if n > 0 {
            let input = ex.lock().unwrap_or_else(|p| p.into_inner()).take_input();
            let f = s.frame(dt, input);
            let status = s.paused.clone().unwrap_or_default();
            ex.lock().unwrap_or_else(|p| p.into_inner()).put_frame(f, &status);
        }
        if now - synced >= SYNC_EVERY {
            synced = now;
            for l in s.sync() {
                eprintln!("lust: {}", l);
            }
        }
        next += frame;
        if next < now {
            next = now + frame;
        }
    }
}

fn test(a: &Args) -> Result<bool, String> {
    let p = Project::open(&a.folder)?;
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
        "dev" => live(&a, true).map(|_| true),
        "run" => live(&a, false).map(|_| true),
        "test" => test(&a),
        _ => Err(String::from(USAGE)),
    });
    match r {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("lust: {}", e);
            std::process::exit(2);
        }
    }
}
