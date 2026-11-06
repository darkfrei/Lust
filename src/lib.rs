// lust: rust + lua + ecs + mcp. a base for programs whose logic is lua: a game engine, an
// erp, a simulation, a tool an AI works in. a program has as many worlds as it needs, all
// of the same kind; what each keeps is its Spec (a ledger: journal, roles, undo; a game:
// none of these, and a lua that remembers between ticks). what it gives:
//   ecs      a world: entities with a name or a number, components (their types are
//            entities too), text values checked by kind and num values in dense columns,
//            prefabs (is_a), rights per component, undo of a failed run, snapshots
//   journal  the module of the journal: every change as an entry of a hash chain, in the
//            worlds whose spec keeps one
//   host     a world with its own lua sandbox (no io, no os, a time and memory budget) and
//            the code of the world: operations (op.*), rules (system = true, after every
//            operation), ticks (tick = true, host.tick(dt)), views (view.*), rust systems
//   link     a result of one world as one operation of another
//   clock    the clock of a game loop: a variable or a fixed dt
//   store    the journal and snapshots on disk
//   mcp      (optional) a door for an AI over the Model Context Protocol: it sees what its
//            role may do and proposes; only the program, a person, confirms
// a program adds its own lua api (Host::add_global, Host::add_to_ecs) and keeps its own
// data in the lua state (mlua app data)

// the lua binding lust is built on, so a program adds its own api with the same types
pub use mlua;

pub mod clock;
pub mod ecs;
pub mod host;
pub mod journal;
pub mod link;
pub mod store;

// leave it out with default-features = false, or delete src/mcp.rs and these two lines
#[cfg(feature = "mcp")]
pub mod mcp;
