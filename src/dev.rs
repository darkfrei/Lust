// the door of a developer over MCP: an AI builds a program in one project folder (project.rs)
// and never touches the rust core. the person gave it that folder (lust dev <folder>); it
// writes lua files there and nowhere else, and every change goes into the live world at
// once (hot: the values stay, the new code runs from the next tick). the tools:
//   lust_guide    the project format and every lua function, once
//   lust_files    the files, and how the live world is
//   lust_read     a file, or some of its lines
//   lust_write    a whole file          lust_edit   exact text replaced in a file
//   lust_delete   a file                lust_undo   the last changes back
//   lust_restart  the live world again from world.lua
//   lust_try      driver code on a fresh world (or the live one)
//   lust_test     the tests of the project
// a change that does not compile is refused and nothing is saved; a change that is saved
// is tried at once on a fresh world (one tick, one frame) and the answer says how it went.
use crate::driver::{run_tests, Driver};
use crate::host::clip;
use crate::mcp::{Tool, Tools};
use crate::project::{entity_of, parse_world, Session, FILE_MAX};
use crate::screen;
use serde_json::{json, Value};

pub const GUIDE: &str = r#"LUST: you build a program in one folder of lua files. You change only these files, through these tools; the live world takes each change at once and keeps its values (the new code runs from the next tick). lust_restart starts the world again from world.lua. When the game is ready, lust_pack makes it one file (dist/<folder>) that a person double clicks: it opens at once on the menu you built, no command line.

FILES (<n> is [a-z0-9_])
world.lua     the start of the world (see WORLD)
sys/<n>.lua   a system: return function(dt) ... end, every tick, dt in seconds
draw/<n>.lua  drawing: return function() ... end, after the ticks of a frame; reads only
op/<n>.lua    an operation: return function(p) ... end; p holds its parameters
rule/<n>.lua  a rule: return function() ... end, after every operation
view/<n>.lua  a view: return function(arg) return {...} end; reads only
lib/<n>.lua   shared code: local M = {} ... return function() return M end
              use it: local M = ecs.call('lib.<n>')
test/<n>.lua  a test: driver code (see DRIVER) on a fresh world; assert() fails it
<n>.md        notes for you and for people
sys/ball.lua is the entity sys.ball; errors name the file and the line.
The top of a code file runs again after each change of it: keep state in components, not in lua locals, and do not spawn there.
Systems and drawings run in the order of ord (world.lua: ['sys.ball'] = {ord = 2}), then by name.

WORLD
return {
  spec = 'game',   -- 'game': fast, no journal, no roles, no undo, lua kept between ticks
                   -- 'ledger': a journal, roles with rights, a failed operation undone
  types = { x = 'num', name = 'text', stock = {kind = 'int', write = 'clerk system'} },
  entities = { ball = {x = 50, y = 30},
    ['op.sell'] = {params = 'item:id n:int', roles = 'clerk', desc = 'sells n'},
    ['role.clerk'] = {desc = 'sells'} },
  spawn = { {x = 1}, {x = 2} },  -- nameless entities #1, #2
}
kinds: num (a number in a fast column), int, bool, text, ref (an existing entity), name, names (words), code. params kinds: id, text, int, long; name? is optional.
world.lua is lua: loops may build entities. A change of it applies what changed (added, changed, removed values); spec and spawn need lust_restart. In a ledger, write lists the roles that may write a type; systems and rules act as the role system.

LUA IN CODE FILES (e is a name or a number, c a component name)
ecs.get(e, c)  ecs.set(e, c, v)  ecs.del(e, c)  ecs.kill(e)
ecs.spawn({c = v}) -> number of a new nameless entity   ecs.spawn(name, {c = v})
ecs.with(c [, v]) -> names   ecs.comps(e)   ecs.count([c])   ecs.id(e)   ecs.name(n)   ecs.tick()
fast, all entities at once (num components): ecs.query(c1, c2, ...) -> n, ids, v1, v2, ...
  ecs.write(c, ids, vals [, n])   ecs.add(c1, c2, k) (c1 += c2 * k)   ecs.scale(c, k)
  ecs.fill(c, v)   ecs.clamp(c, lo, hi)   ecs.sum(c) -> sum, n   ecs.col(c) -> a column number
ecs.call(entity, arg) -> what that entity's function returns
input.down(key)  input.pressed(key) (went down this tick)  input.mouse() -> x, y, down
  keys: up down left right space enter escape shift ctrl alt tab backspace a-z 0-9 mouse
SAVES: ecs.save(slot) and ecs.load(slot) (after this tick; load gives false when there is no such slot), ecs.slots() -> names, ecs.erase(slot). slot: 1..32 of a-z 0-9 _ -, at most 20. Only data is saved, not code; a load is refused when a type is gone or has another kind (said on the status line). In tests and lust_try the saves are in memory; the live game keeps them in the user's data folder, so they stay between starts.
input.mic() -> how loud the microphone hears, 0 to 1. Call gfx.mic(true) in a draw file to ask for it: the browser asks the person and shows 'microphone on'; only the loudness reaches the world, never the sound. Driver: mic(level).
gfx, only in draw files: gfx.size(w, h) (the screen in your units, default 800 600; y grows down)
  gfx.clear(color)  gfx.rect(x, y, w, h, color)  gfx.circle(x, y, r, color)
  gfx.line(x1, y1, x2, y2, width, color)  gfx.text(x, y, size, color, text)
  color: '#rgb', '#rrggbb' or a css name
  gfx.measure(text, size) -> w, h  (anywhere, also in sys files): the box of a text; a widget knows its size
UI (built into every project): local ui = ecs.call('lib.ui')
  ui.button(text, x, y, {w, h, size, pad, enabled, id}) -> b with b.x b.y b.w b.h (as big as its text needs)
  ui.hover(b)  ui.down(b)  ui.clicked(b)  ui.inside(b, px, py)  ui.draw(b, focused) (draw files)
  ui.label(text, x, y, {align = 'left'|'center'|'right', size, color}) -> w, h   ui.panel(x, y, w, h)   ui.paragraph(text, x, y, w, opts) (wraps to w)
  ui.theme: text, dim, back, button, hover, border, focus (colors), size, pad. Text may be in any language (utf-8).
  ui.menu({'Play', 'Options'}, cx, top, {size, gap}) -> list of buttons, centered, one width, stacked
  ui.update_menu(list, sel, {wrap}) -> sel, chosen index or nil, edge (in a sys file)  ui.draw_menu(list, sel)
  ui.stack / ui.row(list, cx, top, gap) place widgets.
  A game with screens (menu, play, ...): one number app.screen in world.lua; lib/layout.lua builds the buttons
  (return function() return M end, M.menu() = ui.menu({...}, 400, 330, {size = 32, gap = 16})); sys/screens.lua
  does `local sel, chosen = ui.update_menu(layout.menu(), ecs.get('app', 'sel'))`, ecs.set the sel, and on
  `chosen == 1` sets app.screen; draw/screens.lua branches on app.screen and calls ui.draw_menu(layout.menu(), sel).
  Escape: input.pressed('escape'). ecs.quit() ends the game (an Exit button; the window closes; in lust dev the world only pauses).
  Build the same widgets in the sys file (clicks) and in the draw file (drawing); draw files cannot write.
  Text is measured as monospace, the width the browser reports. F1 in a game from the lust list goes back to the list.
lua: string, table, math, utf8. No io, os, require, pcall, string patterns (string.find_plain(s, sub) instead). error('msg', 0) fails a run. Each run has a time and memory budget.
Numbers of num components are floats: 'Score: ' .. n shows 3.0; write string.format('%d', n) for a whole number (gfx.text(x, y, size, color, n) shows 3).
Speed: ecs.get/ecs.set cost about 0.1 us a call; for thousands of entities use query/write/add.

DRIVER (lust_try, test files; lua of its own)
step([n [, dt]]) n ticks (1, dt 1/60)   get(e, c)   set(e, c, v)   ents(c)   tick()
keys(k, ...) held from now on (keys() lets go)   press(k) down on the next tick
mouse(x, y, down): the button going down is a click (input.pressed('mouse')). A click: mouse(x, y, true) step() mouse(x, y, false) step()
The driver has no ecs, input, gfx: it looks at the world from outside with get/set/draw.
draw() -> the frame as text lines   events() -> what the world told the display since the last call, e.g. {'quit'}   run(role, op, {k = v})   view(name [, arg])   ms()
print(...) and returned values are the answer (numbers 4 decimals).

TOOLS
lust_write and lust_edit refuse code that does not compile (nothing is saved); a saved change is tried on a fresh world (one tick, one frame) and the answer says how. lust_edit: old must occur once; several edits at once with edits = [{old, new}]. lust_files shows where the person sees the live world.
"#;

pub struct DevTools<'a> {
    pub s: &'a mut Session,
}

fn tool(name: &str, description: &str, input: Value, read_only: bool) -> Tool {
    Tool { name: name.to_string(), description: description.to_string(), input, read_only }
}

fn arg<'v>(args: &'v Value, k: &str) -> Result<&'v str, String> {
    args.get(k).and_then(|v| v.as_str()).ok_or_else(|| format!("missing_param: {}", k))
}

fn lines_of(text: &str) -> usize {
    text.lines().count()
}

impl DevTools<'_> {
    // a fresh world from the files: one tick, one frame
    fn smoke(&self) -> String {
        match self.s.project.fresh() {
            Err(e) => format!("fresh world: {}", e),
            Ok((_, problems)) if !problems.is_empty() => format!("fresh world: {}", problems.join("; ")),
            Ok((app, _)) => {
                let (t, d) = app.frame(1.0 / 60.0, screen::Input::default());
                match (t, d) {
                    (Err(e), _) => format!("fresh world: tick 1: {}", e),
                    (_, Err(e)) => format!("fresh world: draw: {}", e),
                    _ => String::from("fresh world: ok"),
                }
            }
        }
    }

    // a file is checked before it is saved: it must compile (code) or read (world.lua)
    fn check(&self, path: &str, text: &str) -> Result<(), String> {
        if text.len() > FILE_MAX {
            return Err(format!("too_big: at most {} bytes", FILE_MAX));
        }
        if path == "world.lua" {
            return parse_world(text).map(|_| ());
        }
        if let Some((entity, _)) = entity_of(path) {
            if text.trim().is_empty() {
                return Err(String::from("the file is empty"));
            }
            return self.s.app.host.check_code(&entity, text).map_err(|e| e.replace(&format!("[string \"{}\"]", entity), path));
        }
        if path.starts_with("test/") {
            return self.s.app.host.check_code(path, text).map_err(|e| e.replace(&format!("[string \"{}\"]", path), path));
        }
        Ok(())
    }

    // saves (or deletes) a file, gives it to the live world and tries a fresh one
    fn change(&mut self, path: &str, text: Option<&str>) -> Result<String, String> {
        if let Some(t) = text {
            self.check(path, t).map_err(|e| format!("refused, nothing saved: {}", e))?;
        }
        self.s.project.write(path, text)?;
        let lines = self.s.sync();
        Ok(self.report(path, text, lines))
    }

    fn report(&self, path: &str, text: Option<&str>, lines: Vec<String>) -> String {
        let mut out = match text {
            Some(t) => format!("saved {} ({} lines)", path, lines_of(t)),
            None => format!("deleted {}", path),
        };
        for l in lines.iter().filter(|l| !l.ends_with(": applied") && !l.ends_with(": removed")) {
            out.push('\n');
            out.push_str(l);
        }
        out.push('\n');
        out.push_str(&self.smoke());
        if self.s.paused.is_some() {
            out.push('\n');
            out.push_str(&self.s.status());
        }
        out
    }

    fn edit(&mut self, args: &Value) -> Result<String, String> {
        let path = arg(args, "path")?;
        let mut text = self.s.project.read(path)?;
        let mut pairs: Vec<(String, String)> = Vec::new();
        if let Some(list) = args.get("edits").and_then(|v| v.as_array()) {
            for e in list {
                pairs.push((arg(e, "old")?.to_string(), arg(e, "new")?.to_string()));
            }
        }
        if let (Some(o), Some(n)) = (args.get("old").and_then(|v| v.as_str()), args.get("new").and_then(|v| v.as_str())) {
            pairs.push((o.to_string(), n.to_string()));
        }
        if pairs.is_empty() {
            return Err(String::from("missing_param: old and new, or edits"));
        }
        for (i, (old, new)) in pairs.iter().enumerate() {
            let n = if old.is_empty() { 0 } else { text.matches(old.as_str()).count() };
            if n != 1 {
                return Err(format!("edit {}: old occurs {} times in {}; it must occur once (nothing saved)", i + 1, n, path));
            }
            text = text.replacen(old.as_str(), new, 1);
        }
        self.change(path, Some(&text))
    }

    fn files(&self) -> Result<String, String> {
        let (files, skipped) = self.s.project.scan()?;
        let mut out = String::new();
        for (p, t) in &files {
            out.push_str(&format!("{} {}\n", p, lines_of(t)));
        }
        if files.is_empty() {
            out.push_str("(no files yet: start with world.lua)\n");
        }
        if !skipped.is_empty() {
            out.push_str(&format!("not part of the project: {}\n", skipped.join(", ")));
        }
        for p in &self.s.problems {
            out.push_str(p);
            out.push('\n');
        }
        out.push_str(&self.s.status());
        if let Some(u) = &self.s.url {
            out.push_str(&format!("\nthe person sees it at {}", u));
        }
        Ok(out)
    }

    fn read(&self, args: &Value) -> Result<String, String> {
        let text = self.s.project.read(arg(args, "path")?)?;
        let from = args.get("from").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
        let to = args.get("to").and_then(|v| v.as_u64()).map(|x| x as usize);
        if from == 1 && to.is_none() {
            return Ok(text);
        }
        let lines: Vec<&str> = text.lines().collect();
        let end = to.unwrap_or(lines.len()).min(lines.len());
        if from > end {
            return Ok(String::new());
        }
        Ok(lines[from - 1..end].join("\n"))
    }

    fn try_code(&self, args: &Value) -> Result<String, String> {
        let code = arg(args, "code")?;
        let live = args.get("live").and_then(|v| v.as_bool()).unwrap_or(false);
        if live {
            return Driver::new(&self.s.app)?.run("try", code);
        }
        let (app, problems) = self.s.project.fresh()?;
        let d = Driver::new(&app)?;
        let r = d.run("try", code);
        if problems.is_empty() {
            r
        } else {
            let note = format!("(the world has problems: {})", problems.join("; "));
            r.map(|o| format!("{}\n{}", note, o)).map_err(|e| format!("{}\n{}", note, e))
        }
    }

    fn test(&self, args: &Value) -> Result<String, String> {
        let only = args.get("name").and_then(|v| v.as_str());
        let results = run_tests(&self.s.project, only)?;
        let mut out = String::new();
        let mut failed = 0;
        for (name, r, ms) in &results {
            match r {
                Ok(_) => out.push_str(&format!("ok {} ({:.0} ms)\n", name, ms)),
                Err(e) => {
                    failed += 1;
                    out.push_str(&format!("FAIL {}: {}\n", name, clip(e, 600)));
                }
            }
        }
        out.push_str(&format!("{} ok, {} failed", results.len() - failed, failed));
        if failed > 0 {
            Err(out)
        } else {
            Ok(out)
        }
    }
}

impl Tools for DevTools<'_> {
    fn server(&self) -> (String, String, String) {
        (String::from("lust"), String::from(env!("CARGO_PKG_VERSION")), String::from("You build a program in this folder with lua files. Read lust_guide once, then lust_files."))
    }

    fn tools(&self) -> Vec<Tool> {
        let none = json!({"type": "object", "properties": {}});
        let path = json!({"type": "string"});
        vec![
            tool("lust_guide", "The project format and every lua function. Read it once.", none.clone(), true),
            tool("lust_files", "The files (path and lines) and how the live world is.", none.clone(), true),
            tool("lust_read", "A file, or its lines from..to.", json!({"type": "object", "properties": {"path": path, "from": {"type": "integer"}, "to": {"type": "integer"}}, "required": ["path"]}), true),
            tool("lust_write", "Write a whole file; the live world takes it at once.", json!({"type": "object", "properties": {"path": path, "text": {"type": "string"}}, "required": ["path", "text"]}), false),
            tool(
                "lust_edit",
                "Replace exact text in a file: old (occurs once) by new, or edits = [{old, new}].",
                json!({"type": "object", "properties": {"path": path, "old": {"type": "string"}, "new": {"type": "string"}, "edits": {"type": "array", "items": {"type": "object", "properties": {"old": {"type": "string"}, "new": {"type": "string"}}, "required": ["old", "new"]}}}, "required": ["path"]}),
                false,
            ),
            tool("lust_delete", "Delete a file.", json!({"type": "object", "properties": {"path": path}, "required": ["path"]}), false),
            tool("lust_undo", "Take the last n changes of files back (1).", json!({"type": "object", "properties": {"n": {"type": "integer"}}}), false),
            tool("lust_restart", "The live world again from world.lua.", none.clone(), false),
            tool("lust_pack", "Make the game one file a person double clicks (dist/<folder name>, .exe on Windows); it opens on the menu of the game. Do it when the game is ready.", none, false),
            tool(
                "lust_try",
                "Run driver code (see the guide) on a fresh world, or on the live one with live = true; answers what it printed and returned.",
                json!({"type": "object", "properties": {"code": {"type": "string"}, "live": {"type": "boolean"}}, "required": ["code"]}),
                false,
            ),
            tool("lust_test", "Run the tests test/*.lua, each on a fresh world, or only the one named.", json!({"type": "object", "properties": {"name": {"type": "string"}}}), true),
        ]
    }

    fn call(&mut self, name: &str, args: &Value) -> Result<String, String> {
        match name {
            "lust_guide" => Ok(GUIDE.to_string()),
            "lust_files" => self.files(),
            "lust_read" => self.read(args),
            "lust_write" => {
                let path = arg(args, "path")?.to_string();
                let text = arg(args, "text")?.to_string();
                self.change(&path, Some(&text))
            }
            "lust_edit" => self.edit(args),
            "lust_delete" => {
                let path = arg(args, "path")?.to_string();
                self.change(&path, None)
            }
            "lust_undo" => {
                let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 100) as usize;
                let paths = self.s.project.undo(n)?;
                let lines = self.s.sync();
                let mut out = format!("back: {}", paths.join(", "));
                for l in lines.iter().filter(|l| !l.ends_with(": applied") && !l.ends_with(": removed")) {
                    out.push('\n');
                    out.push_str(l);
                }
                out.push('\n');
                out.push_str(&self.smoke());
                Ok(out)
            }
            "lust_restart" => {
                let problems = self.s.restart();
                Ok(if problems.is_empty() { String::from("restarted") } else { format!("restarted with problems:\n{}", problems.join("\n")) })
            }
            "lust_pack" => {
                if !self.s.problems.is_empty() {
                    return Err(format!("not packed, the game has problems:\n{}", self.s.problems.join("\n")));
                }
                let program = std::env::current_exe().map_err(|e| e.to_string())?;
                let (to, size) = crate::pack::pack(&self.s.project.root, &program)?;
                Ok(format!("packed {} ({} KB); double click it", to.display(), size / 1024))
            }
            "lust_try" => self.try_code(args),
            "lust_test" => self.test(args),
            _ => Err(String::from("unknown_tool")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::handle;
    use std::fs;

    fn call(s: &mut Session, name: &str, args: Value) -> (bool, String) {
        let mut t = DevTools { s };
        let r = handle(&mut t, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})).unwrap();
        (r["result"]["isError"].as_bool().unwrap_or(true), r["result"]["content"][0]["text"].as_str().unwrap_or("").to_string())
    }

    #[test]
    fn an_ai_builds_a_program_from_an_empty_folder() {
        let root = std::env::temp_dir().join(format!("lust-dev-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut s = Session::open(&root).unwrap();
        let (err, files) = call(&mut s, "lust_files", json!({}));
        assert!(!err && files.contains("no files yet"), "{}", files);
        let (err, a) = call(&mut s, "lust_write", json!({"path": "world.lua", "text": "return { types = { x = 'num' }, entities = { ball = { x = 0 } } }"}));
        assert!(!err && a.contains("fresh world: ok"), "{}", a);
        // code that does not compile: refused, nothing saved
        let (err, a) = call(&mut s, "lust_write", json!({"path": "sys/move.lua", "text": "return function(dt) ecs.set('ball', 'x', end"}));
        assert!(err && a.starts_with("refused, nothing saved") && a.contains("sys/move.lua:1:"), "{}", a);
        assert!(!root.join("sys/move.lua").exists());
        // code that compiles but fails: saved, and the answer says where it fails
        let (err, a) = call(&mut s, "lust_write", json!({"path": "sys/move.lua", "text": "return function(dt)\n  ecs.set('ball', 'x', ecs.get('ball', 'y') + 1)\nend"}));
        assert!(!err && a.contains("fresh world: tick 1: sys/move.lua:2:"), "{}", a);
        let (err, a) = call(&mut s, "lust_edit", json!({"path": "sys/move.lua", "old": "'y') + 1", "new": "'x') + 60 * dt"}));
        assert!(!err && a.contains("fresh world: ok"), "{}", a);
        let (err, a) = call(&mut s, "lust_edit", json!({"path": "sys/move.lua", "old": "ecs", "new": "x"}));
        assert!(err && a.contains("occurs 2 times"), "{}", a);
        let (err, a) = call(&mut s, "lust_try", json!({"code": "step(60) return get('ball', 'x')"}));
        assert!(!err && a == "60", "{}", a);
        let (err, a) = call(&mut s, "lust_write", json!({"path": "draw/main.lua", "text": "return function() gfx.size(100, 60) gfx.circle(ecs.get('ball', 'x'), 30, 2, '#fff') end"}));
        assert!(!err, "{}", a);
        let (_, a) = call(&mut s, "lust_try", json!({"code": "step(30) return draw()"}));
        assert_eq!(a, "size 100 60\ncircle 30 30 2 #fff");
        let (err, a) = call(&mut s, "lust_write", json!({"path": "test/moves.lua", "text": "step(120) assert(get('ball', 'x') > 100, 'too slow')"}));
        assert!(!err, "{}", a);
        let (err, a) = call(&mut s, "lust_test", json!({}));
        assert!(!err && a.contains("1 ok, 0 failed"), "{}", a);
        let (err, a) = call(&mut s, "lust_undo", json!({"n": 1}));
        assert!(!err && a.starts_with("back: test/moves.lua"), "{}", a);
        let (err, a) = call(&mut s, "lust_write", json!({"path": "../escape.lua", "text": "x"}));
        assert!(err && a.contains("bad_path"), "{}", a);
        let (err, a) = call(&mut s, "lust_read", json!({"path": "sys/move.lua", "from": 2, "to": 2}));
        assert!(!err && a.trim() == "ecs.set('ball', 'x', ecs.get('ball', 'x') + 60 * dt)", "{}", a);
        let _ = fs::remove_dir_all(&root);
    }
}
