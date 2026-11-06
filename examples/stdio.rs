// an MCP server over stdio: an AI client (Claude Desktop, Claude Code, a client of a local
// model) starts this program and talks to it, one JSON message per line on stdin and
// stdout. the AI sees the operations of its role and proposes; the PERSON decides, through
// a channel the AI does not have: a text file of decisions, one line each, "yes 3" or "no 3".
// stdout belongs to the protocol only; what the person should read goes to stderr.
// keep the decisions file where the AI cannot write: an AI client that also has tools to
// write files (Claude Code has) could otherwise decide for the person. a real program
// gives the person a window, a console or a page of its own instead.
//   cargo build --release --example stdio
// in the client's settings, for example Claude Desktop (claude_desktop_config.json):
//   "mcpServers": { "kiosk": { "command": "C:\\path\\to\\lust\\target\\release\\examples\\stdio.exe",
//                              "args": ["C:\\path\\to\\kiosk-decisions.txt"] } }
// without an argument the decisions file is kiosk-decisions.txt in the current folder.
use lust::host::Host;
use lust::mcp::{handle_text, Proposals, Status, WorldTools};
use std::io::{BufRead, Write};

fn kiosk() -> Result<Host, String> {
    let host = Host::new().map_err(|e| e.to_string())?;
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in [
            ("roles", "kind", "names"), ("roles", "write", "dev"),
            ("params", "kind", "text"), ("params", "write", "dev"),
            ("src", "kind", "code"), ("src", "write", "dev"),
            ("stock", "kind", "int"), ("stock", "write", "dev clerk"),
            ("role.clerk", "desc", "sells over the counter"),
            ("op.sell", "desc", "sells n pieces of an item: tea or coffee"),
            ("op.sell", "roles", "clerk"),
            ("op.sell", "params", "item:id n:int"),
            ("op.sell", "src", "return function(p)
                local left = (ecs.get(p.item, 'stock') or 0) - p.n
                if left < 0 then error('not_enough: ' .. p.item, 0) end
                ecs.set(p.item, 'stock', left)
            end"),
            ("tea", "stock", "5"),
            ("coffee", "stock", "2"),
        ] {
            w.set(e, c, v)?;
        }
    }
    Ok(host)
}

// the person's decisions: lines "yes <id>" or "no <id>"; each proposal is decided once
fn apply_decisions(host: &Host, q: &mut Proposals, file: &str) {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (word, id) = (parts.next().unwrap_or(""), parts.next().and_then(|n| n.parse::<u64>().ok()));
        let pending = id.map_or(false, |i| q.get(i).map_or(false, |p| p.status == Status::Pending));
        if pending && word == "yes" {
            let r = q.confirm(host, id.unwrap_or(0));
            eprintln!("kiosk: proposal {} confirmed: {:?}", id.unwrap_or(0), r);
        } else if pending && word == "no" {
            let _ = q.reject(id.unwrap_or(0));
            eprintln!("kiosk: proposal {} rejected", id.unwrap_or(0));
        }
    }
}

fn main() -> Result<(), String> {
    let file = std::env::args().nth(1).unwrap_or_else(|| String::from("kiosk-decisions.txt"));
    let host = kiosk()?;
    let mut q = Proposals::new();
    eprintln!("kiosk: MCP over stdio. decide proposals in {} with lines \"yes <id>\" or \"no <id>\"", file);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if !line.trim().is_empty() {
            // what the person decided since the last message counts before the AI asks again
            apply_decisions(&host, &mut q, &file);
            let waiting = q.pending().count();
            let answer = {
                let mut door = WorldTools { host: &host, role: "clerk", proposals: &mut q };
                handle_text(&mut door, &line)
            };
            if let Some(a) = answer {
                writeln!(out, "{}", a).map_err(|e| e.to_string())?;
                out.flush().map_err(|e| e.to_string())?;
            }
            for p in q.pending().skip(waiting) {
                eprintln!("kiosk: proposal {}: {} {:?} ({}). write \"yes {}\" or \"no {}\" into {}", p.id, p.op, p.args, p.why, p.id, p.id, file);
            }
        }
    }
    Ok(())
}
