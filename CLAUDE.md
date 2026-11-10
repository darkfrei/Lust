# Lust: rules for the model that continues this project

Lust is a runtime (Rust + Lua): a program is a folder of Lua files that a language model builds
through MCP tools, hot, without touching the Rust core. Radost (an ERP and game platform) is
built on it. The owner is N (darkfrei), on Windows.

Read first: `docs/roadmap.ru.md` (goals, decisions already made, the plan by steps),
then `docs/dev.md` (how a project folder works). Do the next open step of the plan, one step at
a time. The hard decisions are made there: follow them. If one does not fit, stop and ask N.

## Layout

- `src/ecs.rs`, `src/host.rs`: the core (world, sandbox). Change only for the core itself.
- `src/project.rs`: a folder as a program, hot sync, history. `src/dev.rs`: the MCP tools and
  the guide text the model reads. `src/driver.rs`: the test driver (lust_try, test/*.lua).
- `src/screen.rs`: frame as text, input. `src/web.rs`: the browser display. `src/saves.rs`,
  `src/pack.rs`, `src/launcher.rs`, `src/bin/lust.rs` (the program lust).
- `src/lua/ui.lua`: the built-in UI library. `examples/pong`, `examples/menu`: sample projects.

## Checks (all must pass before you say a step is done)

```
cargo test --release
cargo clippy --release --all-targets        # 0 warnings
target/release/lust test examples/pong
target/release/lust test examples/menu
```

When you touch ecs.rs, host.rs, journal.rs, store.rs or mcp.rs, also run the tests of Radost.
What a person sees (the page) you check in a browser, or you say plainly that it is not checked.

## Rules that must hold (docs/roadmap.ru.md, section 3)

The model writes only inside the project folder. The MCP door and the page listen on 127.0.0.1
only. The model never confirms and has no tool for it; `--confirm KEY` exists only in our own
test tool. Code that does not compile is not saved. Everything has a limit. `draw/` code only
reads. Only numbers and text reach the world (loudness, messages), never devices. Every feature
has a test without a screen. Every Lua function is in the guide, and the guide names nothing
that does not exist. The guide has a size budget.

## Style

- Comments: English, lowercase, plain ASCII, no em dashes. No Cyrillic in source code (data and
  texts shown to the player may be in any language).
- Lua: no goto, no nested function definitions, no anonymous functions in table.sort.
- Rust: no new dependencies without asking N. Small functions, errors as short codes plus a hint.
- Deliver complete files. Keep README.md and README.ru.md, docs/*.md and docs/*.ru.md in step.

## Working with N

- Answer in Russian. Explain every term in simple words. Say the blunt truth, also when it goes
  against his proposal. End with concrete options to choose from.
- Never hand over a result you know is broken. Say what you did not verify.
- No command line for N: he double clicks. Files for him go to `D:\Claude\Radost`
  (`build-lust.bat` builds lust.exe from `lust-src.zip`). Downloads stay under 50 MB.
- Do not push to GitHub without his word. Show first.
- When N wants a game built, write him a prompt in words (no code) for the building model, and
  test that the prompt is enough by running it through `lust dev` yourself.
