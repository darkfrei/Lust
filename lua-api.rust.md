# The Lua API of Lust, seen from the Rust side

`lua-api.lust.md` says what a Lua function does for a game. This file says **where it lives in
Rust**: which file, which function, who may call it, how a program adds its own. Every Lua
function has a comment `// lua api: <name>(<args>)` right above its code (a function of the test
driver: `// lua api (driver): ...`). To find one: `grep -rn "lua api" src`.

Words:

- **world**: `World` in `src/ecs.rs`, the entities and their values. Functions below read and
  change it through `Host::world`.
- **host**: `Host` in `src/host.rs`, one Lua state with the sandbox around one world.
- **sandbox**: what Lua is allowed to do (no files, no clock, no patterns); `new_sandbox` in
  `src/host.rs`.
- **read only**: a call that must not change the world. A `draw/` file runs read only; the
  functions that write fail with `readonly`.

## 1. How a function gets into Lua

Three doors, all in `src/host.rs`:

| Rust | Lua sees | Used by |
|---|---|---|
| `make_api` | the table `ecs` with the core functions (section 2) | built in |
| `Host::add_to_ecs(name, f)` | one more function `ecs.<name>`; refused when the name is taken (`taken: ecs.<name>`) | `saves`, `screen` (`ecs.quit`), `solver`, `launcher` |
| `Host::add_global(name, value)` | one more global such as `gfx` and `input`; `ecs` and the libraries cannot be replaced | `screen` |
| `Host::add_system(name, f)` | a Rust function that Lua calls as `ecs.run(name, arg)` | the program on top of Lust (Radost) |

All three must be called **before any code of the world runs**: a world that keeps its Lua builds
its environment once. The order in `project::build` (`src/project.rs`):

1. `screen::attach(&host)`: `gfx`, `input`, `ecs.quit`
2. `saves::attach(&host)`: `ecs.save`, `ecs.load`, `ecs.slots`, `ecs.erase`
3. `solver::attach(&host)`: the solver, only with the cargo feature `solver` (on by default)

Lua numbers reach Rust as `f64`; a `num` column holds `f64`. A Rust function that changes the
world should do it through `World::set`, so rights, undo and the journal hold.

## 2. The core: `ecs.*` in `src/host.rs`

All in `make_api`, one closure each. `e` is a name or a number (`ent_index`, `ent_opt`), `c` a
name or a column number (`num_col`, `comp_name`).

| Lua | Rust | Notes |
|---|---|---|
| `ecs.get(e, c)` | `make_api`, `"get"` | `World::get` (with the prefab chain) or `num_at` for a `num` column; hidden columns give nil |
| `ecs.own(e, c)` | `"own"` | the own value only |
| `ecs.set(e, c, v)` | `"set"` | `World::set`; any refusal raises, the host undoes the run when the spec says so |
| `ecs.del(e, c)` | `"del"` | |
| `ecs.kill(e)` | `"kill"` | |
| `ecs.instantiate(prefab, id)` | `"instantiate"` | `World::instantiate` |
| `ecs.spawn([name,] {c = v})` | `"spawn"` | `is_a` and `parent` are set first |
| `ecs.with(c [, v])` | `"with"` | sorted names |
| `ecs.children(e)` | `"children"` | |
| `ecs.exists(e)` | `"exists"` | |
| `ecs.comps(e)` | `"comps"` | |
| `ecs.kind(c)` | `"kind"` | |
| `ecs.id(e)`, `ecs.name(e)`, `ecs.col(c)`, `ecs.count([c])` | `"id"`, `"name"`, `"col"`, `"count"` | |
| `ecs.query(c1, ...)` | `"query"` | one pass over the columns; new tables |
| `ecs.read(c, ids, vals)` | `"read"` | into tables Lua keeps: no garbage |
| `ecs.write(c, ids, vals [, n])` | `"write"` | rights and undo hold |
| `ecs.add(dst, src [, k])`, `ecs.scale`, `ecs.fill`, `ecs.clamp`, `ecs.sum` | `"add"`, `"scale"`, `"fill"`, `"clamp"`, `"sum"` | whole-column math |
| `ecs.run(name [, arg])` | `"run"` | calls a system added by `Host::add_system` |
| `ecs.commit({k = v})` | `"commit"` | one result at a time; the program takes it with `Host::take_result` |
| `ecs.tick()` | `"tick"` | `Ctx::ticks` |
| `ecs.call(entity, arg)` | `"call"` | runs shared code with the rights of the caller; the top of its chunk is read only |
| `ecs.history(e, n)`, `ecs.journal(n)` | `"history"`, `"journal"` | at most 50; empty without a journal |
| `ecs.role()` | `"role"` | `World::current_role` |

The sandbox patches of the Lua libraries are in `new_sandbox` (`src/host.rs`):

| Lua | Notes |
|---|---|
| `setmetatable(t, mt)` | refuses a metatable with `__gc` (a finalizer would run later, outside the run) |
| `string.find_plain(s, sub)` | the only search left; `find`, `match`, `gmatch`, `gsub` are removed (a pattern can run for minutes inside one native call) |
| `string.rep(s, n [, sep])` | capped at `REP_MAX` bytes, else `rep_too_large` |

Removed: `dofile loadfile load loadstring require print pcall xpcall collectgarbage
getmetatable`. `pcall` is gone so a script cannot catch the time limit.

## 3. Drawing and input: `src/screen.rs`

`screen::attach` makes the tables `gfx` (`gfx_table`) and `input` (`input_table`). `gfx.*` writes
one text line per command into the frame (`Screen::push`); `Screen::draw` runs every `draw/`
file read only, with `Screen::drawing` set, and the display (`src/web.rs`) reads the text.

| Lua | Rust | Notes |
|---|---|---|
| `gfx.size(w, h)` | `gfx_table` `"size"` | 1..10000, else `bad_size` |
| `gfx.clear(c)`, `gfx.rect(...)`, `gfx.circle(...)`, `gfx.line(...)`, `gfx.text(...)` | `"clear"`, `"rect"`, `"circle"`, `"line"`, `"text"` | colors are checked (`color`), numbers are printed by `num`; a frame holds at most `CMDS_MAX` commands (`too_many_cmds`). All of `gfx.*` except `measure` raise `gfx_outside_draw` outside `draw/` |
| `gfx.measure(text, size)` | `"measure"` | works in `sys/` too: the width of a character comes from the display (`glyph`) |
| `gfx.mic(on)` | `"mic"` | asks the display for the microphone; only the loudness comes back |
| `input.down(k)`, `input.pressed(k)` | `input_table` | `Screen::key` normalizes the key name; the struct `Input` is filled by the display (`web.rs`, by physical key `e.code`) |
| `input.mouse()`, `input.mic()` | | the fields `Input::mouse`, `Input::mic` |
| `ecs.quit()` | `attach`, `add_to_ecs("quit")` | sends the event `quit` (`Screen::event`); from `draw/` it raises `event_in_draw`; at most `EVENTS_TICK_MAX` events a tick |

## 4. Saves: `src/saves.rs`

`saves::attach` adds the four functions. They only **queue** a job (`Saves::jobs`); the program
carries it out after the tick (`saves::after_tick`; errors in `Saves::take_errors`), so a save never
happens in the middle of a run.

| Lua | Rust |
|---|---|
| `ecs.save(slot)` | `Job::Save` |
| `ecs.load(slot)` | `Job::Load`; returns false at once when the slot is not there |
| `ecs.slots()` | `Saves::slots` |
| `ecs.erase(slot)` | `Saves::erase` |

A slot name is checked by `slot_ok` (1..32 of `a-z 0-9 _ -`); a wrong one raises `bad_slot`.

## 5. The solver: `src/solver/` (cargo feature `solver`)

`solver::attach` creates one `Scratch` (the arrays the columns are gathered into) and registers
each handler through `reg(host, &scratch, name, f)`. A handler has the type
`fn(&mut World, &mut Scratch, Args) -> mlua::Result<R>`: it parses the arguments itself
(`num_arg`, `opt_num_arg`, `pos_arg`, `iterations_arg`; a bad one raises `bad_arg: <name>`), then
runs `access::gather_*` (columns to arrays), the math, `access::scatter_*` (arrays back), under
`released`, which frees the arrays on every path. A handler that writes first calls `writable`
(raises `readonly` in a `draw/` file). A second call while one runs raises `busy`.

| Lua | Handler (`src/solver/mod.rs`) | Math in |
|---|---|---|
| `ecs.solve_tick(dt, substeps, gx, gy, damping [, spin, bend])` | `solve_tick` | calls the modules below in one gather and one scatter |
| `ecs.integrate_verlet(dt, gx, gy, damping)` | `integrate_verlet` | `integrate.rs` |
| `ecs.solve_distance(dt, iterations)` | `solve_distance` | `distance.rs` |
| `ecs.damp_angular(spin, bend)` | `damp_angular` | `angular.rs` |
| `ecs.project_segments(iterations [, dt])` | `project_segments` | `contact.rs` |
| `ecs.project_edges(iterations [, dt])` | `project_edges` | `contact.rs` |
| `ecs.project_points(iterations)` | `project_points` | `contact.rs` |
| `ecs.apply_catenary_load(dt, gy)`, `ecs.update_catenary(gy)`, `ecs.sample_catenary(id, t)` | same names | `catenary.rs` |
| `ecs.path(from, to [, cost])`, `ecs.distances(from [, cost])` | `path`, `distances` | `path.rs` |
| `ecs.query_point(x, y, max_dist [, mask])`, `ecs.raycast_segments(...)` | same names | `query.rs` |

The columns each function reads and writes are listed once, in `access.rs`
(`gather_points`, `gather_links`, `gather_edges`, `gather_segs`, `gather_cables`) and in
`SOLVER-SPEC.ru.md`. `data.rs` holds the array structs (`Points`, `Links`, ...). A new column:
add it in `data.rs`, read it in `access.rs`, give it a default, document it in
`lua-api.lust.md`, add a test in the module that uses it.

## 6. The test driver: `src/driver.rs`

Not a game API: Lua of its own that runs a world from outside (`test/*.lua`, `lust_try`). It
has no `ecs`, `input` or `gfx`; `Driver::install` makes tables of those names that raise a hint
on any use. The functions: `step`, `get`, `set`, `keys`, `press`, `mouse`, `mic`, `glyph`,
`draw`, `run`, `view`, `ents`, `tick`, `events`, `ms`, `print`. Each is marked
`// lua api (driver):`. `assert` is the Lua one.

## 7. The launcher: `src/launcher.rs`

The launcher is a small world of its own (files from `src/lua/launcher_*.lua`). It adds one
function, `ecs.launch(name)`, which remembers which `game.NNN` was picked; marked
`// lua api (launcher world only):`.

## 8. Not Rust

`lib.ui` (`src/lua/ui.lua`) is Lua, built into every project; it is described in
`lua-api.lust.md`, section 6. The MCP tools (`lust_guide`, `lust_write`, ...) are not a Lua API:
they are the door of the AI, in `src/dev.rs` and `src/mcp.rs`.

## 9. A new Lua function, the short way

1. Write the handler: a `create_function` closure in `make_api` (core), or a `fn` with the
   solver `Args` signature (solver), or a `Host::add_to_ecs` call from your own `attach`.
2. Put `// lua api: name(args) -> result` above it.
3. Check the arguments, and refuse a write when the world is read only (the solver's `writable`).
4. A test without a screen (`cargo test`); a function the AI should use is also named in
   `GUIDE` in `src/dev.rs`: the guide names nothing that does not exist, and every Lua function
   is in the guide.
5. Add it to `lua-api.lust.md` and to the table above.
