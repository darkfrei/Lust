# Sample tasks: can a model build a program from prompts in words alone?

`lines/` is the game Lines (Color Lines) in five steps. Each step is one prompt: `common.md`
plus `stepN.md` (Russian, words only, no code). The model gets nothing but the MCP tools of
`lust dev` on an empty folder. `accept/N.lua` are acceptance tests the model never sees; they
test behaviour through the names the prompt fixes (`app.screen`, the cells `c_<row>_<col>`
with `ball`, `app.score`, `app.best`) and through the frame.

To check a built folder: copy `accept/1.lua .. N.lua` into its `test/` folder (in a copy of
the folder) and run `lust test <copy>`.

The acceptance tests use no string patterns (the sandbox has none) and do not depend on where
the 3 new balls of a move land.

## Results

| date | model | client | step | tool calls | passed |
|---|---|---|---|---|---|
| 2026-11-10 | claude-haiku-5-5 | a bridge to lust dev (stdio MCP) | 1 menu | 31 | yes, first try |
| 2026-11-10 | claude-haiku-5-5 | Claude Code, only the lust tools | 2 board | 20 | yes, first try |
| 2026-11-10 | claude-haiku-5-5 | Claude Code, only the lust tools | 3 moves | 27 | yes, first try |
| 2026-11-10 | claude-haiku-5-5 | Claude Code, only the lust tools | 4 lines | 23 | yes, first try |
| 2026-11-10 | claude-haiku-5-5 | Claude Code, only the lust tools | 5 game over, best score | 36 | yes, first try |

Each step was a fresh session: the model knew the project only from the guide, the files and
its own notes.md. All five acceptance tests passed 30 runs in a row at the end.

What the runs showed about Lust (fixed after them): in the test driver a mouse button going
down was not a click without `press('mouse')` (the model wrote its own click detection); the
driver gave a bare Lua error for `ecs` used in driver code. Still open: a save holds the whole
world, so a game that keeps only a best score must reset the rest after a load.
