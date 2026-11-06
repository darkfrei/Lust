# Worlds

English | [Русский](worlds.ru.md)

A program on Lust has as many worlds as it needs, and **every world is the same kind of
thing**: entities, components, its own Lua sandbox, its own code. What differs is what a world
**keeps**, and that is its `Spec`:

| Setting | What it means | `Spec::ledger()` | `Spec::game()` |
|---|---|---|---|
| `journal` | every change is an entry of a hash chain (the journal module) | yes | no |
| `roles` | rights: who may write each component, who may run each operation | yes | no |
| `undo` | a failed operation or tick is undone as a whole | yes | no |
| `keep_lua` | Lua globals live from run to run, as in a game loop; the top of a code chunk runs once, as its load | no | yes |
| `memory` | the memory of the world's Lua | 16 MB | 64 MB |
| `entities_max`, `values_max` | limits | 200 000, 2 000 000 | 1 000 000, 4 000 000 |

The two presets are only starting points; any mix is allowed:

- **An arcade game** (Arkanoid): `Spec::game()`. Nothing to keep but a save.
- **A turn-based strategy** (Heroes III): `Spec::ledger()` for replays and hot-seat players with
  their own rights, or `Spec { journal: false, ..Spec::ledger() }` if no replay is needed.
- **An ERP**: `Spec::ledger()`.
- **A game that runs but must not lose a failed tick**: `Spec { undo: true, ..Spec::game() }`.

```rust
let kingdom = Host::new()?;                          // Spec::ledger()
let battle = Host::with_spec(Spec::game())?;         // made now, dropped after the battle
let menu = Host::with_spec(Spec::game())?;           // the interface, its own world
```

## The journal is a module

`src/journal.rs` is the hash chain: `Entry`, `entry_hash`, `verify`. A world writes entries only
when its spec says `journal`. A world without one still has everything else: values checked by
kind, undo of a failed run (if `undo`), snapshots for saving. `store.rs` writes journals and
snapshots to disk.

## Two ways to store a component

A component type is an entity; its `kind` says how its values are kept:

- **text kinds** (`text`, `code`, `int`, `bool`, `ref`, `name`, `names`): checked on every write,
  in the journal when the world keeps one. For rules, names, decisions, money.
- **`num`**: a number (`f64`) in a dense column (a sparse set: the values of one component lie
  one after another, so a pass over a column is a pass over memory). Never in the journal; undone
  with the rest of a failed run (if `undo`); saved in snapshots; rights hold (if `roles`). For
  what changes every tick: positions, speeds, health.

Both live in the same world. A turn-based game can keep gold as `int` (in the journal) and the
position of a unit on the screen as `num`.

## Entities by name or by number

Every entity has a **name** and a **number**. `ecs.spawn('ball', {...})` makes a named entity;
`ecs.spawn({...})` makes one without a name, which is called `#1`, `#2`, ... (never given twice
in one world). `ecs.id(e)` gives the number: one integer, the index and a generation, so a number
given again after an entity is gone does not reach the new entity through an old id. Every call
of `ecs` takes a name or a number; a number is faster for `num` components. `ecs.col(c)` gives a
column number, faster than the name of the component.

## The code of a world

| Entity | What it is | Runs |
|---|---|---|
| `op.<name>` | an **operation** (`roles`, `params`) | `host.run(role, op, args)`; input, a move, a command |
| anything with `system = true` | a **rule** | after every operation |
| anything with `tick = true` | a **tick** | `host.tick(dt)`, in the order of `ord`, with `dt` |
| `view.<name>` | a **view**: read-only, gives back a table | `host.view` |
| any other `src` | shared code | `ecs.call` |

`dt` is whatever the program passes: the time since the last frame, or a fixed step.
`clock::Clock` gives both:

- `Clock::variable(max_dt)`: one tick per frame with the real frame time (`dt` floats). Smooth;
  what most games that need not replay exactly want. A frame that took too long gives at most
  `max_dt`, so the game does not jump after a pause.
- `Clock::fixed(step, max_steps)`: equal steps; a frame runs as many as have piled up, at most
  `max_steps`, and drops the rest, so one slow frame does not make the next one slower too. For
  physics that must stay stable. `Clock::alpha` tells how far the frame is between two steps.

**With `keep_lua` the top of a code chunk is its load** (as `love.load`): it runs once (again
when the code changes), with the rights of the run that first needs it, so it may spawn the first
entities and set globals; its function is kept. **Without `keep_lua`** every run starts from a
fresh copy of the globals and the top of a chunk is read-only: nothing a script keeps survives a
run. That is what the sandbox of a ledger stands on.

**Rust systems:** `host.add_system(name, f)` with `f: FnMut(&mut World, f64)`, run from Lua as
`ecs.run(name, arg)`. The program's own code over whole columns (`World::column_mut`); it writes
past rights, undo and limits.

## The Lua API: the table `ecs` in every world

Everything that was there before (`get`, `own`, `set`, `del`, `kill`, `spawn`, `instantiate`,
`with`, `children`, `exists`, `comps`, `kind`, `call`, `history`, `journal`, `role`), now taking a
name or a number for an entity, and a name or a column number for a `num` component. New:

| Call | What it does |
|---|---|
| `ecs.spawn({c = v, ...})` | an entity without a name, its number |
| `ecs.id(e)`, `ecs.name(e)` | the number, the name |
| `ecs.col(c)` | the column number of a `num` component |
| `ecs.count([c])` | entities, or entities with an own value of `c` |
| `ecs.query(c, ...)` | `n, ids, vals, ...`: the entities that have all these `num` components, a new table per component, one call |
| `ecs.read(c, ids, vals)` | `n`: one column into two tables the script keeps (no new garbage) |
| `ecs.write(c, ids, vals [, n])` | the values back, one call (rights and undo hold) |
| `ecs.add(dst, src [, k])` | `dst += src * k` for every entity that has both |
| `ecs.scale(c, k)`, `ecs.fill(c, v)`, `ecs.clamp(c, lo, hi)`, `ecs.sum(c)` | a whole column |
| `ecs.run(name [, arg])` | a Rust system of the program |
| `ecs.commit({key = value, ...})` | a result for the program (see below) |
| `ecs.tick()` | how many ticks the world has had |

`history` and `journal` give nothing in a world without a journal.

## From one world to another

Worlds are independent. The one way between them is `link.rs`: a world computes something and
commits a result (`ecs.commit{...}`, a few named values); the program takes it
(`host.take_result()`) and runs it in another world **as one operation of a role**
(`link::commit_to`): checked like any other operation, written to that world's journal between
`from world <name>` and `end world <name> ok|failed` if it keeps one. Any world can give a result
to any other; nothing in the direction is fixed.

A `Seed` remembers what a world read from another when it was made. While it computed, the other
may have changed (the player sold a unit, the king sent reinforcements); then `commit_to` refuses
the result as stale (`stale: red.units changed while world battle computed`). The program decides
what then: make the world again from the new values, or show the person.

`examples/battle.rs`: a kingdom (`Spec::ledger`) and a battle (`Spec::game`) for each fight. A
battle of about 1100 soldiers takes 3308 ticks; the kingdom's journal gets 4 entries.

## Saving

`World::snapshot` holds every value, text and `num`, and the names of entities without a name.
`World::from_snapshot_spec(spec, &snap, entries)` loads it into a world of any spec (a ledger also
takes the journal entries written after it). The globals of a world's Lua are not saved: what must
survive is in components. After loading, the top of each code chunk runs again (its load), so a
world finds its column numbers and its globals again.

`examples/arkanoid.rs` saves in the middle of a game, drops the world, loads it into a new one
and plays on.

## 60 to 120 frames without a stall

A stall is about the **worst** frame: at 60 frames a second a frame has 16.6 ms, at 120 it has
8.3 ms. `examples/frames.rs` prints the mean, p99 (99 of 100 ticks are faster) and the slowest.
The numbers are three runs on a shared cloud machine with 2 cores, release build; the slowest
ones jump between runs there. Measure on your own computer.

| What (one tick or operation) | mean | p99 | slowest |
|---|---|---|---|
| ledger: one empty operation | 0.03 ms | 0.06 ms | 0.2 to 0.3 ms |
| ledger: 10 000 entities by name, kind `int` (journal) | 32 to 36 ms | 44 to 52 ms | 44 to 52 ms |
| ledger: the same, kind `num` (rights and undo, no journal) | 11.3 to 11.6 ms | 12 to 18 ms | 12 to 18 ms |
| ledger: a result of another world committed | 0.03 ms | 0.06 ms | 0.13 ms |
| game, 100 entities, one by one from Lua | 0.03 ms | 0.05 ms | 0.25 ms |
| game, 10 000, one by one, column names | 4.8 ms | 6.8 to 8.9 ms | 10 to 16 ms |
| game, 10 000, one by one, column numbers | 2.8 to 2.9 ms | 4.1 to 5.3 ms | 5.5 to 9.2 ms |
| game, 10 000, one by one, with undo | 3.2 to 3.4 ms | 4.3 to 5.7 ms | 6 to 6.8 ms |
| game, 10 000, `query`, a Lua loop, `write` | 0.88 to 1.0 ms | 0.95 to 2.3 ms | 1.0 to 5.1 ms |
| game, 10 000, plain Lua tables (no world at all) | 0.48 to 0.50 ms | 0.52 to 0.82 ms | 0.55 to 2.7 ms |
| game, 10 000, `ecs.add` or a Rust system | 0.006 ms | 0.01 ms | 0.03 ms |
| game, 10 000, a small new table per entity (garbage) | 1.5 ms | 1.7 to 3.9 ms | 2.4 to 5.1 ms |
| make a world, 1000 entities, two ticks, drop it | 1.6 to 1.7 ms | 1.8 to 2.2 ms | 1.9 to 3.7 ms |
| `examples/arkanoid.rs`, a frame | 0.012 ms | 0.03 ms | 0.18 ms |
| `examples/battle.rs`, a frame (about 1100 soldiers, 4 ticks) | 0.6 to 0.8 ms | | 2 to 4 ms |

What this means for a game:

1. **Few entities, many ticks: easy.** 100 entities one by one: 0.03 ms a tick.
2. **10 000 entities at 120 frames: yes, but not one by one.** One by one from Lua is about
   3 ms a tick on the average, with stalls up to 9 ms: fits 60 frames, is tight at 120. With
   `ecs.query`, a loop in Lua and `ecs.write` it is under 1 ms, close to plain Lua tables. With
   `ecs.add` or a Rust system it is 0.006 ms. So: **the same arithmetic for every entity goes to
   Rust; logic that branches per entity goes to Lua over `query`; `get`/`set` one by one is for
   the few entities that are special** (the player, a boss, a button).
3. **Numbers, not names**, when one by one (`ecs.col` once, `ecs.id` once).
4. **Garbage costs.** A new small table per entity per tick costs about 0.5 ms more on 10 000
   and raises the worst ticks. Keep tables from tick to tick (`ecs.read` into the same tables).
   A world with `keep_lua` uses Lua's generational collector, which takes short-lived garbage in
   short steps; `host.gc_step()` collects a step at the end of a frame that has time left.
5. **In a ledger, `num` instead of `int`** for values that change often: about three times faster,
   because they are not in the journal and not checked as text.
6. **Making a world costs about 1.6 ms** with 1000 entities: fine between frames, not many in one.

Also faster now, in every world: **code is compiled once** and kept until its text changes
(0.25 ms a run for 20 KB of code before, 0.04 ms now); **a whole column crosses** between Rust and
Lua in one guarded call (`query`, `read`, `write`), not one per value.

## Not done, and what is still a problem

- **One by one is slowish.** About 90 ns for every `ecs.get` or `ecs.set` from Lua: the price of a
  call from Lua into Rust through mlua, plus the name-or-number check. Not less without a JIT
  (LuaJIT) or raw C functions.
- **Threads.** A world runs on the thread of the program; its types use `Rc`. A world on a thread
  of its own (a frame that never waits for a battle) needs mlua's `send` feature and messages
  between worlds instead of shared references. That is the right shape for many independent
  worlds; it is a larger change.
- **The journal is a module of the source, not of the build.** A world without a journal carries
  no journal at run time, but `journal.rs` and `sha2` are always compiled (snapshots use `sha2`
  too). Making it a Cargo feature like `mcp` is possible.
- **Disk in the frame.** Journal and snapshots are written on the thread that calls them; a game
  should call them between frames.
- **An AI sees one world.** The MCP door (`mcp::WorldTools`) is made for one host; a program with
  several worlds chooses which one its AI sees.
- **The same bits everywhere** are not promised (`f64`, `math.sin`, the order of `pairs`). A
  ledger does not need them, because it keeps results. A program that wants to replay a game world
  exactly must keep to a fixed step, whole numbers and a fixed order itself.
- **A Rust system writes past rights, undo and limits.** It is the program's own code; a failed
  tick in a world with undo does not undo what a Rust system did in it.
