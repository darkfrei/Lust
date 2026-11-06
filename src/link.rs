// worlds are equal and independent; this is the one way between them. a world computes
// something (a battle, a turn of an ai, a physics scene) and gives a result (ecs.commit);
// the program runs it in another world as one operation of a role, checked like any other
// operation and written to that world's journal if it keeps one (commit_to).
//
// a Seed remembers what a world read from another when it was made. while it computed,
// the other may have changed (the player sold a unit, the king sent reinforcements); then
// the result was computed from old values and is refused as stale

use crate::ecs::{valid_id, World};
use crate::host::{clip, Host};

// what one world read from another
#[derive(Default, Debug)]
pub struct Seed {
    reads: Vec<(String, String, Option<String>)>,
}

impl Seed {
    pub fn new() -> Seed {
        Seed::default()
    }

    // reads a value (text, or a number written out) and remembers it
    pub fn read(&mut self, w: &World, e: &str, c: &str) -> Option<String> {
        let v = w.value_text(e, c);
        self.reads.push((e.to_string(), c.to_string(), v.clone()));
        v
    }

    // the first value read that has changed since, as "entity.component"
    pub fn stale(&self, w: &World) -> Option<String> {
        for (e, c, v) in &self.reads {
            if w.value_text(e, c) != *v {
                return Some(format!("{}.{}", e, c));
            }
        }
        None
    }
}

// a result of world `from` as one operation of `target`: refused when the seed is stale,
// otherwise run as the role, between the journal notes "from world <from>" and "end world
// <from> ok|failed" (in a target that keeps a journal)
pub fn commit_to(target: &Host, from: &str, role: &str, op: &str, seed: &Seed, result: &[(String, String)]) -> Result<(), String> {
    let stale = seed.stale(&target.world.borrow());
    if let Some(what) = stale {
        let reason = format!("stale: {} changed while world {} computed", what, clip(from, 40));
        let mut w = target.world.borrow_mut();
        w.current_role = if valid_id(role) { role.to_string() } else { String::from("-") };
        w.current_op = if valid_id(op) { op.to_string() } else { String::from("-") };
        w.note(&format!("refused: {}", clip(&reason, 120)));
        return Err(reason);
    }
    let args: Vec<(&str, &str)> = result.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    target.run_from(&format!("world {}", from), role, op, &args)
}
