// lust: rust + lua. the smallest part of radost that stands on its own:
//   ecs    the world: entities, components (their types are entities too), roles with
//          rights on each component, prefabs (is_a) and the journal as a hash chain
//   host   the lua sandbox (no io, no os, a time and memory budget per run) and the code
//          of the world: operations (op.*), systems (sys.*, after every operation), views
//          (view.*) and any read-only code; a failed run undoes everything it did
//   store  the journal on disk, one json line per change, replayed at start
//   mcp    (optional) a door for an AI over the Model Context Protocol: it sees what its
//          role may do and proposes; only the program, a person, confirms
pub mod ecs;
pub mod host;
pub mod store;

// leave it out with default-features = false, or delete src/mcp.rs and these two lines
#[cfg(feature = "mcp")]
pub mod mcp;
