# The Lua API of Lust

This is the full guide to everything a Lua file of a Lust program can do. It is written for a
person who sees Lust for the first time: every term is explained, and almost every function has
an example **with the result it really gave**. The examples were run on Lust itself while this
guide was written; a result such as `=> 50.0` is what the run printed.

Other documents: the short form the AI reads is `lust_guide` (the constant `GUIDE` in
`src/dev.rs`); where each function lives in Rust is in `lua-api.rust.md`; the mathematics of the
solver is in `SOLVER-SPEC.ru.md`.

Contents: 1 The idea and a first program · 2 Where code lives · 3 `world.lua` · 4 The Lua that is
there · 5 `ecs`: the world · 6 `input` · 7 `gfx` · 8 `lib.ui` · 9 Saves · 10 Worlds with rights
(ledger) · 11 The solver · 12 The driver and tests · 13 Error codes · 14 Limits

---

## 1. The idea and a first program

A Lust program is a **folder of Lua files**. Lust runs it; nothing has to be compiled. The
world of the program is a table of **entities** (things) and **components** (facts about a
thing):

| Word | Plain meaning | Example |
|---|---|---|
| entity | a thing, with a name | `ball` |
| component | one fact about a thing, with a name and a value | `x = 50` |
| `e` in this guide | an entity: its name `'ball'`, or its number `ecs.id('ball')` | |
| `c` in this guide | a component: its name `'x'`, or for a number component its column number `ecs.col('x')` | |
| **tick** | one step of time; the world has 60 ticks a second, so `dt` is about `1/60` | |
| **system** | a Lua file that runs on every tick (`sys/`) | |
| **drawing** | a Lua file that draws the picture (`draw/`); it may only read | |

**A first program: a ball that bounces between two walls.** Four files:

`world.lua` (the start: what exists)

```lua
return {
  spec = 'game',
  types = { x = 'num', y = 'num', vx = 'num' },   -- the kinds of the facts
  entities = {
    ball = { x = 50, y = 100, vx = 100 },          -- vx: 100 pixels a second
    ['sys.move'] = { ord = 1 },                     -- optional: the order of this system
    ['draw.ball'] = { ord = 1 },                    -- optional: the order of this drawing
  },
}
```

`sys/move.lua` (every tick: move the ball, turn it round at the walls)

```lua
return function(dt)
  local x = ecs.get('ball', 'x') + ecs.get('ball', 'vx') * dt
  if x > 250 then
    x = 250
    ecs.set('ball', 'vx', -ecs.get('ball', 'vx'))
  end
  if x < 50 then
    x = 50
    ecs.set('ball', 'vx', -ecs.get('ball', 'vx'))
  end
  ecs.set('ball', 'x', x)
end
```

`draw/ball.lua` (the picture; it only reads the world)

```lua
return function()
  gfx.size(300, 200)
  gfx.clear('#102030')
  gfx.circle(ecs.get('ball', 'x'), ecs.get('ball', 'y'), 10, 'white')
end
```

`test/bounce.lua` (a test: it runs the world without a screen)

```lua
step(60)                                  -- one second
print(get('ball', 'x'), get('ball', 'vx'))
step(60)
print(get('ball', 'x'), get('ball', 'vx'))
step(60)
print(get('ball', 'x'), get('ball', 'vx'))
print(draw())                              -- the picture as text
```

```
150	100            one second: 50 + 100 pixels
250	100            two seconds: at the right wall
151.6667	-100       the ball has turned and goes left
size 300 200
clear #102030
circle 151.67 100 10 white
```

Run it with `lust dev <folder>` (the AI builds the program through it; the page shows the
picture) or `lust test <folder>` (runs everything in `test/`).

---

## 2. Where code lives

| File | What it is | Runs | May write |
|---|---|---|---|
| `world.lua` | the start: spec, types, entities | at the start; what changed is applied on each edit | |
| `sys/<n>.lua` | `return function(dt) ... end` | every tick, in the order of its `ord`, then by name | yes |
| `draw/<n>.lua` | `return function() ... end` | after the ticks of a frame, in the order of `ord` | **no** (read only) |
| `op/<n>.lua` | `return function(p) ... end`; `p` = its parameters | when an operation is run (section 10) | yes |
| `rule/<n>.lua` | `return function() ... end` | after every operation | yes |
| `view/<n>.lua` | `return function(arg) return {...} end` | when asked | no |
| `lib/<n>.lua` | `local M = {} ... return function() return M end` | when called with `ecs.call('lib.<n>')` | as its caller |
| `test/<n>.lua` | driver code (section 12) on a fresh world | `lust test`, `lust_test` | through the driver |

`<n>` is `[a-z0-9_]`. A file `sys/ball.lua` is the entity `sys.ball`; an error message names the
file and the line. A file is an entity, so it can have components: the component `ord` (a
number) puts a system or a drawing earlier or later. **You do not have to list the files in
`world.lua`**: a `sys/` file without an entry runs too, after those with an `ord`, ordered by name
(a test run with a `sys/count.lua` and a `lib/two.lua` that `world.lua` never mentions: three ticks
add `2` each, so `n = 6`).

In a `game` world the top of a code file runs once, when the file is loaded (and again after
you change the file): keep your state in components, not in Lua local variables, and do not
create entities at the top of a file.

**A draw file that writes fails.** Drawings run possibly more than once per tick, so Lust makes
them read only:

```lua
-- draw/main.lua
return function()
  ecs.set('ball', 'x', 1)
end
```

```
=> draw/main.lua: readonly
```

**Shared code: `lib/`.** A file that two other files need goes into `lib/` and is fetched with
`ecs.call`:

```lua
-- lib/util.lua
local M = {}
function M.clamp(v, lo, hi)
  if v < lo then return lo end
  if v > hi then return hi end
  return v
end
return function() return M end
```

```lua
local u = ecs.call('lib.util')
out(u.clamp(15, 0, 10), u.clamp(-3, 0, 10))     -- out() prints; see the note below
=> 10 | 0
```

(`ecs.call` gives every caller the same table: `ecs.call('lib.util') == ecs.call('lib.util')`
is `true`.) You do not list `lib/util.lua` in `world.lua`.

> **A note on the examples.** `out(...)` in the examples stands for "show these values": the
> examples were run through a small test helper that joins the values with ` | `. In a real
> program you would put the value into a component, draw it, or `print` it from a test.

---

## 3. `world.lua`

```lua
return {
  spec = 'game',          -- 'game' or 'ledger', or a table with your own mix (see below)
  types = {
    x = 'num',                                        -- a fast number column
    name = 'text',
    stock = {kind = 'int', write = 'clerk system'},   -- ledger: the roles that may write it
  },
  entities = {
    ball = {x = 50, y = 30},
    ['sys.ball'] = {ord = 2},                         -- order of systems and drawings
    ['op.sell'] = {params = 'item:id n:int', roles = 'clerk', desc = 'sells n'},
    ['role.clerk'] = {desc = 'sells'},
  },
  spawn = { {x = 1}, {x = 2} },                       -- nameless entities #1, #2
}
```

`world.lua` is Lua, so a loop may build entities:

```lua
local entities = {}
for i = 1, 10 do entities['tree_' .. i] = {x = i * 20, y = 100} end
return { spec = 'game', types = {x = 'num', y = 'num'}, entities = entities }
```

An edit of `world.lua` while the program runs applies what changed; a new `spec` or `spawn`
needs a restart.

### Kinds of a component

| Kind | What it holds | `ecs.get` gives | Setting a wrong value |
|---|---|---|---|
| `num` | a number, in a fast column; **the solver and the bulk calls use only these** | float, `50.0` | `not_a_number`, `not_finite` (nan, inf) |
| `int` | a whole number | integer, `7` | `bad_int: hp` |
| `bool` | true / false | boolean | |
| `text` | text up to 4000 characters, not empty | string | `bad_text: label` |
| `ref` | the name of an existing entity | its name | `no_entity: nobody` |
| `name` | a name of an entity | | |
| `names` | words separated by spaces | a Lua list `{'red', 'fast'}` | |
| `code` | Lua source | | `bad_code: ...` |

A fact of the kind `num` is stored as a float, so printing it shows `50.0`. Turn it into text
with `string.format('%d', n)` when you want `50`. (`gfx.text` already prints `50`.)

```lua
ecs.set('orc', 'hp', 7)                       -- hp is an int
out(ecs.get('orc', 'hp'), math.type(ecs.get('orc', 'hp')))
=> 7 | integer

out('Score: ' .. ecs.get('ball', 'x'), string.format('%d', ecs.get('ball', 'x')))
=> Score: 75.0 | 75
```

Things that surprise people, all checked:

- **A type is an entity too.** `x` and `label` exist as entities with the component `kind`:
  `ecs.get('x', 'kind')` is `'num'`. So `ecs.count()` counts them as well (30 in the small world of these examples).
- **`ecs.set` on a new name makes the entity.** `ecs.set('nobody', 'x', 1)` is not an error: now
  `ecs.exists('nobody')` is `true` and its `x` is `1.0`. A typo in a name therefore creates a
  new, empty entity. Setting a component that has no type fails: `ecs.set('ball', 'colour',
  'red')` → `no_type: colour`.
- **A kind is checked.** `ecs.set('ball', 'hp', 'ten')` → `bad_int: hp`;
  `ecs.set('ball', 'x', 'far')` → `not_a_number`; `ecs.set('ball', 'x', 0/0)` → `not_finite`.

### `spec`

`'game'` is for games, `'ledger'` for business programs with rights, a journal and undo:

| | `game` | `ledger` |
|---|---|---|
| journal (a hash chain of every change) | no | yes |
| roles with rights | no | yes |
| a failed run is undone as a whole | no | yes |
| Lua kept between runs (`keep_lua`) | yes | no (a fresh copy every run) |
| memory for Lua | 64 MB | 16 MB |

---

## 4. The Lua that is there

You have `string`, `table`, `math`, `utf8` and the base functions (`ipairs`, `pairs`, `type`,
`tostring`, `select`, `error`, ...). Removed, on purpose, so a program cannot harm the computer
or hang it: `io`, `os` (so no files and no clock), `require`, `load`, `loadstring`, `dofile`,
`loadfile`, `print` (only in test code), `pcall`, `xpcall`, `collectgarbage`, `getmetatable`,
and the pattern functions `string.find`, `string.match`, `string.gmatch`, `string.gsub`.

| Call | What it does | Example → result |
|---|---|---|
| `string.find_plain(s, sub)` | the position of `sub` in `s` (plain text, no patterns), or nil | `string.find_plain('hello world', 'wor')` → `7`; `string.find_plain('abc', 'z')` → `nil` |
| `string.rep(s, n [, sep])` | as in Lua, but the result may not be huge | `#string.rep('x', 10, ',')` → `19`; `string.rep('x', 100000000)` → `rep_too_large` |
| `setmetatable(t, mt)` | as in Lua, but a metatable with `__gc` is refused (`no_gc`) | |
| `error(msg, 0)` | fails the run with `msg` | `error('not_enough_stock', 0)` |

Why no `pcall`? Every run has a **time limit** (250 ms by default) and a memory limit. With
`pcall` a script could catch the error of the limit and go on forever. So an error always
stops the run:

```lua
while true do end
=> sys/probe.lua: time_limit
```

A loop that sums 200000 numbers is fine (`=> 20000100000`). For random numbers use your own
generator on integers (for example xorshift); there is no clock and no system randomness, so a
run can be repeated exactly.

---

## 5. `ecs`: the world

All examples below start from this world, unless said otherwise:

```lua
types    = { x='num', y='num', vx='num', label='text', hp='int', alive='bool',
             target='ref', tags='names' },
entities = { ball = {x = 50, y = 30, label = 'the ball'}, orc = {hp = 10, label = 'orc'} }
```

### 5.1 Values one by one

| Call | Returns | Notes |
|---|---|---|
| `ecs.get(e, c)` | `value, from` | the own value, or the one of the prefab chain (5.2); `from` is the name of the entity it came from. `nil` when there is none |
| `ecs.own(e, c)` | `value` | the own value only (never from a prefab) |
| `ecs.set(e, c, v)` | | writes. Any refusal raises an error and stops the run. Makes the entity when the name is new |
| `ecs.del(e, c)` | | removes an own value |
| `ecs.exists(e)` | `bool` | |
| `ecs.comps(e)` | `{names}` | the names of its own components |
| `ecs.kind(c)` | `'num'`, `'int'`, ... or nil | the kind of a component type |

```lua
out(ecs.get('ball', 'x'))                      => 50.0 | nil
out(ecs.get('ball', 'label'))                  => the ball | ball
out(tostring(ecs.get('ball', 'hp')))           => nil           -- the ball has no hp
ecs.set('ball', 'x', 75)  out(ecs.get('ball', 'x'))   => 75.0 | nil
```

**`ecs.get` gives two values.** In `out(ecs.get('ball', 'label'))` the second one (`ball`) is the
name of the entity the value came from. (For a `num` component the second value is always `nil`:
the value is found, also through `is_a`, but its origin is not reported.) When you put `ecs.get` in the middle of an expression Lua keeps only the
first value; but at the **end** of an argument list both come along. Wrap it in parentheses
to keep only the value: `f((ecs.get('ball', 'x')))`.

```lua
local c = ecs.comps('ball'); table.sort(c)
out(table.concat(c, ','), ecs.kind('x'), ecs.kind('hp'), ecs.kind('tags'), tostring(ecs.kind('nothing')))
=> label,x,y | num | int | names | nil
```

`ecs.del` removes a value, and a missing entity is not an error for a read:

```lua
ecs.del('ball', 'label')
out(tostring(ecs.get('ball', 'label')), ecs.exists('ball'))     => nil | true
out(tostring(ecs.get('nobody', 'x')))                           => nil
```

A text, a name list and a bool work like this:

```lua
ecs.set('ball', 'tags', 'red fast')
local t = ecs.get('ball', 'tags')
out(#t, t[1], t[2])                    => 2 | red | fast
ecs.set('ball', 'alive', true)
out(ecs.get('ball', 'alive'))          => true | ball
```

A call costs about 0.1 microseconds. For thousands of entities per tick use the bulk calls
(5.4).

### 5.2 Prefabs: `is_a`

A **prefab** is an entity that others copy values from. Give an entity `is_a = 'tpl'` and it
*shows* the values of `tpl` until it has an own value of its own.

```lua
ecs.set('tpl', 'hp', 10)
ecs.set('grunt', 'is_a', 'tpl')       -- a new entity that is a copy of tpl
out(ecs.get('grunt', 'hp'))           => 10 | tpl       -- the value, and where it came from
out(tostring(ecs.own('grunt', 'hp'))) => nil            -- the grunt has none of its own
ecs.set('grunt', 'hp', 4)
out(ecs.get('grunt', 'hp'))           => 4 | grunt      -- now its own
ecs.del('grunt', 'hp')
out(ecs.get('grunt', 'hp'))           => 10 | tpl       -- back to the prefab
```

The chain may be longer (a goblin is an orc is a creature). A loop in the chain is refused
(`loop: is_a`). Change the prefab and every entity that did not override the value changes.

### 5.3 Making and removing entities

| Call | Returns | Notes |
|---|---|---|
| `ecs.spawn({c = v, ...})` | number | a nameless entity, called `#1`, `#2`, ... |
| `ecs.spawn(name, {c = v, ...})` | number | a named entity; `exists: <name>` when the name is taken. `is_a` and `parent` are set first |
| `ecs.kill(e)` | | removes the entity |
| `ecs.instantiate(prefab, id)` | | `id` and its children become instances of `prefab` and its children |
| `ecs.id(e)` | number | the number of an entity |
| `ecs.name(e)` | text | its name; `'#<n>'` for a nameless one |
| `ecs.with(c [, v])` | `{names}` | sorted names of the entities that have an own value of `c` (equal to `v`) |
| `ecs.children(e)` | `{names}` | the entities whose `parent` is `e` |
| `ecs.count([c])` | number | all entities, or those with an own value of `c` |

```lua
local a = ecs.spawn({x = 5})
local b = ecs.spawn('goblin', {hp = 3, label = 'goblin'})
out(ecs.name(a), ecs.name(b), ecs.get('goblin', 'hp'))     => #1 | goblin | 3 | goblin
```

A name is never given twice. A killed nameless entity does not give its number back:

```lua
local a = ecs.spawn({x = 1})            -- #1
local b = ecs.spawn({x = 2})            -- #2
ecs.kill(a)
local c = ecs.spawn({x = 3})
out(ecs.name(b), ecs.exists(a), ecs.name(c))               => #2 | false | #3
```

(`#1` was killed; the next entity is `#3`, not `#1` again.) **A number is safe**: it holds an index *and* a generation, so after `ecs.kill('ball')` and a new
`ecs.spawn('ball', ...)` the old number does not reach the new ball:

```lua
local id = ecs.id('ball')           -- a big number such as 4294967318
ecs.kill('ball'); ecs.spawn('ball', {x = 1})
out(ecs.exists(id), ecs.exists('ball'))                    => false | true
```

Use a number for `num` components when speed matters: `ecs.set(id, 'x', 5)`.

```lua
out(table.concat(ecs.with('x'), ','), table.concat(ecs.with('hp'), ','),
    table.concat(ecs.with('hp', 10), ','), #ecs.with('hp', 99))     => ball | orc | orc | 0
out(ecs.count('x'), ecs.count('hp'))                                => 1 | 1
```

**Parent and children** build a tree of things (a house with a door and a window):

```lua
ecs.set('house', 'label', 'house')
ecs.set('door', 'parent', 'house')
ecs.set('window', 'parent', 'house')
out(table.concat(ecs.children('house'), ','))              => door,window
```

`parent` must name an existing entity: setting `'parent'` to a missing one fails with
`no_entity: house`. A loop is refused (`loop: parent`).

**`ecs.instantiate`** copies a whole tree, so you can stamp out many alike things:

```lua
ecs.set('tpl', 'hp', 10)
ecs.set('tpl.arm', 'parent', 'tpl')
ecs.set('tpl.arm', 'hp', 3)
ecs.instantiate('tpl', 'p1')
out(ecs.get('p1', 'hp'), table.concat(ecs.children('p1'), ','), (ecs.get('p1.arm', 'hp')))
=> 10 | p1.arm | 3
```

**A number that sits in a column is a float.** If you keep an entity number in a `num`
component (the solver does: `node_a`), turn it back before use: `ecs.get(math.tointeger(v), ...)`.

### 5.4 Whole `num` columns at once

A loop in Lua over 1000 entities costs a call each. The bulk calls do the whole column in one
call. They work only on `num` components and only on entities that **already have** the
component (a missing one is not created).

| Call | Returns | Notes |
|---|---|---|
| `ecs.col(c)` | number | the column number of a `num` component (here `ecs.col('x')` → `1`, `ecs.col('y')` → `2`) |
| `ecs.query(c1, c2, ...)` | `n, ids, v1, v2, ...` | the entities that have all these components; a new table per component, same order |
| `ecs.read(c, ids, vals)` | `n` | one column into two tables you keep and reuse (no new garbage); entries after `n` are old |
| `ecs.write(c, ids, vals [, n])` | | the values back in one call (rights and undo hold) |
| `ecs.add(dst, src [, k])` | | `dst += src * k` (`k` = 1) for every entity that has both |
| `ecs.scale(c, k)` | | `c *= k` |
| `ecs.fill(c, v)` | | `c = v` |
| `ecs.clamp(c, lo, hi)` | | `lo <= c <= hi` |
| `ecs.sum(c)` | `sum, n` | the total and how many |

```lua
local n, ids, xs = ecs.query('x')
out(n, ecs.name(ids[1]), xs[1])                  => 1 | ball | 50.0

ecs.set('orc', 'x', 4)                           -- now two entities have x: ball 50, orc 4
ecs.fill('y', 7)                                 -- only the ball has y: it becomes 7
ecs.scale('x', 2)                                -- ball 100, orc 8
ecs.clamp('x', 0, 60)                            -- ball 60, orc 8
local s, n = ecs.sum('x')
out(s, n, ecs.get('ball', 'y'), ecs.get('orc', 'y'))   => 68.0 | 2 | 7.0 | nil | nil

ecs.set('ball', 'vx', 2)                         -- move by speed: x += vx * 0.5
ecs.add('x', 'vx', 0.5)
out((ecs.get('ball', 'x')))                      => 51.0
```

Your own loop over a column, the fast way: `ecs.read` into tables you keep, compute, `ecs.write`
back.

```lua
local ids, v = {}, {}                              -- keep these tables between ticks
local n = ecs.read('x', ids, v)
for i = 1, n do v[i] = v[i] + 1 end
ecs.write('x', ids, v, n)
out(n, (ecs.get('ball', 'x')))                     => 1 | 51.0
```

A loop of 1000 entities through `query`, Lua and `write` takes under 1 ms.

### 5.5 Code, time, results

| Call | Returns | Notes |
|---|---|---|
| `ecs.call(entity, arg)` | what that code returns | runs shared code (`lib.*`) with the rights of the caller |
| `ecs.tick()` | number | how many ticks the world has had |
| `ecs.role()` | text | the role this run acts as (`'system'` in a game) |
| `ecs.run(name [, arg])` | | calls a Rust system that the program added (`Host::add_system`); `no_system` otherwise |
| `ecs.commit({key = value})` | | one result for the program, which may hand it to another world. One at a time |
| `ecs.history(e, n)` | `{{seq, role, op, comp, value}}` | the last `n` (at most 50) changes of one entity, newest first; empty without a journal |
| `ecs.journal(n)` | `{{role, op, entity, comp, value}}` | the last `n` (at most 50) entries of the journal |
| `ecs.quit()` | | asks the game to end (in `lust dev` it only pauses). Not from `draw/` (`event_in_draw`) |

`ecs.call` runs with the rights of the caller, and the top level of the called file is read
only. Calls may be nested up to 8 deep (`too_deep`).

```lua
out(ecs.tick(), ecs.role())                      => 0 | system
ecs.commit({winner = 'ball', score = 12})        -- the program takes it: Host::take_result
```

`ecs.history` and `ecs.journal` are for `ledger` worlds: see section 10 for an example.

---

## 6. `input`

| Call | Returns |
|---|---|
| `input.down(key)` | `true` while the key is held |
| `input.pressed(key)` | `true` in the tick the key went down (once per press) |
| `input.mouse()` | `x, y, down` in the units of `gfx.size` (0, 0 before the first move) |
| `input.mic()` | the loudness of the microphone, 0..1 (after `gfx.mic(true)`) |

Keys: `up down left right space enter escape shift ctrl alt tab backspace f1`, `a`..`z`,
`0`..`9`, and `mouse` (the left button; `input.pressed('mouse')` is a click). Letters and digits
are named by the key itself, whatever the keyboard layout (the key R is `r` on a Russian layout
too). There is no right mouse button in Lust: use keys.

**Held versus pressed** is the difference between "run while the key is down" and "do this
once". A system that moves the ball while `right` is held and counts a press of `space` once:

```lua
return function(dt)
  if input.down('right') then ecs.set('ball', 'x', ecs.get('ball', 'x') + 100 * dt) end
  if input.pressed('space') then ecs.set('ball', 'clicks', ecs.get('ball', 'clicks') + 1) end
end
```

Tested through the driver (the ball starts at x = 50):

```lua
keys('right')       -- hold the right key from now on
step(6)             -- six ticks of 1/60 s
print(get('ball', 'x'))      --> 60          (50 + 100 * 6/60)
keys()              -- let go
step(6)
print(get('ball', 'x'))      --> 60          (it stopped)

press('space') step() step() press('space') step()
print(get('ball', 'clicks')) --> 2           (two presses, not three ticks)
```

**The mouse.** `input.mouse()` gives where the pointer is; `input.pressed('mouse')` is true in the
tick of a click:

```lua
if input.pressed('mouse') then
  local mx, my = input.mouse()
  ecs.set('ball', 'x', mx)
  ecs.set('ball', 'y', my)
end
```

```lua
mouse(120, 40, true)  step()  mouse(120, 40, false)  step()
print(get('ball', 'x'), get('ball', 'y'))      --> 120   40
```

---

## 7. `gfx`: drawing (only in `draw/` files)

You do not draw pixels. You send **commands** (a rectangle, a circle, a text) and the display
draws them. All coordinates are in your own units: `gfx.size(w, h)` says how large the screen
is; the origin is the top left corner and **y grows downward**.

| Call | What it does |
|---|---|
| `gfx.size(w, h)` | the screen in your units (default 800 × 600), each 1..10000, else `bad_size` |
| `gfx.clear(color)` | fills the whole screen |
| `gfx.rect(x, y, w, h, color)` | a filled rectangle; `x, y` is its top left corner |
| `gfx.circle(x, y, r, color)` | a filled circle around `x, y` |
| `gfx.line(x1, y1, x2, y2, width, color)` | a line |
| `gfx.text(x, y, size, color, text)` | one line of text; `y` is its **top**; the text may be text, number or boolean |
| `gfx.measure(text, size)` → `w, h` | the box of a text; works everywhere, also in `sys/` |
| `gfx.mic(true)` | asks the browser for the microphone (the person must agree); only the loudness reaches the world: `input.mic()` |

Colors are `'#rgb'`, `'#rrggbb'` or a CSS name (`'white'`, `'yellow'`). Text is measured at the
width of one character that the browser reports (in a test the width is 0.6 of the size).

```lua
return function()
  gfx.size(200, 100)
  gfx.clear('#102030')
  gfx.rect(10, 10, 50, 20, '#ff0000')
  gfx.circle(ecs.get('ball', 'x'), ecs.get('ball', 'y'), 5, 'white')
  gfx.line(0, 0, 200, 100, 2, '#0f0')
  gfx.text(10, 60, 16, '#fff', 'x = ' .. ecs.get('ball', 'x'))
  local w, h = gfx.measure('hello', 16)
  gfx.text(10, 80, 16, 'yellow', 'w=' .. w .. ' h=' .. h)
end
```

What the test driver's `draw()` returns for it - **the picture as text**, one command a line
(this is also how an AI "sees" the screen):

```
size 200 100
clear #102030
rect 10 10 50 20 #ff0000
circle 50 30 5 white
line 0 0 200 100 2 #0f0
text 10 60 16 #fff x = 50.0
text 10 80 16 yellow w=48.0 h=20.0
```

Rules: a frame holds at most 20 000 commands (`too_many_cmds`). Every `gfx.*` call except
`gfx.measure` fails with `gfx_outside_draw` outside a `draw/` file. A drawing may only read the
world. The drawings of one frame run in the order of their `ord`: the later one is on top.

---

## 8. `lib.ui`: buttons, menus, labels (built into every project)

`local ui = ecs.call('lib.ui')`. It is in every project; you do not write it. A widget is a plain
table with `x, y, w, h`. **Build the same widget in the `sys/` file (to test the click) and in
the `draw/` file (to draw it).**

| Call | Notes |
|---|---|
| `ui.button(text, x, y, {w, h, size, pad, enabled, id})` → `b` | as big as its text needs, unless you give `w` and `h` |
| `ui.inside(b, px, py)`, `ui.hover(b)`, `ui.down(b)` | is the point inside / the mouse over / the mouse key held on it |
| `ui.clicked(b)` | the button was clicked in this tick (in `sys/`) |
| `ui.draw(b, focused)` | draws it (in `draw/`); `focused` gives it a yellow border |
| `ui.label(text, x, y, {align, size, color})` → `w, h` | `align`: `'left'`, `'center'`, `'right'` |
| `ui.panel(x, y, w, h [, color, border])` | a filled box with a border |
| `ui.measure(text, size)` → `w, h` | |
| `ui.wrap(text, w, size)` → lines | breaks a text into lines that fit `w` |
| `ui.paragraph(text, x, y, w, {size, color})` → height | wraps and draws |
| `ui.stack(list, cx, top, gap)` → bottom | puts widgets one under another, centered on `cx` |
| `ui.row(list, cx, top, gap)` → right | side by side, centered |
| `ui.menu(items, cx, top, {size, pad, w, gap})` → list | an item is a text or `{text = .., enabled = false}` |
| `ui.update_menu(list, sel, {wrap})` → `sel, chosen, edge` | in `sys/`: arrow keys, mouse, enter, space, click |
| `ui.draw_menu(list, sel)` | in `draw/` |
| `ui.theme` | the colors and sizes: `text, dim, back, button, hover, down, off, border, focus, size, pad` |

**A button that counts clicks.** The `sys/` file tests the click, the `draw/` file draws:

```lua
-- sys/move.lua
return function(dt)
  local ui = ecs.call('lib.ui')
  local b = ui.button('Play', 150, 100, {w = 80, h = 30})
  if ui.clicked(b) then ecs.set('ball', 'score', ecs.get('ball', 'score') + 1) end
end
-- draw/main.lua
return function()
  gfx.size(300, 200)
  gfx.clear('#102030')
  local ui = ecs.call('lib.ui')
  local b = ui.button('Play', 150, 100, {w = 80, h = 30})
  ui.draw(b, false)
  ui.label('Score: ' .. string.format('%d', ecs.get('ball', 'score')), 10, 10, {size = 16, color = 'white'})
end
```

```lua
mouse(150, 100, true) step() mouse(150, 100, false) step()
print(get('ball', 'score'))          --> 1       (the click was on the button)
mouse(5, 5, true) step() mouse(5, 5, false) step()
print(get('ball', 'score'))          --> 1       (a click elsewhere does nothing)
print(draw())
```

```
size 300 200
clear #102030
rect 148 98 84 34 #7777aa          the border of the button
rect 150 100 80 30 #2b2b44         the button
text 161.2 100 24 #ffffff Play     its text, centered
text 10 10 16 white Score: 0
```

**A column of widgets and a paragraph.**

```lua
ui.panel(10, 10, 280, 180)
local a = ui.button('Yes', 0, 0, {w = 60, h = 30})
local b = ui.button('No', 0, 0, {w = 60, h = 30})
local bottom = ui.stack({a, b}, 150, 40, 10)        -- centered on x = 150, from y = 40, gap 10
ui.draw(a, false)
ui.draw(b, true)                                      -- focused: yellow border
local h = ui.paragraph('Press Yes to go on and No to stop', 20, bottom + 10, 120, {size = 12})
```

`ui.stack` returned `bottom = 110` (the buttons are at y 40 and y 80), `ui.paragraph` wrapped the
text to 120 units wide into three lines of 12 and returned the height `45.0`:

```
text 20 120 12 #ffffff Press Yes to go
text 20 135 12 #ffffff on and No to
text 20 150 12 #ffffff stop
```

**Measuring and wrapping:**

```lua
local w, h = ui.measure('Hello', 16)                  --> 48.0  20.0
local lines = ui.wrap('one two three four five six', 80, 16)
--> 4 lines; the first is 'one two'
```

**A menu.** `ui.menu` builds the list, `ui.update_menu` (in `sys/`) moves the selection with the
arrow keys or the mouse and tells you what was chosen:

```lua
local list = ui.menu({'Play', 'Options', 'Quit'}, 150, 60, {size = 16})
local sel = ecs.get('ball', 'score')                  -- keep the selection in a component
local nsel, chosen = ui.update_menu(list, sel)
ecs.set('ball', 'score', nsel)
if chosen then ecs.set('ball', 'label', list[chosen].text) end
```

```lua
press('down') step()
print(get('ball', 'score'))        --> 2     the selection moved to the second item
press('enter') step()
print(get('ball', 'label'))        --> Options
```

`ui.theme` holds the look; read it to match your own drawing, or change it once at the start:
`ui.theme.size` is `24`, `ui.theme.pad` is `14`, `ui.theme.text` is `'#ffffff'`.

---

## 9. Saves

| Call | Returns | Notes |
|---|---|---|
| `ecs.save(slot)` | | writes all data of the world after this tick |
| `ecs.load(slot)` | `false` when there is no such slot | brings it back after this tick; refused (shown on the status line) when a type is gone or has another kind |
| `ecs.slots()` | `{names}` | the saves there are, sorted |
| `ecs.erase(slot)` | | deletes one |

A slot name is 1 to 32 characters of `a-z 0-9 _ -`; a wrong one raises `bad_slot`. At most 20 slots
of 4 MB. **Only data is saved, never code**: a saved game gets its code from the program. Saves
live in the user's data folder (`%APPDATA%\lust\<game>` on Windows); in tests in memory.

The save and the load happen **after the tick** that asked, never in the middle of a run:

```lua
-- sys/move.lua
return function(dt)
  if input.pressed('f5') then ecs.save('quick') end
  if input.pressed('f9') then ecs.load('quick') end
end
```

```lua
press('f5') step()                  -- saved with x = 50
set('ball', 'x', 999)
press('f9') step() step()           -- loaded
print(get('ball', 'x'))             --> 50
```

```lua
ecs.save('quick') ecs.save('b')
ecs.slots()                         --> {'b', 'quick'}
ecs.load('nothing')                 --> false
ecs.save('Bad Slot!')               --> bad_slot: 'Bad Slot!' (1..32 of a-z 0-9 _ -)
```

---

## 10. Worlds with rights (ledger): operations, roles, views, journal

A `ledger` world is for programs where **who did what** matters (a shop, a warehouse, a strategy
game with players): every change is written to a journal, every component may be written only by
certain **roles**, and a run that fails is **undone as a whole**. The things you do are
**operations** (`op/`); the questions you ask are **views** (`view/`).

```lua
-- world.lua
return {
  spec = 'ledger',
  types = {
    stock = { kind = 'int', write = 'clerk system' },   -- who may write it
    label = { kind = 'text', write = 'clerk system' },
    price = { kind = 'int', write = 'boss' },
  },
  entities = {
    ['role.clerk'] = { desc = 'sells' },
    ['role.boss']  = { desc = 'sets prices' },
    tea    = { label = 'Tea',    stock = 5, price = 3 },
    coffee = { label = 'Coffee', stock = 2, price = 4 },
    ['op.sell']    = { params = 'item:id n:int', roles = 'clerk', desc = 'sells n of an item' },
    ['view.stock'] = { roles = 'clerk boss' },
    ['view.hist']  = { roles = 'clerk boss' },
  },
}
```

```lua
-- op/sell.lua
return function(p)
  local have = ecs.get(p.item, 'stock')
  if have < p.n then error('not_enough_stock', 0) end
  ecs.set(p.item, 'stock', have - p.n)
  ecs.set(p.item, 'label', 'sold ' .. p.n)
end
```

```lua
-- view/stock.lua
return function(arg)
  local rows = {}
  for _, name in ipairs(ecs.with('stock')) do
    rows[#rows + 1] = {item = name, stock = ecs.get(name, 'stock')}
  end
  return rows
end
```

Run them from the driver (the first argument is the role that acts):

```lua
run('clerk', 'sell', {item = 'tea', n = 2})
print(get('tea', 'stock'))                 --> 3

run('clerk', 'sell', {item = 'coffee', n = 9})
--> test/x.lua:1: not_enough_stock          (the run stopped; nothing changed)

run('boss', 'sell', {item = 'tea', n = 1})
--> test/x.lua:1: forbidden: sell for boss  (a boss may not run `sell`)

local rows = view('stock')
print(#rows, rows[1].item, rows[1].stock)  --> 2   coffee   2
```

**Parameter kinds** of an operation: `id` (an existing entity), `text`, `int`, `long`; `name?` is
optional. A writer with no right gets `no_right: <comp> for <role>` from `ecs.set`.

**History and journal.** Every change is an entry of a hash chain; nothing can be changed
afterwards unseen. A view that lists the history of the tea:

```lua
-- view/hist.lua
return function(arg)
  local h = ecs.history('tea', 5)
  local out = {}
  for i = 1, #h do out[i] = h[i].op .. ':' .. h[i].comp .. '=' .. tostring(h[i].value) .. ' by ' .. h[i].role end
  return {n = #h, rows = out, j = #ecs.journal(3)}
end
```

```lua
run('clerk', 'sell', {item = 'tea', n = 2})
local v = view('hist')
print(v.n, v.j)
for i = 1, #v.rows do print(v.rows[i]) end
```

```
5	3
sell:label=sold 2 by clerk
sell:stock=3 by clerk
dev:stock=5 by core
dev:price=3 by core
dev:label=Tea by core
```

The newest change is first; `dev ... by core` is the start state from `world.lua`.

---

## 11. The solver (`ecs.*`)

An **abstract constraint solver**: it moves points so that links between them keep their length
and so that points do not enter barriers. It knows nothing about bridges, wheels, ropes, cars or
strength; every number (mass, stiffness, friction, damping) comes from the components your
program sets. It reads `num` columns, works on plain arrays and writes the result back. The
name says what it does: *XPBD* (extended position based dynamics), which in plain words is "move
every point so the rules hold, and take the speed from how far it moved".

### 11.1 The three kinds of things

| Thing | What it is | In the world |
|---|---|---|
| **point** | a body that has a position and a mass; free, or fixed in place | an entity with `x, y, old_x, old_y, inv_mass` |
| **link** | a rule between two points: keep this distance | an entity with `node_a, node_b, rest_len` |
| **barrier** | something points may not enter: a **segment** (static) or a **link with a layer** (an *edge*, a barrier that moves with its two points) | `x1, y1, x2, y2` or a link with `layer` |

Four ideas explain everything else:

1. **Speed is a difference.** The solver is *Verlet*: the speed of a point is `x - old_x`, how far
   it moved since the last step. So to give a point a speed, set `old_x` behind `x`. To stop it,
   set `old_x = x`.
2. **`inv_mass`** is 1 divided by the mass. `inv_mass = 0` means infinite mass: the point is
   **fixed**. A bigger mass gives a smaller `inv_mass`.
3. **A rule moves points, and the move becomes speed.** If you break a rule (a rod 60 long
   between points 100 apart) the solver pulls them together in one step, and that jump is a
   speed afterwards. Build links at the length you want, and change `rest_len` a little at a
   time (see 11.8).
4. **A tick is 16 small steps.** `solve_tick` splits a tick into substeps (16 by default), each
   with its own gravity, rules and contacts: much more stable than one big step.

### 11.2 The columns

**Point** (needs `x, y, old_x, old_y, inv_mass`):

| Column | Default | Meaning |
|---|---|---|
| `x`, `y` | needed | the position |
| `old_x`, `old_y` | needed | the position one step ago: the speed is `x - old_x` |
| `inv_mass` | needed | 1 / mass; `0` is fixed |
| `radius` | 0 | for collisions; 0 never collides |
| `layer`, `mask` | 1, 1 | bits: what it *is*, and what it *hits* (see 11.6) |
| `group` | 0 | points and edges of one non-zero group never collide with each other (one body) |
| `damping` | the call's | the share of speed kept each step (0..1); below 1 it brakes |
| `bend_damping` | the call's | friction against turning between its links (see `damp_angular`) |
| `spin` | 0 | turning speed of a wheel, rad/s, plus is clockwise on the screen; friction drives with it |
| `inertia` | 0 | moment of inertia of the wheel; with it friction changes `spin` itself |
| `contact`, `contact_nx`, `contact_ny` | written | 1 when the point was pushed out of a barrier this step, and the direction of the push |

**Link** (needs `node_a, node_b, rest_len`; the ends are `ecs.id` numbers):

| Column | Default | Meaning |
|---|---|---|
| `rest_len` | needed | the length it keeps |
| `compliance` | 0 | softness: `0` is rigid, larger is a softer spring (displacement per unit of force) |
| `mode` | 0 | `0` keeps the length both ways (a rod); `1` not longer (a rope); `2` not shorter (a strut) |
| `ang_damping` | the call's | share of its turning taken each step |
| `spring_damping` | 0 | a damper for a soft link, in seconds (see 11.7) |
| `lambda` | written | the multiplier of the last call (the push, in impulse units) |
| `force_max` | written | the biggest force on the link since you last zeroed it |
| `layer` (+ `friction`, `radius`, `group`) | | with a layer the link is also an **edge**: a movable barrier, a capsule of `radius` |
| `catenary_a` (+ `catenary_x0`, `catenary_y0`, `sag`) | | with these it is a **cable**: it hangs as a curve |
| `linear_density` | 0 | weight per length of a cable (it loads its two ends) |
| `min_slack` | 0.001 | a cable is never quite straight: this share of the chord |
| `tension` | written | the pull of a cable |

**Segment** (a static barrier; needs `x1, y1, x2, y2`): `friction` (0), `layer` (1), `radius`
(0; with a radius it is a capsule).

> **Output columns are written only on entities that already have them.** `contact`,
> `lambda`, `force_max`, `sag`, `tension`... are filled in by the solver but never created by
> it. Give a link `force_max = 0` and `lambda = 0` when you create it if you want to read them.

**Two helpers used in the examples below** (put them at the top of your file):

```lua
local function pt(name, x, y, inv, extra)          -- inv = inverse mass; 0 = fixed
  local e = {x = x, y = y, old_x = x, old_y = y, inv_mass = inv or 1}
  for k, v in pairs(extra or {}) do e[k] = v end
  ecs.spawn(name, e)
end
local function link(name, a, b, len, extra)        -- a, b = the names of two points
  local e = {node_a = ecs.id(a), node_b = ecs.id(b), rest_len = len}
  for k, v in pairs(extra or {}) do e[k] = v end
  ecs.spawn(name, e)
end
```

### 11.3 A tick: `ecs.solve_tick`

`ecs.solve_tick(dt, substeps, gx, gy, damping [, spin, bend])` does a whole tick in **one call**:
for each of `substeps` steps it moves the points by their speed and by gravity, keeps the links,
turns friction on, pushes points out of edges and segments. It returns the number of steps.

| Argument | Meaning |
|---|---|
| `dt` | the time of the tick, seconds (`dt` of your system) |
| `substeps` | 1..256; 16 is a good start |
| `gx, gy` | gravity per second squared; in a y-down world `gy = 980` falls downward |
| `damping` | the share of speed kept per step (0.999 loses a little, 1 loses none) |
| `spin`, `bend` | optional friction against turning of links and joints (see 11.5) |

**A ball in free fall** (no links): 60 ticks of 1/60 s is one second; a body falling for one
second under `g = 980` covers `½·g·t² = 490` pixels:

```lua
pt('ball', 100, 0, 1)
for i = 1, 60 do ecs.solve_tick(1/60, 16, 0, 980, 1) end
out(f(ecs.get('ball', 'y')), f(ecs.get('ball', 'y') - ecs.get('ball', 'old_y'), 3))
=> 490.5 | 1.021
```

The 490.5 is the position (the exact formula gives 490.0, the difference is the discrete step);
`1.021` is the last step's move, so the speed is `1.021 · 16 · 60 ≈ 980` pixels per second, as it
should be after one second. A **fixed point** (`inv_mass = 0`) does not move: `pt('anchor', 100,
100, 0)` is still at `y = 100.0` after any number of ticks.

**A pendulum** (a fixed point, a rod, a free point):

```lua
pt('anchor', 100, 100, 0)
pt('bob', 200, 100, 1)                              -- 100 to the right of the anchor
link('rod', 'anchor', 'bob', 100)                   -- a rod 100 long
local worst = 0
for i = 1, 120 do                                    -- two seconds
  ecs.solve_tick(1/60, 16, 0, 980, 0.999)
  local dx, dy = ecs.get('bob', 'x') - 100, ecs.get('bob', 'y') - 100
  worst = math.max(worst, math.abs(math.sqrt(dx*dx + dy*dy) - 100))
end
out(f(worst, 3), f(ecs.get('bob', 'x')), f(ecs.get('bob', 'y')))
=> 0.000 | 140.6 | 191.4
```

In two seconds the rod's length was never wrong by more than `0.000` and the bob swung down and
up to `(140.6, 191.4)`.

**`solve_tick` is the same as the calls one by one.** It exists because it is 5–14 times
faster: it reads the columns once and writes them once. The calls it makes in each step are:

```lua
local h = dt / 16
for i = 1, 16 do
  ecs.integrate_verlet(h, 0, 980, 1)          -- move by speed and gravity
  ecs.apply_catenary_load(h, 980)             -- (if you have cables)
  ecs.solve_distance(h, 1)                    -- keep the links
  ecs.damp_angular(spin, bend)                -- (if you gave spin, bend)
  ecs.project_edges(1, h)                     -- points out of moving barriers
  ecs.project_segments(1, h)                  -- points out of static barriers
end
```

A test in the repository checks that the two give the same numbers; run by hand, the rod above
ends at `bob = (200.000, 100.145)` after one tick either way.

### 11.4 Links: rod, rope, strut, spring

| `mode` | Name | Rule | Example |
|---|---|---|---|
| 0 | rod / beam | keeps the length both ways | a road, a stick |
| 1 | rope | not longer than `rest_len`; shorter is free | a chain, a cable |
| 2 | strut | not shorter than `rest_len`; longer is free | a column that holds up |

```lua
-- a rope: the bob falls until the rope is taut, then hangs
pt('anchor', 100, 100, 0)
pt('bob', 100, 160, 1)                  -- 60 below the anchor, the rope is 100: slack
link('rope', 'anchor', 'bob', 100, {mode = 1})
for i = 1, 30 do ecs.solve_tick(1/60, 16, 0, 980, 1) end
out(f(ecs.get('bob', 'y')))
=> 200.0                                 -- it fell 40 freely, then the rope held it at 100 below
```

A strut is the opposite: a column that bears weight but would not pull:

```lua
pt('anchor', 100, 100, 0)
pt('top', 100, 70, 1)                    -- 30 above the anchor; gravity pushes it down onto it
link('strut', 'anchor', 'top', 30, {mode = 2})
for i = 1, 60 do ecs.solve_tick(1/60, 16, 0, 980, 1) end
out(f(ecs.get('top', 'y')))
=> 70.0                                  -- it stands; with mode = 1 it falls to 130.0
```

**A spring** is a link with `compliance > 0`. A weight of mass 1 hangs on a spring whose rest
length is 50:

```lua
pt('anchor', 100, 100, 0)
pt('bob', 100, 150, 1)
link('spring', 'anchor', 'bob', 50, {compliance = 0.01})
-- lowest, highest and final y over 3 seconds:
=> 150.1 | 168.1 | 159.7
```

It oscillates between 150.1 and 168.1 around its rest at `150 + m·g·compliance = 150 + 9.8`. A
rigid link has `compliance = 0`.

**Reading the force.** `force_max` is the biggest force on a link since you zeroed it:

```lua
pt('anchor', 100, 100, 0)
pt('bob', 100, 150, 1)
link('rod', 'anchor', 'bob', 50, {force_max = 0, lambda = 0})
for i = 1, 60 do ecs.solve_tick(1/60, 16, 0, 980, 0.999) end
out(f(ecs.get('rod', 'force_max')))
=> 980.0                                 -- the weight of a mass 1 under gravity 980: m · g
```

A game compares `force_max` with its own strength number to decide what breaks, then zeroes it
for the next tick with `ecs.fill('force_max', 0)` (the Lua side, `examples/solver`, does exactly
that). **A rigid link that is already taut carries a huge force**: a chain of two links of 100
between fixed points exactly 200 apart sags only `1.7` and reports `force_max ≈ 53 900`; the same
chain with links of 110 sags to `45.8`. Its steady load is `1176` for each link (`m·g` shared by
two links at a slant), but `force_max` is a **peak since you zeroed it**: if you let the chain
fall from straight and read after 240 ticks you see the shock of the catch, `27 555 840`. To read
the load of a settled structure, zero `force_max` and read it after one more tick; a game that
judges parts every tick does it that way, and waits some ticks after the start (the
example calls it `WARMUP`).

### 11.5 Damping: how to take energy away

Points lose speed in three ways; a stable, quiet simulation uses all of them a little.

| Where | What | How much to start with |
|---|---|---|
| `damping` (point column, or the call's argument) | speed kept per step | `0.999` per step |
| `ecs.damp_angular(spin, bend)` or the last two arguments of `solve_tick` | friction against **turning**: `spin` for each link (relative speed across it), `bend` for each joint (turning between its links) | `spin` 0.005, `bend` 0.05 |
| `spring_damping` (link column) | a damper for a soft link | see 11.7 |

A point with `damping = 0.9` keeps 90 % of its speed **each of the 16 steps**: it moves only
`10 · 0.9 / (1 − 0.9) = 90` pixels and stops, however long you wait. Another point with the
default 1 moves `10 · 160 = 1600`:

```lua
pt('a', 0, 0, 1, {damping = 0.9})
pt('b', 0, 0, 1)
ecs.set('a', 'old_x', -10)  ecs.set('b', 'old_x', -10)        -- both at 10 pixels per step
for i = 1, 10 do ecs.solve_tick(1/60, 16, 0, 0, 1) end
out(f(ecs.get('a', 'x')), f(ecs.get('b', 'x')))
=> 90.0 | 1600.0
```

`damp_angular` keeps **momentum** (the push is shared, so the whole body does not slow) and
only removes energy of turning; it exists because beams joined at pins tend to **vibrate**.
It returns the number of changed things.

### 11.6 Barriers: floors, roads and layers

**A floor** is a segment. A ball (a point with a `radius`) lands on it:

```lua
ecs.spawn('floor', {x1 = 0, y1 = 200, x2 = 400, y2 = 200, friction = 0.5, layer = 1})
pt('ball', 100, 0, 1, {radius = 10, layer = 8, mask = 1, contact = 0, contact_nx = 0, contact_ny = 0})
for i = 1, 120 do ecs.solve_tick(1/60, 16, 0, 980, 0.999) end
out(f(ecs.get('ball', 'y')), ecs.get('ball', 'contact'), f(ecs.get('ball', 'contact_ny'), 1))
=> 190.0 | 1.0 | -1.0
```

It rests at `190 = 200 − radius`, `contact` is `1` and the push direction `contact_ny = −1`
(upward on a y-down screen).

**Layers and masks decide who hits whom.** A thing has a `layer` (bits: what it is) and a
`mask` (bits: what it collides with). A collision happens when `thing.mask & barrier.layer ≠ 0`.
The ball has `mask = 1` and the floor `layer = 1`: they meet. A ghost with `mask = 0` falls
through:

```lua
ecs.spawn('floor', {x1 = 0, y1 = 200, x2 = 400, y2 = 200, layer = 1})
pt('ball',  100, 0, 1, {radius = 10, layer = 8,  mask = 1})
pt('ghost', 150, 0, 1, {radius = 10, layer = 16, mask = 0})
-- 2 seconds later: ball y = 190.0, ghost y = 1135.0 (still falling)
```

Use layers to say that a car's wheels hit the road but the bridge's joints do not hit the wheels.
`group` (non-zero) keeps points and edges of **one body** from colliding with each other.

**A road is an edge:** a link with a `layer` and a `radius` is also a barrier shaped like a
capsule (a line with rounded ends) that moves with its two points. Things are pushed out of it,
and, because its two points can be free, **the road gives way** under a weight (the bridges of
`examples/solver` sag under the wheel). Here both ends are fixed, so it only holds the ball up.

```lua
pt('p1', 0, 100, 0)
pt('p2', 200, 100, 0)
link('road', 'p1', 'p2', 200, {layer = 4, radius = 3, friction = 0.5})
pt('ball', 100, 0, 1, {radius = 10, layer = 8, mask = 4})
for i = 1, 120 do ecs.solve_tick(1/60, 16, 0, 980, 0.999) end
out(f(ecs.get('ball', 'y')))
=> 87.0                                  -- 100 (road) − 3 (its radius) − 10 (the ball's)
```

### 11.7 Wheels: spin, friction and inertia

A point with a `radius` and a **`spin`** rolls on what it touches: the rim moves at `spin ·
radius` along the surface, and friction drives the point. `spin` is in radians per second; plus
is **clockwise on the screen** (y points down), so `spin = 5` drives to the right.

- **Friction is Coulomb's.** What a contact can do *along* the surface is at most `friction ×`
  what it does *across* it. So a wheel on ice (`friction = 0`) does not drive, on rubber (`1`)
  it grips, and nothing is ever infinite. The coefficient is a column of the barrier (a
  segment or an edge).
- **`inertia = 0`** (default): the program owns `spin`. It sets it (a motor with no limit); the
  solver only reads it.
- **`inertia > 0`** (moment of inertia; for a disc `mass · r² / 2`): the solver changes `spin`
  itself: friction spins a free wheel up, brakes it, lets it slip. The new value is written
  back. A motor is then a torque you add to `spin` each tick.

A wheel driven at `spin = 5` on rubber travels `5 · 10 · 1 s ≈ 50`; on ice it does not move:

```lua
ecs.spawn('floor', {x1 = -1000, y1 = 100, x2 = 1000, y2 = 100, friction = 1, layer = 1})   -- rubber
pt('w', 0, 89, 1, {radius = 10, layer = 8, mask = 1, spin = 5})
for i = 1, 60 do ecs.solve_tick(1/60, 16, 0, 980, 1) end
out(f(ecs.get('w', 'x')))             => 47.7           -- friction = 0: => 0.0
-- with spin = -5 the wheel goes the other way: => -47.7
```

A **free wheel with inertia** that slides over rubber at 100 pixels per second starts to roll:
momentum is shared with its turning, so it ends slower, at `100 / (1 + I/(m r²)) = 66.7` px/s,
rolling (`spin = v / r = 6.67`):

```lua
-- floor friction = 1; wheel mass 1, radius 10, inertia 50, speed 100 px/s:
pt('w', 0, 90, 1, {radius = 10, layer = 8, mask = 1, spin = 0, inertia = 50, old_x = -0.1041667})
for i = 1, 60 do ecs.solve_tick(1/60, 16, 0, 980, 1) end
out(f(ecs.get('w', 'x'), 1), f(ecs.get('w', 'spin'), 2))
=> 67.3 | 6.67
```

(`old_x = -100 / (60 · 16)` gives 100 pixels per second.) The 67.3 pixels is the distance covered
in the first second; the spin settled on `6.67` as predicted.

**`spring_damping` for a spring that must not swing.** A soft link on a heavy thing oscillates for
a long time (a mouse spring dragging a wheel overshot by 155 px). Add a damper: `beta` in
**seconds** (= `c / k`); the critical value, with no swing past the target, is `2 · sqrt(m ·
compliance)`. It works only on a link with `compliance > 0`:

```lua
link('spring', 'anchor', 'bob', 50, {compliance = 0.01, spring_damping = 0.2})
-- lowest, highest, final: 150.1 | 159.8 | 159.8       (without it: 150.1 | 168.1 | 159.7)
```

The damped spring goes to its rest at 159.8 and never over it; the undamped one overshoots to
168.1.

### 11.8 The rule of the jerk

Everything that makes a position jump makes a speed: **a teleport, a new `rest_len` far from the
old, two far points joined by a rigid link**. Two free points 100 apart joined by a rod of 60 end up at
`20.0` and `80.0` and stay there (a rod holds). But a **strut** of 80 between two free points 50
apart throws them **530 apart** in one tick, and a rope of 60 between two free points 100 apart
makes them cross (`b − a = −60`): the correction of one step was kept as speed. A rod to a fixed
point ends calmly:

```lua
pt('a', 0, 0, 0)
pt('b', 100, 0, 1)
link('rod', 'a', 'b', 60)
for i = 1, 30 do ecs.solve_tick(1/60, 16, 0, 0, 0.99) end
out(f(ecs.get('b', 'x')))             => 60.0
```

To **move a point without a shot**, move `old_x`, `old_y` by the same amount as `x`, `y`. To change
a length (a piston, a winch), change `rest_len` a little each tick, never at once. To join two
things, join points that are close (a few pixels).

### 11.9 Look-ups and paths

| Call | Returns | What it does |
|---|---|---|
| `ecs.query_point(x, y, max_dist [, mask])` | `id, dist` or nil | the nearest point within `max_dist` (optionally of a `mask`) |
| `ecs.raycast_segments(x, y, dx, dy, max_dist [, mask])` | `id, hx, hy, nx, ny, t` or nil | the first segment on a ray: where it hit, the normal, the distance |
| `ecs.path(from, to [, cost])` | `ids, cost` or nil | the cheapest way over links, both ends in `ids`. **A\*** with a straight-line guide for `cost` 0, Dijkstra for 1 and 2. nil when there is no way or an end is not a point |
| `ecs.distances(from [, cost])` | `ids, costs` | the cost to every point that can be reached (Dijkstra), `costs[i]` for `ids[i]` |

`cost`: `0` the length now (default), `1` the rest length, `2` one per link. The ends of a path
are given as numbers (`ecs.id`).

```lua
-- the nearest point to (95, 90) within 50:
pt('a', 10, 10, 0)  pt('b', 100, 100, 0)
local id, d = ecs.query_point(95, 90, 50)
out(ecs.name(id), f(d), tostring(ecs.query_point(500, 500, 50)))
=> b | 11.2 | nil                         -- nothing within 50 of (500, 500): nil

-- a ray from (0, 50) to the right hits a wall at x = 100:
ecs.spawn('wall', {x1 = 100, y1 = 0, x2 = 100, y2 = 200, layer = 1})
local id, hx, hy, nx, ny, t = ecs.raycast_segments(0, 50, 1, 0, 500)
out(ecs.name(id), f(hx), f(hy), f(nx), f(ny), f(t))
=> wall | 100.0 | 50.0 | -1.0 | 0.0 | 100.0    -- hit at (100, 50); the wall faces left
```

**A path.** Four points in a square, the sides and one diagonal are links. The cheapest way from
`a` to `c` is the diagonal (141.4) and not two sides (200):

```lua
pt('a', 0, 0, 0)  pt('b', 100, 0, 0)  pt('c', 100, 100, 0)  pt('d', 0, 100, 0)
link('ab', 'a', 'b', 100)  link('bc', 'b', 'c', 100)  link('cd', 'c', 'd', 100)
link('ad', 'a', 'd', 100)  link('ac', 'a', 'c', 141.42)
local ids, cost = ecs.path(ecs.id('a'), ecs.id('c'))
-- the names of ids, joined with '>', and the cost:
=> a>c | 141.4
```

Two points that no link joins have no path: `ecs.path(ecs.id('a'), ecs.id('b'))` → `nil`. With one
link per step (`cost = 2`):

```lua
local ids, costs = ecs.distances(ecs.id('a'), 2)        -- a - b - c in a row
=> a=0.0 b=1.0 c=2.0
```

### 11.10 Cables (a catenary)

A **cable** is a link that hangs as a curve (a catenary), not a straight line. Give the link
`catenary_a`, `catenary_x0`, `catenary_y0`, `sag`, `linear_density` (and `tension` to read the
pull). `ecs.update_catenary(gy)` computes the curve of each cable, its `sag` and its `tension`;
`ecs.apply_catenary_load(dt, gy)` hangs the weight of each cable on its two end points (done by
`solve_tick`); `ecs.sample_catenary(id, t)` gives a point on the curve, `t` from 0 to 1.

```lua
pt('a', 0, 100, 0)  pt('b', 200, 100, 0)                  -- two fixed ends 200 apart
link('cable', 'a', 'b', 230, {mode = 1, catenary_a = 0, catenary_x0 = 0, catenary_y0 = 0,
                              sag = 0, linear_density = 1, tension = 0})
ecs.update_catenary(980)
local x, y = ecs.sample_catenary(ecs.id('cable'), 0.5)    -- the middle of the cable
out(f(ecs.get('cable', 'sag')), f(x), f(y), f(ecs.get('cable', 'tension')))
=> 49.9 | 100.0 | 149.9 | 154408.3
```

A cable 230 long between points 200 apart sags `49.9` (the middle at `y = 149.9`) and pulls its
ends with `154 408` (the `tension`: a weight of `230 · 980` is shared, and the sag is shallow).
A cable never pushes: when its ends come closer it sags more.

### 11.11 Errors, limits and details

- A bad argument raises `bad_arg: <name>`: `ecs.solve_tick(0, 16, 0, 980, 1)` → `bad_arg: dt`;
  `ecs.solve_distance(1/60, 0)` → `bad_arg: iterations`.
- `iterations` is 1..64; `substeps` 1..256; `dt` must be above 0.
- A writing call in a `draw/` file fails with `readonly`; two solver calls inside each other
  fail with `busy`.
- An empty world is not an error: every solver call returns `0` when it has nothing to do;
  `ecs.solve_tick` returns `16` once there is at least one point.
- **Speed** (`cargo test --release solver::bench -- --ignored --nocapture`, `solve_tick`, 16
  substeps, a lattice of about three boards per joint): 905 boards 1.8 ms a tick, 9 305 boards
  13.5 ms, 93 305 boards 123 ms. There is no grid for collisions yet: many wheels on a long road
  are each tested against each edge.
- Layer convention of the example game: ground 1, semi-solid 2, road edges 4, joints layer 8 mask 1,
  wheels layer 16 mask 23, mouse marker layer 32 mask 0.

### 11.12 A tick, as the example does it

`examples/solver` is a complete small game on the solver (a suspension bridge, a truss bridge, a
wheel you drive; strength, breaking, restart). Its tick is this short:

```lua
return function(dt)
  ecs.fill('force_max', 0)                                    -- forget last tick's loads
  ecs.solve_tick(dt, 16, 0, 980, 0.999, 0.00005, 0.0005)       -- the whole physics
  ecs.update_catenary(980)                                    -- the curve and tension of cables
  -- then the program reads force_max / tension and compares them with its own strength
  -- column: a part whose load is too high breaks (the demo replaces it by two half beams)
end
```

Run `lust test examples/solver` to see its tests, and `lust dev examples/solver` to play it
(hold the mouse button to drag the wheel, A and D turn it, R restarts, 1 and 2 pick a bridge,
F toggles the turn friction).

---

## 12. The driver and tests

`test/*.lua` is a different Lua: **driver code**. It runs a world from the outside, with no
screen, and checks what happens. It has no `ecs`, `input` or `gfx` of its own (using one gives
an error that says so); it has the functions below. `assert(condition, message)` fails a test.

| Call | What it does |
|---|---|
| `step([n [, dt]])` | runs `n` ticks (default 1), each of `dt` seconds (default 1/60) |
| `get(e, c)`, `set(e, c, v)` | reads / writes a value; `set(e, c, '')` deletes it |
| `ents(c)` | names of the entities that have `c` |
| `tick()` | how many ticks have run |
| `keys(k, ...)` | holds these keys from now on; `keys()` lets go |
| `press(k)` | the key goes down on the next tick (once) |
| `mouse(x, y, down)` | moves the mouse; `down` going from false to true is a click |
| `mic(level)` | sets the microphone loudness |
| `glyph(w)` | the width of a character as a share of its size (default 0.6) |
| `draw()` | the frame as text lines (see section 7) |
| `events()` | what the world told the display since the last call, e.g. `{'quit'}` |
| `run(role, op, {k = v})`, `view(name [, arg])` | runs an operation, asks a view (section 10) |
| `ms()` | milliseconds since the start |
| `print(...)` | its output is the answer of `lust_try`; in a failing test it is shown |

A click is `mouse(x, y, true) step() mouse(x, y, false) step()`.

A test file is a script; it passes when it ends without an error:

```lua
-- test/move.lua
keys('right')
step(6)
assert(math.abs(get('ball', 'x') - 60) < 0.001, 'the ball did not move: ' .. get('ball', 'x'))
keys()
step(6)
assert(math.abs(get('ball', 'x') - 60) < 0.001, 'the ball did not stop')
```

```
$ lust test examples/solver
ok scene (116 ms)
ok truss (90 ms)
```

A failing test prints what the test printed and why it failed:

```
FAIL h: 50
0
error: test/a.lua:3: show
```

You can change code in a test to try it out: `set('sys.move', 'src', 'return function(dt) ecs.quit() end')`
followed by `step()` and `events()` (→ `{'quit'}`).

Rust code that tests a program without a driver uses the same calls: see `lua-api.rust.md`.

---

## 13. Error codes

When a run fails, the message begins with a short code, then a hint. The ones you will meet:

| Code | When |
|---|---|
| `no_type: c` | `ecs.set` of a component that has no type in `world.lua` |
| `bad_int: c`, `bad_text: c`, `bad_num: c`, `not_a_number`, `not_finite`, `bad_value` | the value does not fit the kind (`text` must not be empty; `num` must not be nan or inf) |
| `no_entity: e` | a `ref`, `parent` or `is_a` that names an entity that does not exist |
| `exists: e` | `ecs.spawn(name, ...)` with a name already taken |
| `loop: parent` / `loop: is_a` | a loop in a tree or a prefab chain |
| `no_right: c for role` | the role may not write the component (ledger) |
| `forbidden: op for role` | the role may not run the operation |
| `out_of_scope: c` | a component that belongs to another entity's subtree |
| `readonly` | a write from a `draw/` file or from a read-only call |
| `time_limit` | the run took too long (default 250 ms) |
| `memory` | the run used more Lua memory than allowed |
| `rep_too_large`, `no_gc` | `string.rep` result too big; a `__gc` metatable |
| `too_deep` | `ecs.call` nested more than 8 deep |
| `bad_slot: ...` | a save slot with a wrong name |
| `no_slot`, `too_many_slots` | a missing / too many save slots |
| `gfx_outside_draw` | a `gfx.*` call outside a `draw/` file |
| `event_in_draw` | `ecs.quit()` from a `draw/` file |
| `too_many_cmds`, `too_many_events` | more than 20 000 drawing commands / 16 events in one tick |
| `bad_size` | `gfx.size` outside 1..10000 |
| `bad_arg: name` | a solver call with a wrong argument |
| `busy` | two solver calls inside each other |
| `bad_code: file ...` | a code file that does not compile or does not return a function |
| `type_in_use: c` | changing the kind of a component that already has values |

An error in an operation of a ledger world is **undone**: nothing it wrote stays. An error in a
tick of a game world stops that tick, and the program shows it on the status line.

---

## 14. Limits

- A world: `game` up to 1 000 000 entities and 4 000 000 values; `ledger` 200 000 and 2 000 000.
- Text 4000 characters; a name of an entity 64; a code file 64 KB, at most 2000 files in a program.
- A run: 250 ms wall time and a cap on Lua instructions; Lua memory 64 MB (game) / 16 MB (ledger).
- `ecs.call` depth 8; `history` and `journal` give at most 50 entries.
- At most 16 events (`quit`) and 20 000 drawing commands per tick; a text on the screen is clipped.
- Saves: a slot name of 32 characters, at most 20 slots of 4 MB.
