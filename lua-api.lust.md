# The Lua API of Lust

Every function a Lua file of a Lust program can call, in one place. The short form the AI reads
is `lust_guide` (the constant `GUIDE` in `src/dev.rs`); the details of the solver are in
`SOLVER-SPEC.ru.md`. This file is the full reference.

Words used below:

- **e**: an entity, given as its **name** (`'ball'`) or its **number** (`ecs.id('ball')`).
  A number is faster for `num` components.
- **c**: a component, given as its **name** (`'x'`) or, for a `num` component, its **column
  number** (`ecs.col('x')`).
- **num component**: a number in a fast dense column. The solver and the bulk calls (`query`,
  `write`, `fill`, ...) work only on these.
- **raises**: the call fails with an error; the run stops (and is undone when the spec of the
  world says `undo`).

## 1. Where code lives

| File | What it is | Runs | May write |
|---|---|---|---|
| `world.lua` | the start: spec, types, entities | at the start, and what changed on each edit | |
| `sys/<n>.lua` | `return function(dt) ... end` | every tick, in the order of `ord`, then by name | yes |
| `draw/<n>.lua` | `return function() ... end` | after the ticks of a frame | no (read only) |
| `op/<n>.lua` | `return function(p) ... end`, `p` = its parameters | when an operation is run | yes |
| `rule/<n>.lua` | `return function() ... end` | after every operation | yes |
| `view/<n>.lua` | `return function(arg) return {...} end` | when asked | no |
| `lib/<n>.lua` | `local M = {} ... return function() return M end` | through `ecs.call('lib.<n>')` | as its caller |
| `test/<n>.lua` | driver code (section 8) on a fresh world | `lust test`, `lust_test` | through the driver |

`<n>` is `[a-z0-9_]`. `sys/ball.lua` is the entity `sys.ball`; errors name the file and line.

In a `game` world (`keep_lua`) the top of a code file runs once, as its load, and again after
each change of the file: keep state in components, not in Lua locals, and do not spawn there.

### world.lua

```lua
return {
  spec = 'game',          -- 'game' or 'ledger', or {journal = .., roles = .., undo = .., keep_lua = ..}
  types = {
    x = 'num',                                   -- a fast number column
    name = 'text',
    stock = {kind = 'int', write = 'clerk system'},   -- ledger: the roles that may write it
  },
  entities = {
    ball = {x = 50, y = 30},
    ['sys.ball'] = {ord = 2},                    -- order of systems and drawings
    ['op.sell'] = {params = 'item:id n:int', roles = 'clerk', desc = 'sells n'},
    ['role.clerk'] = {desc = 'sells'},
  },
  spawn = { {x = 1}, {x = 2} },                  -- nameless entities #1, #2
}
```

Kinds: `num`, `int`, `bool`, `text`, `ref` (an existing entity), `name`, `names` (words), `code`.
Parameter kinds of an operation: `id`, `text`, `int`, `long`; `name?` is optional.

| | `game` | `ledger` |
|---|---|---|
| journal (hash chain of every change) | no | yes |
| roles with rights | no | yes |
| a failed run is undone | no | yes |
| Lua kept between runs (`keep_lua`) | yes | no (a fresh copy every run) |
| memory | 64 MB | 16 MB |

`world.lua` is Lua: loops may build entities. An edit applies what changed; a new `spec` or
`spawn` needs a restart.

## 2. The Lua that is there

Libraries: `string`, `table`, `math`, `utf8` and the base functions. Removed: `io`, `os`,
`require`, `load`, `loadstring`, `dofile`, `loadfile`, `print`, `pcall`, `xpcall`,
`collectgarbage`, `getmetatable`, and the pattern functions `string.find`, `string.match`,
`string.gmatch`, `string.gsub`.

| Call | What it does |
|---|---|
| `string.find_plain(s, sub)` | the position of `sub` in `s` (plain text, no patterns), or nil |
| `string.rep(s, n [, sep])` | as in Lua, but the result is capped in size |
| `setmetatable(t, mt)` | as in Lua, but a metatable with `__gc` is refused |
| `error(msg, 0)` | fails the run with `msg` |

Every run has a time and a memory budget; a run that spends it fails.

Numbers of `num` components are floats: `'Score: ' .. n` shows `3.0`; write
`string.format('%d', n)`. `gfx.text` shows `3`.

## 3. `ecs`: the world

### Values one by one

| Call | Returns | Notes |
|---|---|---|
| `ecs.get(e, c)` | `value, from` | the own value, or the one of the `is_a` (prefab) chain; `from` names the entity it came from. nil when there is none |
| `ecs.own(e, c)` | `value` | the own value only |
| `ecs.set(e, c, v)` | | raises on any refusal (wrong kind, no right, limit). To remove a value use `ecs.del` |
| `ecs.del(e, c)` | | removes an own value |
| `ecs.exists(e)` | `bool` | |
| `ecs.comps(e)` | `{names}` | the names of its own components |
| `ecs.kind(c)` | `'num'`, `'int'`, ... or nil | the kind of a component type |

About 0.1 µs a call. For thousands of entities use the bulk calls below.

### Entities

| Call | Returns | Notes |
|---|---|---|
| `ecs.spawn({c = v, ...})` | number | a nameless entity, called `#1`, `#2`, ... (a name is never given twice) |
| `ecs.spawn(name, {c = v, ...})` | number | a named entity. `is_a` and `parent` are set first |
| `ecs.kill(e)` | | removes the entity |
| `ecs.instantiate(prefab, id)` | | `id` and its children become instances of `prefab` and its children |
| `ecs.id(e)` | number | the number: index and generation, so an old number never reaches a new entity |
| `ecs.name(e)` | text | `'#<n>'` for a nameless one |
| `ecs.with(c [, v])` | `{names}` | sorted names of the entities with an own value of `c` (equal to `v`) |
| `ecs.children(e)` | `{names}` | entities whose `parent` is `e` |
| `ecs.count([c])` | number | all entities, or those with an own value of `c` |

A number kept in a `num` column (for example `node_a`) is a float: turn it back with
`math.tointeger(v)` before `ecs.get(v, ...)`.

### Whole `num` columns at once

| Call | Returns | Notes |
|---|---|---|
| `ecs.col(c)` | number | the column number of a `num` component |
| `ecs.query(c1, c2, ...)` | `n, ids, v1, v2, ...` | the entities that have all these components, one new table per component, same order |
| `ecs.read(c, ids, vals)` | `n` | one column into two tables the script keeps (no new garbage); entries after `n` are old |
| `ecs.write(c, ids, vals [, n])` | | the values back in one call (rights and undo hold) |
| `ecs.add(dst, src [, k])` | | `dst += src * k` (k = 1) for every entity that has both |
| `ecs.scale(c, k)` | | `c *= k` |
| `ecs.fill(c, v)` | | `c = v` |
| `ecs.clamp(c, lo, hi)` | | `lo <= c <= hi` |
| `ecs.sum(c)` | `sum, n` | |

A loop of 1000 entities through `query`, Lua and `write` takes under 1 ms.

### Code, time, rights

| Call | Returns | Notes |
|---|---|---|
| `ecs.call(entity, arg)` | what that code returns | shared code (`lib.*`, views). Runs with the rights of the caller |
| `ecs.tick()` | number | how many ticks the world has had |
| `ecs.role()` | text | the role this run acts as |
| `ecs.run(name [, arg])` | | a Rust system the program added (`Host::add_system`) |
| `ecs.commit({key = value})` | | a result for the program (to give to another world). One at a time |
| `ecs.history(e, n)` | `{{seq, role, op, comp, value}}` | last `n` (≤ 50) changes of one entity, newest first; empty without a journal |
| `ecs.journal(n)` | `{{role, op, entity, comp, value}}` | last `n` (≤ 50) entries of the journal |
| `ecs.quit()` | | ends the game once the page has heard it (in `lust dev` it only pauses). Not from `draw/` |

### Saves

| Call | Returns | Notes |
|---|---|---|
| `ecs.save(slot)` | | after this tick |
| `ecs.load(slot)` | `false` when there is no such slot | after this tick; refused (said on the status line) when a type is gone or has another kind |
| `ecs.slots()` | `{names}` | |
| `ecs.erase(slot)` | | |

A slot is 1..32 of `a-z 0-9 _ -`; at most 20 slots of 4 MB. Only data is saved, never code.
Saves live in the user's data folder (`%APPDATA%\lust\<game>` on Windows); in tests in memory.

## 4. `input`

| Call | Returns |
|---|---|
| `input.down(key)` | `true` while the key is held |
| `input.pressed(key)` | `true` in the tick it went down |
| `input.mouse()` | `x, y, down` in the units of `gfx.size` (0, 0 before the first move) |
| `input.mic()` | the loudness of the microphone, 0..1 (after `gfx.mic(true)`) |

Keys: `up down left right space enter escape shift ctrl alt tab backspace f1`, `a`..`z`,
`0`..`9`, and `mouse` (the button; `input.pressed('mouse')` is a click). Letters and digits are
named by the key itself, whatever the keyboard layout (the key R is `r` on a Russian layout too).

## 5. `gfx`: drawing (only in `draw/` files)

| Call | Notes |
|---|---|
| `gfx.size(w, h)` | the screen in your units (default 800 × 600, y grows down), 1..10000 |
| `gfx.clear(color)` | |
| `gfx.rect(x, y, w, h, color)` | filled |
| `gfx.circle(x, y, r, color)` | filled |
| `gfx.line(x1, y1, x2, y2, width, color)` | |
| `gfx.text(x, y, size, color, text)` | `y` is the top; text, number or boolean; one line |
| `gfx.measure(text, size)` → `w, h` | the box of a text; works anywhere, also in `sys/` |
| `gfx.mic(true)` | asks the browser for the microphone; only the loudness reaches the world |

Colors: `'#rgb'`, `'#rrggbb'` or a CSS name. Text is measured as monospace, at the width the
browser reports.

## 6. `lib.ui`: widgets (built into every project)

`local ui = ecs.call('lib.ui')`. A widget is a plain table with `x, y, w, h`; build the same
widgets in the `sys/` file (clicks) and in the `draw/` file (drawing).

| Call | Notes |
|---|---|
| `ui.button(text, x, y, {w, h, size, pad, enabled, id})` → `b` | as big as its text needs unless `w`, `h` |
| `ui.inside(b, px, py)`, `ui.hover(b)`, `ui.down(b)` | |
| `ui.clicked(b)` | clicked in this tick (in `sys/`) |
| `ui.draw(b, focused)` | in `draw/` |
| `ui.label(text, x, y, {align, size, color})` → `w, h` | `align`: `'left'`, `'center'`, `'right'` |
| `ui.panel(x, y, w, h [, color, border])` | |
| `ui.measure(text, size)` → `w, h` | |
| `ui.wrap(text, w, size)` → lines | |
| `ui.paragraph(text, x, y, w, {size, color})` → height | wraps to `w` |
| `ui.stack(list, cx, top, gap)` → bottom | one under another, centered, one width |
| `ui.row(list, cx, top, gap)` → right | side by side, centered |
| `ui.menu(items, cx, top, {size, pad, w, gap})` → list | an item is a text or `{text = .., enabled = false}` |
| `ui.update_menu(list, sel, {wrap})` → `sel, chosen, edge` | in `sys/`: arrows, mouse, enter, space, click |
| `ui.draw_menu(list, sel)` | in `draw/` |
| `ui.theme` | `text, dim, back, button, hover, border, focus, size, pad` |

`examples/menu` is a template for a game with menu, play, options and about screens.

## 7. The solver (`ecs.*`, `src/solver`)

An abstract XPBD solver: **points**, **links** between points, **barriers**. It knows nothing
of bridges, wheels or strength; every number (mass, stiffness, friction, damping) comes from the
components the program sets. It reads `num` columns, works on plain arrays and writes back.

### Columns

Point (needs `x, y, old_x, old_y, inv_mass`):

| Column | Default | Meaning |
|---|---|---|
| `x`, `y` | needed | position |
| `old_x`, `old_y` | needed | position one substep ago (speed = the difference) |
| `inv_mass` | needed | 1 / mass; `0` is fixed |
| `radius` | 0 | for collisions; 0 never collides |
| `layer`, `mask` | 1, 1 | bits: what it is, what it hits |
| `group` | 0 | points and edges of one non-zero group never collide with each other (one body) |
| `damping` | the call's | share of speed kept per substep (0..1) |
| `bend_damping` | the call's | share of turning between its links taken per substep (`damp_angular`) |
| `spin` | 0 | turning speed as a circle, rad/s, plus is clockwise on screen; friction drives with it |
| `inertia` | 0 | moment of inertia; with it friction changes `spin` (spins up, brakes, slips) and the solver writes it back |
| `contact`, `contact_nx`, `contact_ny` | written | 1 when pushed out this substep, and the normal |

Link (needs `node_a, node_b, rest_len`; the ends are `ecs.id` numbers):

| Column | Default | Meaning |
|---|---|---|
| `rest_len` | needed | the length it keeps |
| `compliance` | 0 | softness; 0 is rigid |
| `mode` | 0 | 0 keeps the length, 1 not longer (a rope), 2 not shorter (a strut) |
| `ang_damping` | the call's | share of its turning taken per substep |
| `spring_damping` | 0 | a damper on a soft link, seconds (c / k); critical for a mass m: 2 * sqrt(m * compliance) |
| `lambda` | written | the multiplier of the last call |
| `force_max` | written | the biggest force since the program zeroed it (`ecs.fill('force_max', 0)`) |
| `layer` (+ `friction`, `radius`, `group`) | | with a layer the link is also an **edge**: a movable barrier, a capsule of `radius` |
| `catenary_a` (+ `catenary_x0`, `catenary_y0`, `sag`) | | with these it is a **cable**: a curve of its own |
| `linear_density` | 0 | weight per length of a cable (loads its ends) |
| `min_slack` | 0.001 | a cable is never quite straight (share of the chord) |
| `tension` | written | the pull of a cable (`update_catenary`) |

Segment (a static barrier; needs `x1, y1, x2, y2`): `friction` (0), `layer` (1),
`radius` (0, a capsule).

`friction` (of a segment or an edge) is the coefficient of Coulomb friction: the friction of a
contact is at most `friction` times the push that keeps the point out.

### Functions

| Call | Returns | What it does |
|---|---|---|
| `ecs.solve_tick(dt, substeps, gx, gy, damping [, spin, bend])` | steps | **the whole tick in one call**: each substep integrate, cable load, links, turn friction, edges, segments. Reads and writes the world once; bit for bit the same as the calls below one by one |
| `ecs.integrate_verlet(dt, gx, gy, damping)` | moved | inertia and gravity for every free point; clears `contact` |
| `ecs.apply_catenary_load(dt, gy)` | cables | the weight of each cable onto its ends |
| `ecs.solve_distance(dt, iterations)` | links | keeps the links (XPBD); updates `force_max` |
| `ecs.damp_angular(spin, bend)` | changed | friction against turning: `spin` of each link, `bend` between the links of a joint. Keeps momentum, only loses energy |
| `ecs.project_edges(iterations [, dt])` | pushes | points out of edges; momentum kept, no bounce; with `dt` the `spin` of a point drives it |
| `ecs.project_segments(iterations [, dt])` | pushes | points out of static segments, with friction |
| `ecs.project_points(iterations)` | pushes | circles out of circles (masks both ways; linked pairs skip) |
| `ecs.update_catenary(gy)` | cables | the curve of each cable, its `sag` and `tension` |
| `ecs.sample_catenary(id, t)` | `x, y` | a point on the curve of a cable, t = 0..1 |
| `ecs.query_point(x, y, max_dist [, mask])` | `id, dist` or nil | the nearest point |
| `ecs.raycast_segments(x, y, dx, dy, max_dist [, mask])` | `id, hx, hy, nx, ny, t` or nil | the first segment on a ray |
| `ecs.path(from, to [, cost])` | `ids, cost` or nil | the cheapest way over links (A* / Dijkstra) |
| `ecs.distances(from [, cost])` | `ids, costs` | the cost to every reachable point |

`cost`: 0 the length now (default), 1 the rest length, 2 one per link. `iterations`: 1..64.
`substeps`: 1..256. A bad argument raises `bad_arg: <name>`; a `draw/` file cannot call the
writing ones (`readonly`).

### A tick, as the example does it

```lua
return function(dt)
  ecs.fill('force_max', 0)
  ecs.solve_tick(dt, 16, 0, 980, 0.999, 0.00005, 0.0005)
  ecs.update_catenary(980)
  -- the program reads force_max / tension and compares with its own strength column
end
```

Layer convention of the example game: ground 1, semi-solid 2, road edges 4, joints layer 8
mask 1, wheels layer 16 mask 23, mouse marker layer 32 mask 0.

Speed (`cargo test --release solver::bench -- --ignored --nocapture`, `solve_tick`, 16
substeps): 905 links 1.8 ms a tick, 9 305 links 13.5 ms, 93 305 links 123 ms. There is no grid
for collisions yet: many wheels on a long road are tested each against each edge.

## 8. The driver (`test/*.lua`, `lust_try`)

Lua of its own that runs a world from outside. It has no `ecs`, `input`, `gfx`.

| Call | What it does |
|---|---|
| `step([n [, dt]])` | `n` ticks (1), `dt` (1/60) |
| `get(e, c)`, `set(e, c, v)` | a value (`set(e, c, '')` deletes it) |
| `ents(c)` | names of entities with `c` |
| `tick()` | ticks so far |
| `keys(k, ...)` | held from now on; `keys()` lets go |
| `press(k)` | down on the next tick |
| `mouse(x, y, down)` | the button going down is a click |
| `mic(level)`, `glyph(w)` | the microphone loudness; the width of a character as a share of its size (0.6) |
| `draw()` | the frame as text lines (`rect ...`, `circle ...`, `line ...`, `text ...`) |
| `events()` | what the world told the display since the last call, e.g. `{'quit'}` |
| `run(role, op, {k = v})`, `view(name [, arg])` | an operation, a view |
| `ms()` | milliseconds since the start |
| `print(...)` | the answer of `lust_try` |

A click: `mouse(x, y, true) step() mouse(x, y, false) step()`. `assert()` fails a test.

## 9. Limits

- A world: `game` up to 1 000 000 entities and 4 000 000 values; `ledger` 200 000 and 2 000 000.
- `ecs.call` depth is limited; `history` and `journal` give at most 50 entries.
- At most 16 events (`quit`) a tick; a text on the screen is clipped.
- A file of the program: at most 64 KB, 2000 files.
