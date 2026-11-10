# Lust: goals and plan (summary)

The full document, with the decisions worked out in detail, is [roadmap.ru.md](roadmap.ru.md)
(Russian). The rules for a model that continues the project are in `CLAUDE.md`.

Lust is a runtime: a program is a folder of Lua text files, and a language model builds and
changes it through tools, without touching the core.

## Goals (each has a measure in the full document)

1. Any program is built without the core: only files in one folder.
2. The model need not know the whole program: a guide of a fixed size, small files, a map.
3. Changes are hot: the running world takes them and keeps its values.
4. Safe by construction: the folder is the permission, a sandbox with limits, nothing open to
   the outside, the model cannot confirm or widen its rights.
5. Everything can be checked without eyes: frames, sounds and input are text.
6. No command line for the person: double click, a game as one file, saves.
7. It works with a weaker model too: a small API, errors that say what to do, samples.
8. A small core with replaceable modules.
9. The same base serves ledgers (Radost): journal, roles, undo.

Not goals: 3D, physics, an editor, microcontrollers, a language of its own, free disk or network
access for a program.

## Steps

| # | Step | Size | Who |
|---|---|---|---|
| 1 | Push what is done | S | N says yes |
| 2 | The guide by topics, tests against drift (core at most 6000 chars) | S | simple model |
| 3 | Subfolders (one level), file descriptions, big listings, lust_find | M | simple model |
| 4 | Errors that teach (a code, the place, one line of advice) | M | simple model |
| 5 | Sample tasks with hidden acceptance tests, lust eval | M | simple model |
| 6 | An event channel from the world to the display (exactly once): done | M | strong model |
| 7 | A window of its own (browser app mode); ecs.quit() is done | S | simple, checked by N |
| 8 | Images as text: art/<n>.txt, gfx.sprite | M | simple model |
| 9 | Sound as text: sfx/<n>.txt, sfx.play | M | simple model |
| 10 | Text input and widgets for applications | L | simple model, in parts |
| 11 | png and sound files | M | strong model |
| 12 | Network | L | strong model and N |
| 13 | Radost on this path | L | strong model and N |

Done so far: the core, a folder as a program, 11 MCP tools, hot edits, the browser display,
keys, mouse, microphone loudness, the built-in UI library, double click start, a game as one
file, saves, the game ends with its page.
