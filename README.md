# Lust

**Rust + Lua + ECS + MCP.** A base for programs whose logic is written in Lua.

English | [Русский](README.ru.md)

Lust gives a program four things at once, already working together:

- **Lua in a sandbox**, fast (Rust underneath) and safe: a script cannot touch files, the
  system or other runs, and has a time and memory budget.
- **A native ECS**: one world of entities and components. The rules are in it too, as
  Lua code stored as data, so they can change while the program runs.
- **Roles with rights and a journal**: every write is checked against the rights of the
  role, a failed run is undone as a whole, and every change is an entry in a hash chain.
- **A door for an AI (MCP)**: an AI client sees what its role may do and proposes; only
  a person confirms.

On top of that, a program adds **its own Lua API**. A game engine in the spirit of LÖVE
adds its drawing and input, an ERP its documents and pages, a simulation its clock. The
hard, shared part (the sandbox, the world, the rights, the journal, the AI) is already
there.

**Status: 0.1.0, young.** About 2500 lines, 34 tests. The API may still change.

## Try it

```
cargo run --example hello     # 30 lines: one type, one role, one operation, a journal
cargo run --example kiosk     # a rule, a view, a failed run undone, save and replay
cargo run --example engine    # a tiny engine: its own screen.print and ecs.now, a game loop
cargo run --example door      # an AI through MCP: it sees its operations, proposes, a person confirms
cargo test
```

```rust
use lust::host::Host;

let host = Host::new()?;
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
types `roles`, `params` and `src` that operations need.)

## The world (`src/ecs.rs`)

- An **entity** is only a name (`door`, `op.knock`, `role.guest`).
- A **component** is a table from entity to value. Every entry of one component has the
  same rights. So the world is stored component by component.
- The **types of components are entities too**. The entity `hits` with the components
  `kind = int` and `write = dev guest` describes the component `hits`. A developer
  changes types, code and data with the same calls.
- **Kinds** of values, checked on every write: `text`, `code`, `int`, `bool`, `ref` (the
  name of an entity that exists), `name`, `names` (several names, separated by spaces).
- **Two additions to a plain ECS:** `is_a` (an entity takes the values of its prefab
  unless it has its own; a prefab is a template entity) and `parent` (a component type
  that has a parent belongs to that entity: it may be set only on that entity, its
  children and their instances).
- **Rights:** `write` lists the roles that may write a component. The built-in role
  `core` writes the first entries and cannot be used by any script.
- **Private values:** a component type marked `local_ai = true` reads as nothing while
  the world is shown to a reader that may not see it (`World::hide_private`), for example
  an AI in the cloud.
- **Limits:** 200 000 entities, 2 000 000 values, names up to 64 characters, text up
  to 4000, code up to 64 KB.

## The code (`src/host.rs`)

The code of an entity lives in its component `src` and returns a function. Four ways to
use it, all by naming convention:

| Entity | What it is | Runs |
|---|---|---|
| `op.<name>` | an **operation**: what a role may do (`roles`, `params`, `desc`) | when a role calls `host.run` |
| `sys.<name>` with `system = true` | a **rule**: runs as the role `system`, in the order of `ord` | after every operation |
| `view.<name>` | a **view**: read-only, gives back a table (as JSON) | when asked |
| any other `src` | shared code, reached with `ecs.call` | when called |

**An operation and its rules are one piece.** If anything in it fails, everything it did is
undone, and the journal records why. Arguments are checked before anything runs
(`id`, `int`, `text`, `long`).

**The rights of a role hold inside code too.** If a clerk's operation tries to write a
component the clerk may not write, the run fails with `no_right`.

**The sandbox** is Lua 5.4 with only the libraries `table`, `string`, `math` and `utf8`.
There is no `io`, `os`, `require`, `load`, `pcall` or `getmetatable`, and `setmetatable`
refuses a metatable with `__gc` (a finalizer would run later, outside its run). Lua
patterns (`string.find`, `match`, `gsub`) are removed because they can run for minutes
inside one call; use `string.find_plain`. Each run has a time budget (250 ms by default,
set it with `budget.set_run_ms`) and a memory limit (16 MB). Nothing a script keeps in a
global variable survives the run: every run starts from fresh copies.

**The Lua API** is one table, `ecs`:

| Call | What it does |
|---|---|
| `ecs.get(e, c)` | the value, from the entity or its prefab chain |
| `ecs.own(e, c)` | only the entity's own value |
| `ecs.set(e, c, v)` | write; raises on any refusal and the run is undone |
| `ecs.del(e, c)` | remove an own value |
| `ecs.kill(e)` | remove an entity |
| `ecs.spawn(id, {comp = value, ...})` | create an entity |
| `ecs.instantiate(prefab, id)` | make `id` (and its children) an instance of a prefab |
| `ecs.with(c [, v])` | sorted entities that have component `c` (equal to `v`) |
| `ecs.children(e)`, `ecs.exists(e)`, `ecs.comps(e)`, `ecs.kind(c)` | look around |
| `ecs.call(entity, arg)` | run another entity's code (read-only at the top, same rights) |
| `ecs.history(e, n)`, `ecs.journal(n)` | the last changes (n at most 50) |
| `ecs.role()` | the role this run acts as |

## Your own API (for an engine, an ERP, anything)

A program built on Lust adds to what scripts see, before or between runs:

- `host.add_global(name, value)`: a new global, for example a table `screen` with
  functions, as LÖVE has `love.graphics`.
- `host.add_to_ecs(name, function)`: one more function in the table `ecs`.
- `host.lua()`: the Lua state, to make those functions (Lust re-exports the binding as
  `lust::mlua`). The program keeps its own data there too, out of the world and out of the
  journal: `lua.set_app_data(...)`, read in a function with `lua.app_data_ref::<T>()`.
- `Host::on_refusal(hook)`: what happens after a refused or failed operation (for example
  counting refusals per role).
- `World::overlay`: values a reader sees instead of the stored ones while the world is
  read-only (for example several screens drawn from one world).

What is added is seen by every later run. A table is copied fresh for each run, so one run
cannot change what the next one gets, and names that exist (`ecs`, the libraries, the
calls of `ecs`) cannot be replaced. An added function that changes the world should do it
through `World::set`, so the rights and the journal hold. `examples/engine.rs` is a tiny
engine of this kind: `screen.print`, `ecs.now` and a game loop.

## The journal (`src/store.rs`)

Every change is one entry: who (role), in which operation, what was changed, the new value,
the hash of the previous entry and its own hash (SHA-256). The entries form a chain.

- `Store::append` writes new entries to a file, one JSON line each.
- `World::replay` rebuilds the world from the entries and checks the whole chain. A
  changed, cut or reordered entry is found.
- **What the chain does not do:** it has no secret. Whoever can write the file can rewrite
  it from some entry on and compute the hashes again. To notice that, keep the hash of the
  last entry (`World::head`) somewhere else, or sign it.
- A half-written last line is dropped with a warning. An old journal is never
  overwritten: `store::archive` moves it aside.
- Refusals and rollbacks are in the journal too, as entries that change nothing.

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

**It is optional.** It is on by default. To build without it, depend on Lust with
`default-features = false`. To remove it from the source: delete `src/mcp.rs` and
`examples/door.rs`, the `mcp` lines in `src/lib.rs`, and the blocks `[features]` and
`[[example]] door` in `Cargo.toml`. Nothing else in Lust uses it.

## What Lust is not

Lust has no window, no graphics, no network and no pages: those belong to the program
built on it, through its own API. It has no prompt format for an AI either.

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
