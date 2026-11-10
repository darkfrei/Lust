# A program as a folder of Lua files (`lust dev`)

English | [Русский](dev.ru.md)

Lust is the core. A program built on it is **a folder of Lua files**: a game, a tool, an
ERP module. Nobody, neither a person nor an AI, has to change the Rust code to make one.
An AI builds the program through MCP, and the person watches it run in the browser while
it grows.

```
cargo build --release                      # makes target/release/lust (lust.exe on Windows)
lust dev  <folder> [--port 8730] [--fps 60] [--no-web]   # an MCP server for an AI, and the live world
lust run  <folder> [--port 8730] [--fps 60]              # the world in the browser
lust test <folder>                                       # the tests of the folder
```

Try it with the Pong that an AI built through MCP alone (below):

```
lust run examples/pong          # then open http://127.0.0.1:8730, keys W/S or up/down
lust test examples/pong
```

## Giving an AI a folder

The folder is the permission. The person chooses it once, in the settings of the AI
client. The AI writes Lua files there and nowhere else, and never touches Rust.

Claude Desktop (`claude_desktop_config.json`) or any MCP client that starts a program:

```json
"mcpServers": {
  "pong": { "command": "C:\\tools\\lust.exe", "args": ["dev", "D:\\games\\pong"] }
}
```

Claude Code: `claude mcp add pong -- C:\tools\lust.exe dev D:\games\pong`

The folder must exist. While the client runs, the person opens the address `lust dev`
prints (default `http://127.0.0.1:8730`) and sees the world change with every edit.

## The folder

| File | What it is | Its entity |
|---|---|---|
| `world.lua` | the start: spec, types, named entities, nameless ones | |
| `sys/<n>.lua` | a system: `return function(dt) ... end`, every tick | `sys.<n>`, `tick = true` |
| `draw/<n>.lua` | drawing, after the ticks, read-only | `draw.<n>`, `draw = true` |
| `op/<n>.lua` | an operation: `return function(p) ... end` | `op.<n>` |
| `rule/<n>.lua` | a rule, after every operation | `rule.<n>`, `system = true` |
| `view/<n>.lua` | a view: returns a table | `view.<n>` |
| `lib/<n>.lua` | shared code, reached with `ecs.call('lib.<n>')` | `lib.<n>` |
| `test/<n>.lua` | a test: driver code on a fresh world | not in the world |
| `<n>.md` | notes for the AI and for people | |

`<n>` is `[a-z0-9_]`. Other files are left alone. `world.lua` is Lua, so loops may build
entities:

```lua
return {
  spec = 'game',   -- or 'ledger', or {journal = true, roles = false, undo = true, keep_lua = true}
  types = { x = 'num', y = 'num', stock = {kind = 'int', write = 'clerk system'} },
  entities = {
    ball = { x = 400, y = 300 },
    ['sys.ball'] = { ord = 2 },                              -- the order of systems
    ['op.sell'] = { params = 'item:id n:int', roles = 'clerk', desc = 'sells n' },
  },
  spawn = { {x = 1}, {x = 2} },                              -- nameless entities
}
```

Everything else (the Lua API inside the files, the driver, the tools) is in one text the
AI reads once: `lust_guide`, the constant `GUIDE` in `src/dev.rs`. It is the reference.

## A change is hot

Lua is interpreted, so nothing restarts. When a file changes:

- **code** (`sys/`, `draw/`, `op/`, ...): it is compiled first; code that does not compile is
  refused and the old code goes on. Code that compiles replaces the old one, and the world
  runs it from the next tick. **The values of the world stay**: the ball keeps flying.
  The top of the file runs again (its locals start over), so the state belongs in
  components.
- **`world.lua`**: what changed in it is applied (added, changed, removed values and
  types); the rest of the state stays. A new `spec` or `spawn` needs a restart.
- **a deleted code file**: its entity loses its code and stops.

This works the same for files the person edits in an editor: `lust dev` and `lust run`
look at the folder twice a second.

A world that fails (an error in a tick or a drawing) stops, keeps showing its last frame
and says on the page where it failed: `tick 912: sys/ball.lua:23: attempt to ...`. The next
change lets it run again. `lust_restart` starts it again from `world.lua`.

## The tools of the AI (`src/dev.rs`)

| Tool | What it does |
|---|---|
| `lust_guide` | the format and every Lua function, once |
| `lust_files` | the files with their lines, how the live world is, where the person sees it |
| `lust_read` | a file, or its lines `from`..`to` |
| `lust_write` | a whole file |
| `lust_edit` | exact text replaced: `old` (must occur once) by `new`, or several `edits` |
| `lust_delete` | a file |
| `lust_undo` | the last `n` changes back (the person's edits too) |
| `lust_restart` | the live world again from `world.lua` |
| `lust_try` | driver code on a fresh world, or on the live one (`live = true`) |
| `lust_test` | the tests `test/*.lua`, each on a fresh world |

Every saved change is tried at once on a fresh world (one tick, one frame), and the answer
says how it went. So an AI learns about an error in the same answer, with the file and the
line, instead of finding it later.

**The driver** (`src/driver.rs`) is Lua of its own that runs a world from outside:
`step(n, dt)`, `get`, `set`, `keys('up')`, `press('space')`, `mouse(x, y, down)`, `draw()`
(the frame as text), `run(role, op, args)`, `view`, `ms()`, `print`. An AI tests a game it
cannot see: it holds keys, steps, reads the values and the frame.

**History** (`.lust/history` in the folder): every change keeps what was there before, the
last 500. `lust_undo` takes them back.

## The screen is a module (`src/screen.rs`, `src/web.rs`)

The core knows only a **frame as text** and the **input**:

```
size 800 600
clear #000
rect 25 255 10 90 #fff
circle 400 300 8 #ff0
text 330 20 40 #fff 3
```

Lua draws with `gfx.size/clear/rect/circle/line/text` (only in `draw/` files) and reads
`input.down(key)`, `input.pressed(key)`, `input.mouse()`, `input.mic()` (the loudness of the microphone, 0 to 1; a draw file asks for it with `gfx.mic(true)`; the browser asks the person and shows that it is on; only the number reaches the world). A display shows the frame and
gives back the input through an `Exchange` (a mutex, no Lua in it).

`gfx.measure(text, size)` gives the width and height of a text, anywhere (also in `sys/`).
The browser reports the width of its monospace letters (`cw=`), so the answer is what you
see. Every project also has `lib.ui` built in (`local ui = ecs.call('lib.ui')`): buttons
that know their `w` and `h`, labels, panels, `stack`/`row` placing, `menu`, `update_menu`
(keys, mouse, enter), `paragraph` (wraps text to a width). `examples/menu` is a template for
a game with a menu, play, options and about screens.

## Saves (`src/saves.rs`)

`ecs.save(slot)` and `ecs.load(slot)` act after the tick (a load gives `false` when there is
no such slot); `ecs.slots()` lists them, `ecs.erase(slot)` removes one. A slot is 1 to 32 of
`a-z 0-9 _ -`, at most 20 slots, 4 MB each. Only the data is saved: the code and the types
come from the files, so changing the code never breaks a save. A load is checked against
the types of the game and refused, with the reason on the status line, when a type is gone
or has another kind. The saves are in the folder of the user's data (`%APPDATA%\lust\<game>`
on Windows, `~/.local/share/lust/<game>` elsewhere), named after the game folder; a packed
game carries that name. Tests and `lust_try` keep saves in memory. `examples/menu` saves its
score and options.

## Events and the end of a game (`src/screen.rs`)

A frame may be missed or taken twice; a sound or the end of the game must happen exactly
once. So the world has a second way to the display: events. Code of a tick makes one
(`ecs.quit()` now, sounds later; at most 16 a tick, never from a `draw/` file). The program
numbers them and keeps the last 64 for 5 s. The page sends the number of the last event it has
(`ack=`) with every request and gets the events after it; a new page (`ack=new`) gets none.
`ecs.quit()` ends the program once the page has heard it (or after 1 s); the page says that
the game has ended. In `lust dev` the world only pauses, so the AI can go on. In a test,
`events()` gives what the world told the display since the last call.

## When the page is closed

`lust run` and a double click pause the game while no page shows it (a hidden tab, or before
the browser opened) and end when the page is closed (3 s later, so a reload is not an end)
or when no page was heard of for 3 minutes. `lust dev` keeps running: its AI client ends it.
A game that draws nothing shows what to do instead of a black page.

## A game as one file (`lust_pack`, `src/pack.rs`)

When the game is ready, the AI calls `lust_pack` (or you run `lust pack <folder>`): it makes
`<folder>/dist/<folder name>` (`.exe` on Windows), the program lust with the lua files of
the folder glued to its end. A double click on that file opens the game on its own menu:
no command line, no list of games, no second file. At the start the file unpacks its lua
files into the temp directory and runs them; the console window of the double click closes.
The size is about 3 MB plus the files. It is the same lust, so a game needs no Rust.

## Starting without a command line

Double click `lust`: it shows the games next to it (folders with a `world.lua`, in the
folder `games` beside `lust`) as buttons in the browser. A click starts one, F1 comes back
to the list. Drop a game folder onto `lust.exe` to run just that game. With a `world.lua`
beside `lust`, it runs that folder.

`web.rs` is one such display: a page on `127.0.0.1` with a canvas. It has no dependencies
(std only), answers only requests whose `Host` is `127.0.0.1` or `localhost` (so a web page
elsewhere cannot reach the world through the browser), and takes at most 4 KB of input a
request. To show the world another way (a window, a terminal), write a module that reads
`Exchange::frame` and calls `Exchange::put_input`; the rest does not change. Without the
feature `web` it is not built.

## Safety

- The AI writes only paths of the table above, inside the folder: no `..`, no other
  extensions, at most 64 KB a file, 2000 files.
- The Lua of the world and of the driver runs in the sandbox of Lust: no `io`, `os`,
  `require`, `load`, `pcall`, no string patterns; a time and a memory budget for every run.
  A driver run may take 20 s.
- The page listens on `127.0.0.1` only. MCP is stdio: only the program that started
  `lust dev` talks to it.
- There is still no tool that confirms an operation of a role (`src/mcp.rs`). `lust dev`
  is for building, with the folder as the permission; a finished program gives an AI the
  proposal door instead.

## How many tokens (measured)

Pong, built from an empty folder through MCP only, the same game twice:

| | before (one `put` per value) | `lust dev` |
|---|---|---|
| messages | 101 | 11 |
| sent by the AI | 8 839 characters | 4 099 |
| read by the AI | 3 539 | 7 016, of which 6 645 once per session (tool list and guide) |
| a two-line fix | the whole file again, ~1 500 | `lust_edit`, ~70 |
| the result | numbers only, no picture, no keys | playable in the browser, with a test |

About 4 characters make a token, so the whole game cost roughly 3 000 tokens of content
plus the overhead of 11 calls. In the second run the code was right the first time (the
mistakes of the first run were known); a mistake now costs one `lust_edit` and the answer
already names the file and line.

A tick of this Pong takes 0.0064 ms; nothing of the AI side runs in the frame.

## Limits (honest)

- The guide is about 1 100 tokens, read once a session; it could be shorter.
- One project is one world. Several worlds and links between them (`src/link.rs`) still
  need Rust.
- The screen is simple 2D: rectangles, circles, lines, text. No images, no sound yet: each
  would be one more command of the frame and of the display.
- `lust dev` runs the world in the thread of the MCP messages: a long `lust_try` (up to
  20 s) holds the live frames meanwhile.
- The live world of `lust dev` keeps no save between sessions (a game spec has no journal).
