// the journal: a module of the world. a world keeps it when its Spec says journal = true
// (the ledger of an erp, a game whose every move is evidence); most games do not.
// every change of a world that keeps it is one entry: who (role), in which operation, what
// was changed, the new value, the hash of the entry before and its own hash. the entries
// form a chain: a changed, cut or reordered entry is found. the chain has no secret:
// whoever can write the file can compute the hashes again, so the hash of the last entry
// (World::head) should be kept somewhere else, or signed.
// the world writes the entries (ecs.rs: push, note, replay); the disk is store.rs

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

// an entry changes one component of one entity. value "" means the component was removed.
// entries that change nothing (a refusal, a rollback) have entity "-" and comp "-"
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub seq: u64,
    pub role: String,
    pub op: String,
    pub entity: String,
    pub comp: String,
    pub value: String,
    pub prev: String,
    pub hash: String,
}

// every field is length prefixed, so "a|b" + "c" can never equal "a" + "b|c"
pub fn entry_hash(prev: &str, seq: u64, role: &str, op: &str, entity: &str, comp: &str, value: &str) -> String {
    let mut h = Sha256::new();
    h.update(seq.to_le_bytes());
    for field in [prev, role, op, entity, comp, value] {
        h.update((field.len() as u64).to_le_bytes());
        h.update(field.as_bytes());
    }
    format!("{:x}", h.finalize())
}

// checks the whole chain; expected_head is the last hash somebody else holds,
// without it a cut-off tail of the journal would go unnoticed
pub fn verify(journal: &[Entry], expected_head: &str) -> Result<(), String> {
    let mut prev = GENESIS.to_string();
    for (i, e) in journal.iter().enumerate() {
        let seq = i as u64 + 1;
        if e.seq != seq || e.prev != prev || e.hash != entry_hash(&prev, seq, &e.role, &e.op, &e.entity, &e.comp, &e.value) {
            return Err(format!("broken at entry {}", seq));
        }
        prev = e.hash.clone();
    }
    if prev != expected_head {
        return Err("head differs: the journal was cut or replaced".to_string());
    }
    Ok(())
}
