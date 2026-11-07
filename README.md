# Lust

**Rust + Lua + ECS + MCP.** A base for programs whose logic is written in Lua.

English | [Русский](README.ru.md)

Lust gives a program five things at once, already working together:

- **Lua in a sandbox**, fast (Rust underneath) and safe: a script cannot touch files or the
  system, and has a time and memory budget.
- **A native ECS**, as many worlds as the program needs, all of the same kind: entities with a
  name or a number, components as checked text or as numbers in dense columns. The rules are
  in a world too, as Lua code stored as data, so they can change while the program runs.
- **What a world keeps is its choice** (`Spec`): a journal (every change an entry of a hash
  chain), roles with rights, undo of a failed run, a Lua that remembers between ticks. A ledger
  or a turn-based game keeps much; an arcade game keeps nothing but a save.
- **A door for an AI (MCP)**: an AI client sees what its role may do and proposes; only
  a person confirms.
- **A program as a folder of Lua files** (`lust dev <folder>`): an AI builds a game or a
  tool through MCP, writing Lua files only, never Rust. Every change is hot (the world keeps
  its values, the new code runs from the next tick), and the person watches it in the
  browser. See [docs/dev.md](docs/dev.md).

On top of that, a program adds **its own Lua API**. A game engine in the spirit of LÖVE
adds its drawing and input, an ERP its documents and pages, a simulation its clock. The
hard, shared part (the sandbox, the worlds, the rights, the journal, the AI) is already
there.

**Status: 0.1.0, young.** About 7200 lines, 69 tests. The API may still change.

## Try it

```
cargo run --example hello     # 30 lines: one type, one role, one operation, a journal
cargo run --example kiosk     # a rule, a view, a failed run undone, save and replay
cargo run --example engine    # a tiny engine: its own screen.print and ecs.now, a game loop
cargo run --example door      # an AI through MCP: it sees its operations, proposes, a person confirms
cargo run --release --example save       # a ledger that computes: snapshots and journal volumes; run it again, it goes on
cargo run --release --example arkanoid   # a game world: no journal, no roles, a variable dt, saved and loaded midway
cargo run --release --example battle     # a kingdom (ledger) and a world for each battle: its result as one operation
cargo run --release --example frames     # how long a tick takes: mean, p99, the slowest
cargo build --example stdio   # an MCP server for a real AI client over stdio (see below)
cargo test

cargo build --release                           # the program lust
target/release/lust run examples/pong           # Pong, built by an AI through MCP: http://127.0.0.1:8730
target/release/lust dev <folder>                # an MCP server: an AI builds in that folder
```

```rust
use lust::host::Host;

let host = Host::new()?;                           // a ledger: journal, roles, undo
{
    let mut w = host.world.borrow_mut();
    w.current_role = String::from("dev");
    w.set("hits", "kind", "int")?;                 // a component type is an entity too
    w.set("hits", "write", "dev guest")?;          // who may write it
    w.set("role.guest", "desc", "may only knock")?;
    w.set("op.knock", "roles", "guest")?;          // who may run this operation
    w.set("op.knock", "src", "return function()
        ecs.set('door', 'hits', (ecs.get('door', 'hits') or 0) + 1)
    end")?;
}
host.run("guest", "knock", &[])?;                  // door.hits is now 1
```

(Shortened. `examples/hello.rs` is the complete, tested version; it also declares the
types `roles`, `params` and `src` that operations need.) A game world is
`Host::with_spec(Spec::game())`; see `examples/arkanoid.rs`.

## Worlds and what they keep (`src/ecs.rs`, `Spec`)

Every world is the same kind of thing; its `Spec` says what it keeps:

| Setting | `Spec::ledger()` | `Spec::game()` |
|---|---|---|
| `journal`: every change an entry of a hash chain | yes | no |
| `roles`: rights per component and per operation | yes | no |
| `undo`: a failed operation or tick is undone as a whole | yes | no |
| `keep_lua`: Lua globals live from run to run | no | yes |

Any mix is allowed (`Spec { journal: false, ..Spec::ledger() }`). A program makes as many worlds
as it needs (`Host::with_spec`) and drops them when it wants. One world gives a result to another
only as one operation of a role (`link.rs`). The model, the measurements and what is still a
problem: [docs/worlds.md](docs/worlds.md).

## The world (`src/ecs.rs`)

- An **entity** has a name (`door`, `op.knock`, `role.guest`) and a number (`ecs.id`: its index
  and a generation, so an old number never reaches a new entity). One made without a name is
  called `#1`, `#2`, ...
- A **component** is a table from entity to value. Every entry of one component has the
  same rights. So the world is stored component by component.
- The **types of components are entities too**. The entity `hits` with the components
  `kind = int` and `write = dev guest` describes the component `hits`. A developer
  changes types, code and data with the same calls.
- **Kinds** of values: `text`, `code`, `int`, `bool`, `ref` (the name of an entity that exists),
  `name`, `names` (several names, separated by spaces), all checked on every write and in the
  journal of a world that keeps one; and `num`, a number (`f64`) in a dense column, never in
  the journal, for what changes every tick (positions, speeds, health).
- **Two additions to a plain ECS:** `is_a` (an entity takes the values of its prefab
  unless it has its own; a prefab is a template entity) and `parent` (a component type
  that has a parent belongs to that entity: it may be set only on that entity, its
  children and their instances).
- **Rights** (in a world with roles): `write` lists the roles that may write a component. The
  built-in role `core` writes the first entries and cannot be used by any script.
- **Undo** (in a world with undo): a run (`World::begin`) that fails (`World::rollback`) puts
  back every value it changed, text and `num`, and every entity it made or removed.
- **Private values:** a component type marked `local_ai = true` reads as nothing while
  the world is shown to a reader that may not see it (`World::hide_private`), for example
  an AI in the cloud.
- **Limits** (a ledger; a game world has more): 200 000 entities, 2 000 000 values, names up to
  64 characters, text up to 4000, code up to 64 KB.

## The code (`src/host.rs`)

The code of an entity lives in its component `src` and returns a function. Five ways to
use it, all by naming convention:

| Entity | What it is | Runs |
|---|---|---|
| `op.<name>` | an **operation**: what a role may do (`roles`, `params`, `desc`) | when the program calls `host.run` (input, a move, a command) |
| anything with `system = true` | a **rule**: runs as the role `system`, in the order of `ord` | after every operation |
| anything with `tick = true` | a **tick**: runs as the role `system`, in the order of `ord`, with `dt` | when the program calls `host.tick(dt)` |
| `view.<name>` | a **view**: read-only, gives back a table (as JSON) | when asked |
| any other `src` | shared code, reached with `ecs.call` | when called |

`dt` is whatever the program passes: the time since the last frame, or a fixed step.
`clock::Clock` gives both (`Clock::variable(max_dt)`, `Clock::fixed(step, max_steps)`).

**An operation and its rules are one piece.** In a world with undo, if anything in it fails,
everything it did is undone, and the journal records why. Arguments are checked before
anything runs (`id`, `int`, `text`, `long`).

**The rights of a role hold inside code too.** If a clerk's operation tries to write a
component the clerk may not write, the run fails with `no_right`.

**The sandbox** is Lua 5.4 with only the libraries `table`, `string`, `math` and `utf8`.
There is no `io`, `os`, `require`, `load`, `pcall` or `getmetatable`, and `setmetatable`
refuses a metatable with `__gc` (a finalizer would run later, outside its run). Lua
patterns (`string.find`, `match`, `gsub`) are removed because they can run for minutes
inside one call; use `string.find_plain`. Each run has a time budget (250 ms by default,
set it with `budget.set_run_ms`) and a memory limit (`Spec::memory`). The code is compiled
once and kept until its text changes.

**Without `keep_lua`** (a ledger) nothing a script keeps in a global variable survives the run:
every run starts from fresh copies, and the top of a code chunk is read-only. **With
`keep_lua`** (a game) the globals live on, as in a game loop, and the top of a code chunk is its
load: it runs once (again when the code changes), may spawn the first entities and set globals.

**The Lua API** is one table, `ecs`, in every world. An entity is a name or a number, a `num`
component a name or a column number (`ecs.col`, faster):

| Call | What it does |
|---|---|
| `ecs.get(e, c)` | the value, from the entity or its prefab chain |
| `ecs.own(e, c)` | only the entity's own value |
| `ecs.set(e, c, v)` | write; raises on any refusal and the run is undone (with undo) |
| `ecs.del(e, c)` | remove an own value |
| `ecs.kill(e)` | remove an entity |
| `ecs.spawn(id, {c = v, ...})`, `ecs.spawn({c = v, ...})` | create an entity, with a name or without; gives its number |
| `ecs.id(e)`, `ecs.name(e)`, `ecs.col(c)`, `ecs.count([c])` | numbers, names, counts |
| `ecs.instantiate(prefab, id)` | make `id` (and its children) an instance of a prefab |
| `ecs.with(c [, v])` | sorted entities that have component `c` (equal to `v`) |
| `ecs.children(e)`, `ecs.exists(e)`, `ecs.comps(e)`, `ecs.kind(c)` | look around |
| `ecs.query(c, ...)` | `n, ids, vals, ...`: the entities that have all these `num` components, one call |
| `ecs.read(c, ids, vals)`, `ecs.write(c, ids, vals [, n])` | a whole `num` column out and back, one call each |
| `ecs.add(dst, src [, k])`, `ecs.scale`, `ecs.fill`, `ecs.clamp`, `ecs.sum` | a whole column at once |
| `ecs.run(name [, arg])` | a Rust system the program added (`host.add_system`) |
| `ecs.commit({key = value, ...})` | a result for the program, to give to another world |
| `ecs.call(entity, arg)` | run another entity's code (same rights) |
| `ecs.history(e, n)`, `ecs.journal(n)` | the last changes (n at most 50), in a world with a journal |
| `ecs.role()`, `ecs.tick()` | the role this run acts as, how many ticks the world has had |

On 10 000 entities one tick costs about 1 ms with `query`, a Lua loop and `write`, 0.006 ms with
`ecs.add` or a Rust system, about 3 ms one by one. More in [docs/worlds.md](docs/worlds.md).

## Your own API (for an engine, an ERP, anything)

A program built on Lust adds to what scripts see, before or between runs:

- `host.add_global(name, value)`: a new global, for example a table `screen` with
  functions, as LÖVE has `love.graphics`.
- `host.add_to_ecs(name, function)`: one more function in the table `ecs`.
- `host.add_system(name, f)`: a Rust function over the world's columns (`World::column_mut`),
  run from Lua as `ecs.run(name, arg)`. It is the program's own code: it writes past rights,
  undo and limits.
- `host.lua()`: the Lua state, to make those functions (Lust re-exports the binding as
  `lust::mlua`). The program keeps its own data there too, out of the world and out of the
  journal: `lua.set_app_data(...)`, read in a function with `lua.app_data_ref::<T>()`.
- `Host::on_refusal(hook)`: what happens after a refused or failed operation (for example
  counting refusals per role).
- `World::overlay`: values a reader sees instead of the stored ones while the world is
  read-only (for example several screens drawn from one world).

What is added is seen by every later run. In a world without `keep_lua` a table is copied fresh
for each run, so one run cannot change what the next one gets; names that exist (`ecs`, the
libraries, the calls of `ecs`) cannot be replaced. An added function that changes the world
should do it through `World::set`, so the rights and the journal hold. `examples/engine.rs` is a
tiny engine of this kind: `screen.print`, `ecs.now` and a game loop.

## The journal: a module (`src/journal.rs`, `src/store.rs`)

A world keeps a journal when its spec says `journal`. Every change is then one entry: who
(role), in which operation, what was changed, the new value, the hash of the previous entry and
its own hash (SHA-256). The entries form a chain. `num` values are never in it.

- `Store::append` writes new entries to a file, one JSON line each.
- `World::replay` rebuilds the world from the entries and checks the whole chain. A
  changed, cut or reordered entry is found.
- **What the chain does not do:** it has no secret. Whoever can write the file can rewrite
  it from some entry on and compute the hashes again. To notice that, keep the hash of the
  last entry (`World::head`) somewhere else, or sign it.
- A half-written last line is dropped with a warning. An old journal is never
  overwritten: `store::archive` moves it aside.
- Refusals and rollbacks are in the journal too, as entries that change nothing.

**Saving and loading.** A **snapshot** holds the whole state (`World::snapshot`,
`store::save_snapshot`): text and `num` values and the names of entities without a name, with
its own hash, so a snapshot changed by hand is refused. A game world saves only snapshots and
loads one with `World::from_snapshot_spec(spec, &snapshot, Vec::new())`
(`examples/arkanoid.rs`). The globals of a world's Lua are not saved: what must survive is in
components; after loading, the top of each code chunk runs again.

A ledger that computes much (`examples/save.rs`, 40 bodies pulling each other every tick)
keeps a snapshot now and then and a **journal volume**: the entries since that snapshot. They
continue the same hash chain. At each snapshot the old volume is moved aside
(`store::archive`), so the history is kept but is not needed to load. Loading is
`store::load_snapshot`, `store::load` of the volume and `World::from_snapshot(snapshot,
entries)`. The hash of the last entry kept in a file of its own shows a save cut or rewritten
outside the game. The rules are in the world, so a saved game keeps the rules it was played
with.

## From one world to another (`src/link.rs`)

Worlds are independent. A world computes something (a battle, a turn of an AI, a physics scene)
and commits a result (`ecs.commit{...}`); the program takes it (`host.take_result()`) and runs it
in another world as one operation of a role (`link::commit_to`), checked like any other
operation and written to that world's journal if it keeps one. A `Seed` remembers what the
computing world read from the other when it was made; if that changed meanwhile, the result is
refused as stale. `examples/battle.rs`: a battle of about 1100 soldiers, 3308 ticks, 4 entries in
the kingdom's journal.

## A program as a folder (`lust dev`, `src/project.rs`)

The program `lust` (`src/bin/lust.rs`) runs a folder of Lua files: `world.lua` (the start:
types and entities), `sys/*.lua` (systems), `draw/*.lua` (drawing), `op/*.lua`
(operations), `view/`, `rule/`, `lib/`, and `test/*.lua` (tests). `lust dev <folder>` is an
MCP server over stdio: an AI reads one guide, writes and edits the files, tries code on a
fresh world, runs the tests, and takes changes back. A change is hot: code that does not
compile is refused, code that does replaces the old from the next tick, the values stay.
The person sees the live world in the browser (`src/web.rs`, a module: another display can
take its place). Pong built this way from an empty folder took 11 MCP messages and about
3 000 tokens of content. All of it: [docs/dev.md](docs/dev.md).

## The door for an AI (`src/mcp.rs`)

MCP (Model Context Protocol) is the common way for an AI client to use the tools of a
program. Lust speaks it natively, without any prompt format of its own. Three parts, each
usable alone:

- **The protocol:** `mcp::handle` takes one JSON-RPC 2.0 message and gives the answer
  (`initialize`, `ping`, `tools/list`, `tools/call`); `mcp::handle_text` does the same for
  one line of text. The program chooses the transport (HTTP, or stdio: one message per
  line) and the tools, through the trait `mcp::Tools`.
- **A queue of proposals** (`mcp::Proposals`): an AI proposes an operation of its role. It
  is checked at once, as the role would run it (the operation, the right to run it, every
  argument), and waits. Only the program, a person, calls `confirm`; then it runs as that
  role, never as more, and the journal marks it `mcp <id>`. **There is no tool that
  confirms.** Limits: pending proposals, size of a request, decided ones kept.
- **Three ready tools** over a world and a role (`mcp::WorldTools`): `lust_ops` (the
  operations the role may propose, with typed parameters), `lust_propose` and
  `lust_status`. The AI sees no values of the world: what it should know is the program's
  choice, as a tool of its own.

**A real AI client over stdio** (`examples/stdio.rs`): a client such as Claude Desktop
starts the program and talks to it one JSON message per line; stdout belongs to the
protocol, what the person reads goes to stderr. The person needs a channel of their own
that the AI does not have: in the example a text file of decisions (`yes 3`, `no 3`),
kept where the AI cannot write (a client with file tools could otherwise decide for the
person). A real program gives the person a window or a page instead. How to add it to the
client is at the top of the example.

**It is optional.** It is on by default. To build without it, depend on Lust with
`default-features = false`. To remove it from the source: delete `src/mcp.rs`, `src/dev.rs`,
`src/bin/lust.rs`, `examples/door.rs` and `examples/stdio.rs`, the `mcp` lines in
`src/lib.rs`, and the blocks `[features]`, `[[example]] door`, `[[example]] stdio` and
`[[bin]] lust` in `Cargo.toml`. Nothing else in Lust uses it.

## What Lust is not

Lust has no window and no network beyond `127.0.0.1`. Its screen is a simple 2D frame
(rectangles, circles, lines, text) shown by a page in the browser; a program that needs
more (images, sound, a window, pages of an ERP) adds it through its own API or its own
display module. It has no prompt format for an AI either.

## Built on Lust

- **Radost**: an ERP core with a web interface and voice input, mods, and its own AI
  tools on Lust's door (prompt cards in the S-Card format, the inbox of what the person
  said, files). Its games, such as "Bolts Corporation: 20 years", are subprojects of
  Radost.

A program uses Lust as a normal Cargo dependency:

```toml
[dependencies]
lust = { git = "https://github.com/darkfrei/Lust" }
```

## Build

You need a Rust toolchain. The first build compiles Lua 5.4, so you need a C compiler (on
Windows, the Visual Studio Build Tools; on Linux, `gcc` or `clang`).

```
cargo build --release
cargo test
```

## License

MIT or Apache-2.0, at your option. See `LICENSE-MIT`, `LICENSE-APACHE` and `NOTICE`.
Lust includes Lua 5.4 (MIT, PUC-Rio) through the `mlua` crate; the other crates it uses are
listed in `Cargo.lock`.
