// the world: entities and components, and what a world keeps besides them (its Spec).
//
// every world is the same kind of thing; its Spec says what it keeps:
//   journal   every change as an entry of a hash chain (journal.rs). a ledger keeps it,
//             most games do not
//   roles     rights: who may write each component. off: anyone writes anything
//   undo      a failed operation or tick is undone as a whole
//   keep_lua  (for the host) lua globals live from run to run, as in a game loop
// Spec::ledger() is an erp or a game where every move counts, Spec::game() an arcade
// game; any other mix is allowed.
//
// an entity has a name ("door", "op.knock", "role.guest") and a number (its index and a
// generation: a number given again after an entity is gone does not reach the new one
// through an old id). an entity made without a name is called "#<n>". it exists while it
// has a component.
//
// a component is a store: entity -> value, and every entry of one component has the same
// rights. so the world is stored component by component. two ways to store:
//   - text kinds (text, code, int, bool, ref, name, names): checked on every write, in the
//     journal when the world keeps one
//   - num: a number (f64) in a dense column (a sparse set: the values lie one after
//     another, so a pass over a column is a pass over memory). never in the journal;
//     undone with the rest of a failed run; saved by snapshots. for what changes every
//     tick: positions, speeds, health
//
// component types are entities too: the entity "minute" with the components kind, desc and
// write describes the component "minute". a type has components of its own, and the
// developer edits types, rules, code and texts with the same calls as everything else.
// two extensions over a plain ECS:
//   - is_a: an entity takes the values of its prefab unless it has an own value
//     (a ticket of a party is_a the ticket template the developer wrote)
//   - a type that has a parent belongs to that entity: its component may be set
//     only on that entity, its children and their instances (not for num types)

use crate::journal::entry_hash;
pub use crate::journal::{verify, Entry, GENESIS};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const TEXT_MAX: usize = 4000;
pub const CODE_MAX: usize = 64 * 1024;
pub const ID_MAX: usize = 64;
pub const ENTITIES_MAX: usize = 200_000;
pub const VALUES_MAX: usize = 2_000_000;
const INT_MAX: i64 = 1_000_000_000_000;
const NAMES_MAX: usize = 32;
// how far is_a and parent chains are followed
pub const DEPTH_MAX: usize = 8;
// the role that writes the genesis entries; no script can act as it
pub const CORE_ROLE: &str = "core";
const NONE: u32 = u32::MAX;

// what a world keeps besides its values (see the top of this file)
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub journal: bool,
    pub roles: bool,
    pub undo: bool,
    pub keep_lua: bool,
    // the memory of the world's lua, in bytes
    pub memory: usize,
    pub entities_max: usize,
    pub values_max: usize,
}

impl Spec {
    // rules and decisions that must be kept: an erp, a turn-based game with replays
    pub fn ledger() -> Spec {
        Spec { journal: true, roles: true, undo: true, keep_lua: false, memory: 16 * 1024 * 1024, entities_max: ENTITIES_MAX, values_max: VALUES_MAX }
    }

    // a game that runs: no journal, no rights, no undo; lua remembers between ticks
    pub fn game() -> Spec {
        Spec { journal: false, roles: false, undo: false, keep_lua: true, memory: 64 * 1024 * 1024, entities_max: 1_000_000, values_max: 4_000_000 }
    }
}

impl Default for Spec {
    fn default() -> Self {
        Spec::ledger()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Text,  // 1..TEXT_MAX characters
    Code,  // lua source, up to CODE_MAX bytes
    Int,   // a whole number
    Bool,  // true or false
    Ref,   // the name of an existing entity
    Name,  // a name (same rules as an entity name), not checked for existence
    Names, // names separated by spaces
    Num,   // a number (f64), stored in a column
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "text" => Kind::Text,
            "code" => Kind::Code,
            "int" => Kind::Int,
            "bool" => Kind::Bool,
            "ref" => Kind::Ref,
            "name" => Kind::Name,
            "names" => Kind::Names,
            "num" => Kind::Num,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Code => "code",
            Kind::Int => "int",
            Kind::Bool => "bool",
            Kind::Ref => "ref",
            Kind::Name => "name",
            Kind::Names => "names",
            Kind::Num => "num",
        }
    }
}

fn clip_text(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

// an id: lower case letters, digits, _ and . (parts), - as in the names of mods (mods.saga-firm)
pub fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    id.len() <= ID_MAX && !id.ends_with('.') && !id.contains("..") && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.' || c == '-')
}

// length in characters, not bytes
pub fn valid_text(s: &str) -> bool {
    let n = s.chars().count();
    n > 0 && n <= TEXT_MAX
}

// the number of an entity made without a name: "#12" -> 12
pub fn anon_number(e: &str) -> Option<u64> {
    let digits = e.strip_prefix('#')?;
    if digits.is_empty() || digits.len() > 18 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

// a name an entity may have: an id, or "#<n>"
pub fn valid_entity(e: &str) -> bool {
    valid_id(e) || anon_number(e).is_some()
}

// a number as text, so that it reads back to the same number
pub fn fmt_num(v: f64) -> String {
    format!("{}", v)
}

// an entity as a number: the generation above the index
pub fn to_id(index: u32, generation: u32) -> i64 {
    ((generation as i64) << 32) | index as i64
}

// one num component: a sparse set. sparse[index] is the place in the dense arrays or NONE
#[derive(Default)]
pub struct Column {
    name: String,
    live: bool,
    sparse: Vec<u32>,
    ents: Vec<u32>,
    vals: Vec<f64>,
    // the roles that may write it (from the component write of its type)
    writers: Vec<String>,
}

impl Column {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn len(&self) -> usize {
        self.vals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vals.is_empty()
    }

    fn place(&self, index: u32) -> Option<usize> {
        match self.sparse.get(index as usize) {
            Some(&p) if p != NONE => Some(p as usize),
            _ => None,
        }
    }

    pub fn get(&self, index: u32) -> Option<f64> {
        self.place(index).map(|p| self.vals[p])
    }

    // true when the value is new
    fn put(&mut self, index: u32, v: f64) -> bool {
        match self.place(index) {
            Some(p) => {
                self.vals[p] = v;
                false
            }
            None => {
                let i = index as usize;
                if self.sparse.len() <= i {
                    self.sparse.resize(i + 1, NONE);
                }
                self.sparse[i] = self.vals.len() as u32;
                self.ents.push(index);
                self.vals.push(v);
                true
            }
        }
    }

    // swap with the last value: the dense arrays stay without holes
    fn take(&mut self, index: u32) -> Option<f64> {
        let p = self.place(index)?;
        let v = self.vals[p];
        let last = self.vals.len() - 1;
        self.vals.swap(p, last);
        self.ents.swap(p, last);
        self.vals.pop();
        self.ents.pop();
        if p < self.vals.len() {
            self.sparse[self.ents[p] as usize] = p as u32;
        }
        self.sparse[index as usize] = NONE;
        Some(v)
    }

    // the entity indexes, in the order of the values
    pub fn entities(&self) -> &[u32] {
        &self.ents
    }

    pub fn values(&self) -> &[f64] {
        &self.vals
    }

    // for a rust system of the program: it writes past rights, undo and limits
    pub fn values_mut(&mut self) -> &mut [f64] {
        &mut self.vals
    }

    pub fn reserve(&mut self, n: usize) {
        self.ents.reserve(n);
        self.vals.reserve(n);
        self.sparse.reserve(n);
    }
}

// one place for an entity: its name while it exists, and its generation
struct Slot {
    name: Option<String>,
    gen: u32,
}

// what a failed run has to put back, newest last
enum Undo {
    Value { entity: String, comp: String, before: Option<String> },
    Num { entity: String, col: usize, before: Option<f64> },
    Column { col: usize, vals: Vec<f64> },
    Freed { index: u32, gen: u32 },
}

// where a run began: what to undo and where the journal was
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    undo: usize,
    journal: usize,
}

pub struct World {
    pub spec: Spec,
    // text components: component -> entity -> value (canonical text)
    comps: BTreeMap<String, BTreeMap<String, String>>,
    // entity -> number of own components (text and num); an entity exists while it has one
    counts: BTreeMap<String, usize>,
    // index of every ref value: target -> (component, entity) that point at it.
    // children, instances and the check before a removal read it instead of scanning
    refs: BTreeMap<String, BTreeSet<(String, String)>>,
    values: usize,
    // the numbers of the entities
    ids: HashMap<String, u32>,
    slots: Vec<Slot>,
    free: Vec<u32>,
    anon_next: u64,
    // the name new_anon gave last, while that entity has no component yet: only this
    // nameless entity may be made by a write
    anon_born: Option<String>,
    // num components
    cols: Vec<Column>,
    col_ids: HashMap<String, usize>,
    pub journal: Vec<Entry>,
    // what the open run changed, so it can be undone (only while a run is open)
    undo: Vec<Undo>,
    depth: usize,
    restoring: bool,
    pub current_role: String,
    pub current_op: String,
    // set while read-only code runs (a view, a prompt card): nothing may change
    pub read_only: bool,
    // set while something is built for a reader that may not see private values: the
    // values of types marked local_ai (the text of a file, a wage) read as nothing
    pub hide_private: bool,
    // only measurements lift the limits
    unlimited: bool,
    // values a reader sees instead of the stored ones while the world is read-only:
    // (entity, component, value or None for no value). the host sets them around a
    // read-only run, for example one screen of several drawn from one world; never
    // journaled, the world does not change. text components only
    pub overlay: Vec<(String, String, Option<String>)>,
    // a world loaded from a snapshot holds only the entries after it: these say how many
    // came before and the hash of the last of them, so the chain goes on unbroken
    base_seq: u64,
    base_head: String,
}

// the state of a world: every value (num values as text), the number of journal entries so
// far and the hash of the last one, so a journal after it continues the same chain. a game
// saves this; a ledger that computes much saves it now and then and starts a new volume of
// its journal. state is the hash of the rest, to find a snapshot changed by hand
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub format: String,
    pub seq: u64,
    pub head: String,
    pub values: Vec<(String, String, String)>,
    pub state: String,
}

pub const SNAPSHOT_FORMAT: &str = "lust-snapshot-1";

fn snapshot_hash(seq: u64, head: &str, values: &[(String, String, String)]) -> String {
    let mut h = Sha256::new();
    h.update(SNAPSHOT_FORMAT.as_bytes());
    h.update(seq.to_string().as_bytes());
    h.update([0u8]);
    h.update(head.as_bytes());
    for (e, c, v) in values {
        for part in [e, c, v] {
            h.update([0u8]);
            h.update(part.len().to_string().as_bytes());
            h.update([0u8]);
            h.update(part.as_bytes());
        }
    }
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

// the types every world starts with. only the developer may change them (in a world with
// roles); the content pack widens the rights of parent and is_a for the roles that build
const GENESIS_TYPES: &[(&str, &str, &str, bool)] = &[
    ("kind", "name", "the type of a component: text, code, int, bool, ref, name, names or num", true),
    ("desc", "text", "what an entity is, in one sentence", false),
    ("write", "names", "the roles that may write a component", true),
    ("local", "bool", "the values of a component are not taken over by instances", true),
    ("parent", "ref", "the entity this one belongs to", true),
    ("is_a", "ref", "the prefab: values not set here are taken from it", true),
    ("ord", "int", "the place among siblings", false),
];

impl World {
    // a world with nothing in it, not even the genesis types
    fn empty(spec: Spec) -> World {
        World {
            spec,
            comps: BTreeMap::new(),
            counts: BTreeMap::new(),
            refs: BTreeMap::new(),
            values: 0,
            ids: HashMap::new(),
            slots: Vec::new(),
            free: Vec::new(),
            anon_next: 1,
            anon_born: None,
            cols: Vec::new(),
            col_ids: HashMap::new(),
            journal: Vec::new(),
            undo: Vec::new(),
            depth: 0,
            restoring: false,
            current_role: String::from("-"),
            current_op: String::from("-"),
            read_only: false,
            hide_private: false,
            unlimited: false,
            overlay: Vec::new(),
            base_seq: 0,
            base_head: GENESIS.to_string(),
        }
    }

    // a ledger: journal, roles, undo (Spec::ledger)
    pub fn new() -> World {
        World::with_spec(Spec::ledger())
    }

    // a world that keeps what the spec says
    pub fn with_spec(spec: Spec) -> World {
        let mut w = World::empty(spec);
        w.current_role = CORE_ROLE.to_string();
        w.current_op = String::from("genesis");
        for (name, kind, desc, local) in GENESIS_TYPES {
            w.raw_set(name, "kind", kind);
            w.raw_set(name, "desc", desc);
            w.raw_set(name, "write", "dev");
            if *local {
                w.raw_set(name, "local", "true");
            }
        }
        w.current_role = String::from("-");
        w.current_op = String::from("-");
        w
    }

    // the world a saved journal describes (a ledger). every entry must follow the one before
    // (seq, prev, hash); the values were checked when they were written, so they are put
    // back as they are. a journal edited by hand breaks the chain and is refused
    pub fn replay(entries: Vec<Entry>) -> Result<World, String> {
        let mut w = World::empty(Spec::ledger());
        w.apply_entries(entries)?;
        Ok(w)
    }

    fn apply_entries(&mut self, entries: Vec<Entry>) -> Result<(), String> {
        if !entries.is_empty() && !self.spec.journal {
            return Err(String::from("this world keeps no journal"));
        }
        for e in entries {
            let seq = self.seq() + 1;
            let prev = self.head();
            if e.seq != seq || e.prev != prev || e.hash != entry_hash(&prev, seq, &e.role, &e.op, &e.entity, &e.comp, &e.value) {
                return Err(format!("journal broken at entry {}", seq));
            }
            if e.comp != "-" {
                if !valid_entity(&e.entity) || !valid_id(&e.comp) {
                    return Err(format!("journal entry {} names no entity", seq));
                }
                let v = if e.value.is_empty() { None } else { Some(e.value.as_str()) };
                self.store(&e.entity, &e.comp, v);
            }
            self.journal.push(e);
        }
        Ok(())
    }

    // how many entries the journal has had: those before a snapshot and those since
    pub fn seq(&self) -> u64 {
        self.base_seq + self.journal.len() as u64
    }

    // every value now, text and num, sorted by entity and component
    fn value_list(&self) -> Vec<(String, String, String)> {
        let mut values = Vec::with_capacity(self.values);
        for (c, map) in &self.comps {
            for (e, v) in map {
                values.push((e.clone(), c.clone(), v.clone()));
            }
        }
        // num values after the text ones: their types (kind = num) are text and come first
        let mut nums = Vec::new();
        for col in self.cols.iter().filter(|c| c.live) {
            for (p, &i) in col.ents.iter().enumerate() {
                if let Some(name) = &self.slots[i as usize].name {
                    nums.push((name.clone(), col.name.clone(), fmt_num(col.vals[p])));
                }
            }
        }
        nums.sort();
        values.extend(nums);
        values
    }

    // the state as it is now (see Snapshot)
    pub fn snapshot(&self) -> Snapshot {
        let values = self.value_list();
        let (seq, head) = (self.seq(), self.head());
        let state = snapshot_hash(seq, &head, &values);
        Snapshot { format: SNAPSHOT_FORMAT.to_string(), seq, head, values, state }
    }

    // a ledger from a snapshot and the entries written after it (the journal volume begun
    // at the snapshot). the snapshot must be whole and the entries must continue its chain
    pub fn from_snapshot(snap: &Snapshot, after: Vec<Entry>) -> Result<World, String> {
        World::from_snapshot_spec(Spec::ledger(), snap, after)
    }

    // the same for a world of any spec (a game: no entries after it)
    pub fn from_snapshot_spec(spec: Spec, snap: &Snapshot, after: Vec<Entry>) -> Result<World, String> {
        if snap.format != SNAPSHOT_FORMAT {
            return Err(format!("not a snapshot of this kind: {}", clip_text(&snap.format, 40)));
        }
        if snapshot_hash(snap.seq, &snap.head, &snap.values) != snap.state {
            return Err(String::from("the snapshot was changed: its state hash differs"));
        }
        let mut w = World::empty(spec);
        for (e, c, v) in &snap.values {
            if !valid_entity(e) || !valid_id(c) || v.is_empty() {
                return Err(format!("the snapshot names no entity: {}.{}", clip_text(e, 40), clip_text(c, 40)));
            }
            if w.col_ids.contains_key(c) && !v.parse::<f64>().map(|x| x.is_finite()).unwrap_or(false) {
                return Err(format!("the snapshot has a bad number: {}.{}", clip_text(e, 40), clip_text(c, 40)));
            }
            w.store(e, c, Some(v));
        }
        w.reindex();
        w.base_seq = snap.seq;
        w.base_head = snap.head.clone();
        w.apply_entries(after)?;
        Ok(w)
    }

    // a world without the limits of size, for measurements only
    pub fn new_unlimited_for_bench() -> World {
        let mut w = World::new();
        w.lift_limits_for_bench();
        w
    }

    pub fn lift_limits_for_bench(&mut self) {
        self.unlimited = true;
    }

    // a value hidden from the code that runs now (see hide_private)
    pub fn hidden(&self, c: &str) -> bool {
        self.hide_private && self.own(c, "local_ai") == Some("true")
    }

    pub fn head(&self) -> String {
        match self.journal.last() {
            Some(e) => e.hash.clone(),
            None => self.base_head.clone(),
        }
    }

    // a journal entry, when the world keeps a journal
    fn push(&mut self, entity: &str, comp: &str, value: &str) {
        if !self.spec.journal {
            return;
        }
        let seq = self.seq() + 1;
        let prev = self.head();
        let hash = entry_hash(&prev, seq, &self.current_role, &self.current_op, entity, comp, value);
        self.journal.push(Entry {
            seq,
            role: self.current_role.clone(),
            op: self.current_op.clone(),
            entity: entity.to_string(),
            comp: comp.to_string(),
            value: value.to_string(),
            prev,
            hash,
        });
    }

    fn recording(&self) -> bool {
        self.spec.undo && self.depth > 0 && !self.restoring
    }

    fn record(&mut self, u: Undo) {
        if self.recording() {
            self.undo.push(u);
        }
    }

    // a value from a crystal: checked when it was first written, put back as it is, as the
    // role and operation set now (radost's crystal.rs). None removes it
    pub fn crystal_write(&mut self, e: &str, c: &str, v: Option<&str>) {
        let before = self.store(e, c, v);
        if !self.col_ids.contains_key(c) {
            self.record(Undo::Value { entity: e.to_string(), comp: c.to_string(), before });
            self.push(e, c, v.unwrap_or(""));
        }
    }

    // the reference index made again from the values: after values were put back in an
    // order where a type could come after its first values
    pub fn reindex(&mut self) {
        let mut refs: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
        for (c, map) in &self.comps {
            if self.kind_of(c) == Some(Kind::Ref) {
                for (e, v) in map {
                    refs.entry(v.clone()).or_default().insert((c.clone(), e.clone()));
                }
            }
        }
        self.refs = refs;
    }

    // every value of the world, entity by entity: to compare two worlds
    pub fn all_values(&self) -> BTreeMap<(String, String), String> {
        self.value_list().into_iter().map(|(e, c, v)| ((e, c), v)).collect()
    }

    // an entry that changes nothing: a refusal or a note (only in a world with a journal)
    pub fn note(&mut self, text: &str) {
        self.push("-", "-", text);
    }

    // writes without any check: genesis only
    fn raw_set(&mut self, e: &str, c: &str, v: &str) {
        self.store(e, c, Some(v));
        self.push(e, c, v);
    }

    // ---------- entities: names and numbers ----------

    fn count_up(&mut self, e: &str) {
        match self.counts.get_mut(e) {
            Some(n) => *n += 1,
            None => {
                self.counts.insert(e.to_string(), 1);
                self.alloc(e);
            }
        }
    }

    fn count_down(&mut self, e: &str) {
        let gone = match self.counts.get_mut(e) {
            Some(n) => {
                *n -= 1;
                *n == 0
            }
            None => false,
        };
        if gone {
            self.counts.remove(e);
            self.release(e);
        }
    }

    fn alloc(&mut self, e: &str) {
        let i = match self.free.pop() {
            Some(i) => i,
            None => {
                self.slots.push(Slot { name: None, gen: 1 });
                (self.slots.len() - 1) as u32
            }
        };
        self.slots[i as usize].name = Some(e.to_string());
        self.ids.insert(e.to_string(), i);
        if self.anon_born.as_deref() == Some(e) {
            self.anon_born = None;
        }
        if let Some(n) = anon_number(e) {
            if n >= self.anon_next {
                self.anon_next = n + 1;
            }
        }
    }

    fn release(&mut self, e: &str) {
        if let Some(i) = self.ids.remove(e) {
            let gen = self.slots[i as usize].gen;
            self.record(Undo::Freed { index: i, gen });
            let s = &mut self.slots[i as usize];
            s.name = None;
            s.gen = gen.wrapping_add(1).max(1);
            self.free.push(i);
        }
    }

    // a new name for an entity made without one: "#<n>", never given before in this world
    pub fn new_anon(&mut self) -> String {
        let n = self.anon_next;
        self.anon_next += 1;
        let name = format!("#{}", n);
        self.anon_born = Some(name.clone());
        name
    }

    // a nameless entity is made only right after new_anon gave its name: "#7" that is gone,
    // or was never given, stays gone
    fn is_dead_anon(&self, e: &str) -> bool {
        anon_number(e).is_some() && !self.exists(e) && self.anon_born.as_deref() != Some(e)
    }

    // the number of an entity that exists
    pub fn id(&self, e: &str) -> Option<i64> {
        self.ids.get(e).map(|&i| to_id(i, self.slots[i as usize].gen))
    }

    // the index behind a number, while that entity lives
    pub fn index_of(&self, id: i64) -> Option<u32> {
        let index = (id & 0xffff_ffff) as u32;
        let gen = ((id >> 32) & 0xffff_ffff) as u32;
        match self.slots.get(index as usize) {
            Some(s) if s.name.is_some() && s.gen == gen => Some(index),
            _ => None,
        }
    }

    pub fn index_by_name(&self, e: &str) -> Option<u32> {
        self.ids.get(e).copied()
    }

    pub fn name_at(&self, index: u32) -> Option<&str> {
        self.slots.get(index as usize).and_then(|s| s.name.as_deref())
    }

    pub fn id_at(&self, index: u32) -> i64 {
        to_id(index, self.slots.get(index as usize).map_or(0, |s| s.gen))
    }

    // ---------- storing ----------

    // puts a value (or removes it with None) and keeps the counts, the numbers and the
    // reference index; returns the old value. every change of the world passes here
    fn store(&mut self, e: &str, c: &str, v: Option<&str>) -> Option<String> {
        if let Some(&col) = self.col_ids.get(c) {
            let x = v.map(|s| s.trim().parse::<f64>().unwrap_or(0.0));
            return self.store_num(e, col, x).map(fmt_num);
        }
        let old = self.store_text(e, c, v);
        if c == "kind" || c == "write" {
            self.sync_type(e);
        }
        old
    }

    fn store_text(&mut self, e: &str, c: &str, v: Option<&str>) -> Option<String> {
        let is_ref = self.kind_of(c) == Some(Kind::Ref);
        let map = self.comps.entry(c.to_string()).or_default();
        let old = match v {
            Some(v) => map.insert(e.to_string(), v.to_string()),
            None => map.remove(e),
        };
        if map.is_empty() {
            self.comps.remove(c);
        }
        if is_ref {
            let key = (c.to_string(), e.to_string());
            if let Some(o) = &old {
                if let Some(set) = self.refs.get_mut(o) {
                    set.remove(&key);
                    if set.is_empty() {
                        self.refs.remove(o);
                    }
                }
            }
            if let Some(n) = v {
                self.refs.entry(n.to_string()).or_default().insert(key);
            }
        }
        match (&old, v) {
            (None, Some(_)) => {
                self.values += 1;
                self.count_up(e);
            }
            (Some(_), None) => {
                self.values -= 1;
                self.count_down(e);
            }
            _ => {}
        }
        old
    }

    fn store_num(&mut self, e: &str, col: usize, v: Option<f64>) -> Option<f64> {
        let old = self.ids.get(e).and_then(|&i| self.cols[col].get(i));
        match v {
            Some(x) => {
                if old.is_none() {
                    self.values += 1;
                    self.count_up(e);
                }
                let i = self.ids[e];
                self.cols[col].put(i, x);
            }
            None => {
                if old.is_some() {
                    let i = self.ids[e];
                    self.cols[col].take(i);
                    self.values -= 1;
                    self.count_down(e);
                }
            }
        }
        old
    }

    // a type became num (a column), stopped being num, or got new writers
    fn sync_type(&mut self, t: &str) {
        let is_num = self.own(t, "kind") == Some("num");
        let writers: Vec<String> = self.own(t, "write").map(|w| w.split(' ').map(|s| s.to_string()).collect()).unwrap_or_default();
        match (self.col_ids.get(t).copied(), is_num) {
            (Some(col), true) => self.cols[col].writers = writers,
            (Some(col), false) => {
                self.cols[col].live = false;
                self.col_ids.remove(t);
            }
            (None, true) => {
                // a column that was num before comes back under its old number
                let col = match self.cols.iter().position(|c| c.name == t) {
                    Some(col) => col,
                    None => {
                        self.cols.push(Column { name: t.to_string(), ..Column::default() });
                        self.cols.len() - 1
                    }
                };
                self.cols[col].live = true;
                self.cols[col].writers = writers;
                self.col_ids.insert(t.to_string(), col);
            }
            (None, false) => {}
        }
    }

    // ---------- reading ----------

    pub fn exists(&self, e: &str) -> bool {
        self.counts.contains_key(e)
    }

    pub fn count(&self) -> usize {
        self.counts.len()
    }

    pub fn entities(&self) -> impl Iterator<Item = &String> {
        self.counts.keys()
    }

    // the own value of a text component (num components: own_num)
    pub fn own(&self, e: &str, c: &str) -> Option<&str> {
        if let Some(v) = self.viewed(e, c) {
            return v;
        }
        self.comps.get(c).and_then(|m| m.get(e)).map(|s| s.as_str())
    }

    // the value an overlay gives while the world is read-only: Some(value or none)
    fn viewed(&self, e: &str, c: &str) -> Option<Option<&str>> {
        if !self.read_only || self.overlay.is_empty() {
            return None;
        }
        self.overlay.iter().find(|(oe, oc, _)| oe == e && oc == c).map(|(_, _, v)| v.as_deref())
    }

    pub fn kind_of(&self, c: &str) -> Option<Kind> {
        self.own(c, "kind").and_then(Kind::parse)
    }

    pub fn is_local(&self, c: &str) -> bool {
        self.own(c, "local") == Some("true")
    }

    // the value an entity has, its own or the one of its prefab chain,
    // and the entity it comes from (text components; num: World::num)
    pub fn get(&self, e: &str, c: &str) -> Option<(&str, &str)> {
        if let Some(v) = self.viewed(e, c) {
            return v.map(|x| (x, "ui"));
        }
        let map = self.comps.get(c)?;
        if let Some((k, v)) = map.get_key_value(e) {
            return Some((v.as_str(), k.as_str()));
        }
        if self.is_local(c) {
            return None;
        }
        let mut cur = e;
        for _ in 0..DEPTH_MAX {
            cur = self.own(cur, "is_a")?;
            if let Some((k, v)) = map.get_key_value(cur) {
                return Some((v.as_str(), k.as_str()));
            }
        }
        None
    }

    pub fn value(&self, e: &str, c: &str) -> Option<&str> {
        self.get(e, c).map(|(v, _)| v)
    }

    // a value of either store as text: the text, or the number written out
    pub fn value_text(&self, e: &str, c: &str) -> Option<String> {
        match self.col_ids.get(c) {
            Some(&col) => self.ids.get(e).and_then(|&i| self.num_at(i, col)).map(fmt_num),
            None => self.value(e, c).map(|s| s.to_string()),
        }
    }

    // the components an entity has itself, text and num
    pub fn own_comps(&self, e: &str) -> Vec<String> {
        let mut out: Vec<String> = self.comps.iter().filter(|(_, m)| m.contains_key(e)).map(|(c, _)| c.clone()).collect();
        if let Some(&i) = self.ids.get(e) {
            for col in self.cols.iter().filter(|c| c.live) {
                if col.get(i).is_some() {
                    out.push(col.name.clone());
                }
            }
        }
        out.sort();
        out
    }

    // own components and those taken over from the prefab chain: (comp, value, from)
    pub fn all_comps(&self, e: &str) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for c in self.comps.keys() {
            if let Some((v, from)) = self.get(e, c) {
                out.push((c.clone(), v.to_string(), from.to_string()));
            }
        }
        if let Some(&i) = self.ids.get(e) {
            for col in self.cols.iter().filter(|c| c.live) {
                if let Some(v) = col.get(i) {
                    out.push((col.name.clone(), fmt_num(v), e.to_string()));
                }
            }
        }
        out
    }

    // the entities that point at e through component c, from the index; sorted
    fn pointing(&self, e: &str, c: &str) -> Vec<String> {
        match self.refs.get(e) {
            None => Vec::new(),
            Some(set) => set.range((c.to_string(), String::new())..).take_while(|(k, _)| k == c).map(|(_, x)| x.clone()).collect(),
        }
    }

    // the entities that have an own value of a component, equal to v if given; sorted.
    // a ref value is found through the index, anything else by a scan of the component
    pub fn with(&self, c: &str, v: Option<&str>) -> Vec<String> {
        if let Some(&col) = self.col_ids.get(c) {
            let want = v.and_then(|x| x.trim().parse::<f64>().ok());
            if v.is_some() && want.is_none() {
                return Vec::new();
            }
            let column = &self.cols[col];
            let mut out: Vec<String> = column
                .ents
                .iter()
                .zip(column.vals.iter())
                .filter(|(_, &x)| want.is_none_or(|w| w == x))
                .filter_map(|(&i, _)| self.slots[i as usize].name.clone())
                .collect();
            out.sort();
            return out;
        }
        if let Some(x) = v {
            if self.kind_of(c) == Some(Kind::Ref) {
                return self.pointing(x, c);
            }
        }
        match self.comps.get(c) {
            None => Vec::new(),
            Some(m) => m.iter().filter(|(_, val)| v.is_none_or(|x| x == val.as_str())).map(|(e, _)| e.clone()).collect(),
        }
    }

    // the children of an entity, by ord (taken over too) and then by name
    pub fn children(&self, e: &str) -> Vec<String> {
        let mut kids = self.with("parent", Some(e));
        kids.sort_by_key(|k| (self.value(k, "ord").and_then(|o| o.parse::<i64>().ok()).unwrap_or(0), k.clone()));
        kids
    }

    // e is root itself, below it, or an instance of one of those
    fn within(&self, e: &str, root: &str) -> bool {
        let mut frontier = vec![e.to_string()];
        let mut seen = 0;
        while let Some(cur) = frontier.pop() {
            if cur == root {
                return true;
            }
            seen += 1;
            if seen > DEPTH_MAX * DEPTH_MAX {
                return false;
            }
            for link in ["parent", "is_a"] {
                if let Some(next) = self.own(&cur, link) {
                    frontier.push(next.to_string());
                }
            }
        }
        false
    }

    pub fn may_write(&self, role: &str, c: &str) -> bool {
        !self.spec.roles || role == CORE_ROLE || self.own(c, "write").is_some_and(|w| w.split(' ').any(|r| r == role))
    }

    // ---------- writing ----------

    // the canonical form of a value, or a short error code
    fn canonical(&self, e: &str, c: &str, kind: Kind, raw: &str) -> Result<String, String> {
        match kind {
            Kind::Text => {
                if valid_text(raw) {
                    Ok(raw.to_string())
                } else {
                    Err(format!("bad_text: {}", c))
                }
            }
            Kind::Code => {
                if !raw.is_empty() && raw.len() <= CODE_MAX {
                    Ok(raw.to_string())
                } else {
                    Err(format!("bad_code: {}", c))
                }
            }
            Kind::Int => match raw.trim().parse::<i64>() {
                Ok(n) if n.abs() <= INT_MAX => Ok(n.to_string()),
                _ => Err(format!("bad_int: {}", c)),
            },
            Kind::Num => match raw.trim().parse::<f64>() {
                Ok(x) if x.is_finite() => Ok(fmt_num(x)),
                _ => Err(format!("bad_num: {}", c)),
            },
            Kind::Bool => match raw {
                "true" | "false" => Ok(raw.to_string()),
                _ => Err(format!("bad_bool: {}", c)),
            },
            Kind::Name => {
                if valid_id(raw) {
                    Ok(raw.to_string())
                } else {
                    Err(format!("bad_name: {}", c))
                }
            }
            Kind::Names => {
                let parts: Vec<&str> = raw.split_whitespace().collect();
                if parts.is_empty() || parts.len() > NAMES_MAX || !parts.iter().all(|p| valid_id(p)) {
                    return Err(format!("bad_names: {}", c));
                }
                Ok(parts.join(" "))
            }
            Kind::Ref => {
                if !valid_entity(raw) {
                    return Err(format!("bad_ref: {}", c));
                }
                if !self.exists(raw) && raw != e {
                    return Err(format!("no_entity: {}", raw));
                }
                // parent and is_a must not form a loop
                if (c == "parent" || c == "is_a") && (raw == e || self.within(raw, e)) {
                    return Err(format!("loop: {}", c));
                }
                Ok(raw.to_string())
            }
        }
    }

    fn check_write(&self, e: &str, c: &str) -> Result<Kind, String> {
        if self.read_only {
            return Err(String::from("readonly"));
        }
        if !valid_entity(e) {
            return Err(String::from("bad_entity"));
        }
        if self.is_dead_anon(e) {
            return Err(format!("dead_entity: {}", e));
        }
        let kind = self.kind_of(c).ok_or_else(|| format!("no_type: {}", c))?;
        if !self.may_write(&self.current_role, c) {
            return Err(format!("no_right: {} for {}", c, self.current_role));
        }
        // a type that belongs to an entity is set only inside that entity
        if let Some(owner) = self.own(c, "parent") {
            if !self.within(e, owner) {
                return Err(format!("out_of_scope: {}", c));
            }
        }
        Ok(kind)
    }

    fn type_in_use(&self, t: &str) -> bool {
        self.comps.contains_key(t) || self.col_ids.get(t).is_some_and(|&c| !self.cols[c].is_empty())
    }

    // sets one component of one entity; creates the entity if it had no component yet
    pub fn set(&mut self, e: &str, c: &str, raw: &str) -> Result<(), String> {
        let kind = self.check_write(e, c)?;
        let v = self.canonical(e, c, kind, raw)?;
        if c == "kind" {
            let new = Kind::parse(&v).ok_or_else(|| String::from("bad_kind"))?;
            if self.type_in_use(e) && self.kind_of(e) != Some(new) {
                return Err(format!("type_in_use: {}", e));
            }
            if new == Kind::Num && self.own(e, "parent").is_some() {
                return Err(format!("num_type: {} belongs to an entity", e));
            }
        }
        if c == "parent" && self.col_ids.contains_key(e) {
            return Err(format!("num_type: {} cannot belong to an entity", e));
        }
        if kind == Kind::Num {
            let col = self.col_ids[c];
            let x = v.parse::<f64>().unwrap_or(0.0);
            return self.set_num_checked(e, col, x);
        }
        if self.own(e, c) == Some(v.as_str()) {
            return Ok(());
        }
        self.check_room(e, self.own(e, c).is_none())?;
        let before = self.store(e, c, Some(&v));
        self.record(Undo::Value { entity: e.to_string(), comp: c.to_string(), before });
        self.push(e, c, &v);
        Ok(())
    }

    fn check_room(&self, e: &str, new_value: bool) -> Result<(), String> {
        if self.unlimited {
            return Ok(());
        }
        if !self.exists(e) && self.counts.len() >= self.spec.entities_max {
            return Err(String::from("too_many_entities"));
        }
        if new_value && self.values >= self.spec.values_max {
            return Err(String::from("too_many_values"));
        }
        Ok(())
    }

    // names that point at e: refs to it, or e used as a component type
    fn uses_of(&self, e: &str) -> Option<String> {
        if self.type_in_use(e) {
            return Some(format!("type_in_use: {}", e));
        }
        if let Some(set) = self.refs.get(e) {
            if let Some((c, _)) = set.iter().find(|(_, k)| k != e) {
                return Some(format!("referenced: {} by {}", e, c));
            }
        }
        None
    }

    // removes an own component; the value of the prefab shows again
    pub fn del(&mut self, e: &str, c: &str) -> Result<(), String> {
        self.check_write(e, c)?;
        if let Some(&col) = self.col_ids.get(c) {
            let Some(i) = self.ids.get(e).copied() else { return Ok(()) };
            if self.cols[col].get(i).is_none() {
                return Ok(());
            }
            if self.counts.get(e) == Some(&1) {
                if let Some(why) = self.uses_of(e) {
                    return Err(why);
                }
            }
            let before = self.store_num(e, col, None);
            self.record(Undo::Num { entity: e.to_string(), col, before });
            return Ok(());
        }
        if self.own(e, c).is_none() {
            return Ok(());
        }
        if c == "kind" && self.type_in_use(e) {
            return Err(format!("type_in_use: {}", e));
        }
        // the last component: the entity disappears, nothing may point at it
        if self.counts.get(e) == Some(&1) {
            if let Some(why) = self.uses_of(e) {
                return Err(why);
            }
        }
        let before = self.store(e, c, None);
        self.record(Undo::Value { entity: e.to_string(), comp: c.to_string(), before });
        self.push(e, c, "");
        Ok(())
    }

    // removes an entity with all its own components
    pub fn kill(&mut self, e: &str) -> Result<(), String> {
        if !self.exists(e) {
            return Err(format!("no_entity: {}", e));
        }
        if let Some(why) = self.uses_of(e) {
            return Err(why);
        }
        if !self.children(e).is_empty() {
            return Err(format!("has_children: {}", e));
        }
        // "kind" last: removing it is refused while the entity is still a type in use
        let mut cs = self.own_comps(e);
        cs.sort_by_key(|c| c == "kind");
        for c in cs {
            self.del(e, &c)?;
        }
        Ok(())
    }

    // makes id an instance of prefab, and every child of the prefab an instance under id
    pub fn instantiate(&mut self, prefab: &str, id: &str, depth: usize) -> Result<(), String> {
        if depth > DEPTH_MAX {
            return Err(String::from("too_deep"));
        }
        if !self.exists(prefab) {
            return Err(format!("no_entity: {}", prefab));
        }
        if self.exists(id) {
            return Err(format!("exists: {}", id));
        }
        self.set(id, "is_a", prefab)?;
        for child in self.children(prefab) {
            let tail = child.strip_prefix(&format!("{}.", prefab)).unwrap_or_else(|| child.rsplit('.').next().unwrap_or(&child)).to_string();
            let cid = format!("{}.{}", id, tail);
            self.instantiate(&child, &cid, depth + 1)?;
            self.set(&cid, "parent", id)?;
        }
        Ok(())
    }

    // ---------- num components ----------

    // the number of a num component (its column)
    pub fn col(&self, c: &str) -> Option<usize> {
        self.col_ids.get(c).copied()
    }

    // a column by number, if it is a num component now
    pub fn live_col(&self, col: usize) -> Option<usize> {
        self.cols.get(col).filter(|c| c.live).map(|_| col)
    }

    pub fn column(&self, col: usize) -> &Column {
        &self.cols[col]
    }

    // for a rust system of the program: past rights, undo and limits
    pub fn column_mut(&mut self, col: usize) -> &mut Column {
        &mut self.cols[col]
    }

    // a num value hidden from the code that runs now
    pub fn col_hidden(&self, col: usize) -> bool {
        self.hide_private && self.own(&self.cols[col].name, "local_ai") == Some("true")
    }

    // the own value, or the one of the prefab chain
    pub fn num_at(&self, index: u32, col: usize) -> Option<f64> {
        let column = &self.cols[col];
        if let Some(v) = column.get(index) {
            return Some(v);
        }
        if self.is_local(&column.name) {
            return None;
        }
        let mut cur = self.name_at(index)?;
        for _ in 0..DEPTH_MAX {
            cur = self.own(cur, "is_a")?;
            let i = self.ids.get(cur)?;
            if let Some(v) = column.get(*i) {
                return Some(v);
            }
        }
        None
    }

    pub fn num(&self, e: &str, c: &str) -> Option<f64> {
        let col = self.col(c)?;
        let i = self.index_by_name(e)?;
        self.num_at(i, col)
    }

    fn may_write_col(&self, col: usize) -> bool {
        !self.spec.roles || self.current_role == CORE_ROLE || self.cols[col].writers.contains(&self.current_role)
    }

    fn check_col_write(&self, col: usize) -> Result<(), String> {
        if self.read_only {
            return Err(String::from("readonly"));
        }
        if !self.may_write_col(col) {
            return Err(format!("no_right: {} for {}", self.cols[col].name, self.current_role));
        }
        Ok(())
    }

    fn set_num_checked(&mut self, e: &str, col: usize, v: f64) -> Result<(), String> {
        let old = self.ids.get(e).and_then(|&i| self.cols[col].get(i));
        if old == Some(v) {
            return Ok(());
        }
        self.check_room(e, old.is_none())?;
        let before = self.store_num(e, col, Some(v));
        self.record(Undo::Num { entity: e.to_string(), col, before });
        Ok(())
    }

    // a num value of an entity by name; makes a named entity that does not exist yet
    pub fn set_num(&mut self, e: &str, col: usize, v: f64) -> Result<(), String> {
        if self.live_col(col).is_none() {
            return Err(String::from("bad_column"));
        }
        if !v.is_finite() {
            return Err(String::from("not_finite"));
        }
        self.check_col_write(col)?;
        if !valid_entity(e) {
            return Err(String::from("bad_entity"));
        }
        if self.is_dead_anon(e) {
            return Err(format!("dead_entity: {}", e));
        }
        self.set_num_checked(e, col, v)
    }

    // a num value of a living entity by its index: the fast way
    pub fn set_num_at(&mut self, index: u32, col: usize, v: f64) -> Result<(), String> {
        if !v.is_finite() {
            return Err(String::from("not_finite"));
        }
        self.check_col_write(col)?;
        let column = &mut self.cols[col];
        if let Some(p) = column.place(index) {
            let before = column.vals[p];
            if before == v {
                return Ok(());
            }
            column.vals[p] = v;
            if self.recording() {
                let entity = self.slots[index as usize].name.clone().unwrap_or_default();
                self.undo.push(Undo::Num { entity, col, before: Some(before) });
            }
            return Ok(());
        }
        let e = self.name_at(index).ok_or_else(|| String::from("dead_entity"))?.to_string();
        self.set_num_checked(&e, col, v)
    }

    // the entities that have all the columns (own values), walking the shortest one; the
    // values of each column in the same order
    pub fn query(&self, cols: &[usize]) -> (Vec<i64>, Vec<Vec<f64>>) {
        let mut out: Vec<Vec<f64>> = vec![Vec::new(); cols.len()];
        let mut ids: Vec<i64> = Vec::new();
        let Some(&shortest) = cols.iter().min_by_key(|&&c| self.cols[c].len()) else { return (ids, out) };
        let walk = &self.cols[shortest];
        ids.reserve(walk.len());
        for o in out.iter_mut() {
            o.reserve(walk.len());
        }
        for &index in &walk.ents {
            let places: Option<Vec<usize>> = cols.iter().map(|&c| self.cols[c].place(index)).collect();
            if let Some(places) = places {
                ids.push(self.id_at(index));
                for (k, &c) in cols.iter().enumerate() {
                    out[k].push(self.cols[c].vals[places[k]]);
                }
            }
        }
        (ids, out)
    }

    // the whole column before a change of many values, so a failed run can put it back
    fn record_column(&mut self, col: usize) {
        if self.recording() {
            let vals = self.cols[col].vals.clone();
            self.undo.push(Undo::Column { col, vals });
        }
    }

    // dst += src * k, for every entity that has both
    pub fn add_cols(&mut self, dst: usize, src: usize, k: f64) -> Result<(), String> {
        if !k.is_finite() {
            return Err(String::from("not_finite"));
        }
        self.check_col_write(dst)?;
        self.record_column(dst);
        if dst == src {
            for v in self.cols[dst].vals.iter_mut() {
                *v += *v * k;
            }
            return self.check_finite(dst);
        }
        let (d, s) = if dst < src {
            let (a, b) = self.cols.split_at_mut(src);
            (&mut a[dst], &b[0])
        } else {
            let (a, b) = self.cols.split_at_mut(dst);
            (&mut b[0], &a[src])
        };
        // the usual case: the same entities in the same order, one straight pass
        if d.ents == s.ents {
            for (a, b) in d.vals.iter_mut().zip(s.vals.iter()) {
                *a += *b * k;
            }
        } else {
            for p in 0..d.vals.len() {
                if let Some(b) = s.get(d.ents[p]) {
                    d.vals[p] += b * k;
                }
            }
        }
        self.check_finite(dst)
    }

    // every value of a column through f
    pub fn map_col(&mut self, col: usize, f: impl Fn(f64) -> f64) -> Result<(), String> {
        self.check_col_write(col)?;
        self.record_column(col);
        for v in self.cols[col].vals.iter_mut() {
            *v = f(*v);
        }
        self.check_finite(col)
    }

    // a column that overflowed to inf is set back to its last finite values when a run is
    // open (by undo); outside a run its values are clamped to the largest finite number
    fn check_finite(&mut self, col: usize) -> Result<(), String> {
        if self.cols[col].vals.iter().all(|v| v.is_finite()) {
            return Ok(());
        }
        for v in self.cols[col].vals.iter_mut() {
            if !v.is_finite() {
                *v = if v.is_nan() { 0.0 } else { v.signum() * f64::MAX };
            }
        }
        Err(String::from("not_finite"))
    }

    // ---------- runs: all or nothing ----------

    // a run begins: what it changes can be undone (when the spec says undo)
    pub fn begin(&mut self) -> Mark {
        self.depth += 1;
        Mark { undo: self.undo.len(), journal: self.journal.len() }
    }

    // the run went well
    pub fn commit(&mut self, _m: Mark) {
        self.end();
    }

    fn end(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        if self.depth == 0 {
            self.undo.clear();
        }
    }

    // the run failed: everything it did is undone (with undo), the journal entries it wrote
    // are taken back, and one entry says why. without undo what it did stays, and the entry
    // says it failed
    pub fn rollback(&mut self, m: Mark, reason: &str) {
        self.restoring = true;
        while self.undo.len() > m.undo {
            if let Some(u) = self.undo.pop() {
                self.put_back(u);
            }
        }
        self.restoring = false;
        if self.spec.undo {
            self.journal.truncate(m.journal);
        }
        self.end();
        let word = if self.spec.undo { "rollback" } else { "failed" };
        self.note(&format!("{}: {}", word, reason));
    }

    fn put_back(&mut self, u: Undo) {
        match u {
            Undo::Value { entity, comp, before } => {
                self.store(&entity, &comp, before.as_deref());
            }
            Undo::Num { entity, col, before } => {
                self.store_num(&entity, col, before);
            }
            Undo::Column { col, vals } => {
                if self.cols[col].vals.len() == vals.len() {
                    self.cols[col].vals = vals;
                }
            }
            Undo::Freed { index, gen } => {
                self.slots[index as usize].gen = gen;
            }
        }
    }
}

impl Default for World {
    fn default() -> Self {
        World::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // a world with a few types, as the developer would make them
    fn world() -> World {
        let mut w = World::new();
        w.current_role = String::from("dev");
        w.set("label", "kind", "text").unwrap();
        w.set("label", "write", "dev gm").unwrap();
        w.set("minutes", "kind", "int").unwrap();
        w.set("minutes", "write", "dev").unwrap();
        w.set("parent", "write", "dev gm").unwrap();
        w.set("is_a", "write", "dev gm").unwrap();
        w
    }

    #[test]
    fn genesis_is_a_world_that_describes_itself() {
        let w = World::new();
        assert_eq!(w.kind_of("kind"), Some(Kind::Name));
        assert_eq!(w.kind_of("parent"), Some(Kind::Ref));
        assert_eq!(w.value("desc", "write"), Some("dev"));
        assert!(verify(&w.journal, &w.head()).is_ok());
        assert!(w.journal.iter().all(|e| e.role == CORE_ROLE));
    }

    #[test]
    fn a_value_needs_a_type_and_the_right() {
        let mut w = world();
        assert_eq!(w.set("a", "color", "red"), Err("no_type: color".to_string()));
        w.current_role = String::from("clerk");
        assert_eq!(w.set("a", "label", "x"), Err("no_right: label for clerk".to_string()));
        w.current_role = String::from("gm");
        w.set("a", "label", "x").unwrap();
        assert_eq!(w.set("a", "minutes", "5"), Err("no_right: minutes for gm".to_string()));
        assert!(w.exists("a"));
    }

    #[test]
    fn values_are_checked_by_kind() {
        let mut w = world();
        assert_eq!(w.set("a", "minutes", "ten"), Err("bad_int: minutes".to_string()));
        w.set("a", "minutes", " 30 ").unwrap();
        assert_eq!(w.own("a", "minutes"), Some("30"));
        assert_eq!(w.set("a", "label", ""), Err("bad_text: label".to_string()));
        assert_eq!(w.set("a", "parent", "nobody"), Err("no_entity: nobody".to_string()));
        assert!(w.set("a", "local", "yes").is_err());
        let long: String = "\u{0436}".repeat(TEXT_MAX);
        assert!(w.set("b", "label", &long).is_ok(), "characters, not bytes");
        assert!(w.set("c", "label", &(long + "x")).is_err());
    }

    #[test]
    fn instances_take_values_of_their_prefab_until_they_have_their_own() {
        let mut w = world();
        w.set("tpl", "minutes", "60").unwrap();
        w.set("tpl", "label", "template").unwrap();
        w.set("x", "is_a", "tpl").unwrap();
        assert_eq!(w.get("x", "minutes"), Some(("60", "tpl")));
        // the developer changes the template: the instance follows
        w.set("tpl", "minutes", "40").unwrap();
        assert_eq!(w.value("x", "minutes"), Some("40"));
        // an own value wins, removing it shows the prefab's again
        w.set("x", "minutes", "10").unwrap();
        assert_eq!(w.get("x", "minutes"), Some(("10", "x")));
        w.del("x", "minutes").unwrap();
        assert_eq!(w.value("x", "minutes"), Some("40"));
        // local components are not taken over
        assert_eq!(w.value("x", "is_a"), Some("tpl"));
        w.set("tpl", "parent", "x").unwrap_err();
    }

    #[test]
    fn instantiate_copies_the_tree_of_a_prefab() {
        let mut w = world();
        w.set("tpl.t1", "label", "ticket").unwrap();
        w.set("tpl.t1.read", "parent", "tpl.t1").unwrap();
        w.set("tpl.t1.read", "minutes", "10").unwrap();
        w.set("tpl.t1.fix", "parent", "tpl.t1").unwrap();
        w.set("tpl.t1.fix", "minutes", "60").unwrap();
        w.set("tpl.t1.fix", "ord", "2").unwrap();
        w.set("tpl.t1.read", "ord", "1").unwrap();
        w.current_role = String::from("gm");
        w.set("p1", "label", "party").unwrap();
        w.instantiate("tpl.t1", "p1.t1", 0).unwrap();
        w.set("p1.t1", "parent", "p1").unwrap();
        assert_eq!(w.children("p1.t1"), vec!["p1.t1.read", "p1.t1.fix"]);
        assert_eq!(w.value("p1.t1.fix", "minutes"), Some("60"));
        assert_eq!(w.instantiate("tpl.t1", "p1.t1", 0), Err("exists: p1.t1".to_string()));
    }

    #[test]
    fn a_type_can_belong_to_one_entity() {
        let mut w = world();
        w.set("kim", "label", "Kim").unwrap();
        w.set("anna", "label", "Anna").unwrap();
        // kim has a component of her own
        w.set("kim.patience", "kind", "int").unwrap();
        w.set("kim.patience", "write", "dev").unwrap();
        w.set("kim.patience", "parent", "kim").unwrap();
        w.set("kim", "kim.patience", "3").unwrap();
        assert_eq!(w.set("anna", "kim.patience", "3"), Err("out_of_scope: kim.patience".to_string()));
        // an instance of kim may have it too
        w.set("kim2", "is_a", "kim").unwrap();
        w.set("kim2", "kim.patience", "5").unwrap();
    }

    #[test]
    fn loops_and_dangling_names_are_refused() {
        let mut w = world();
        w.set("a", "label", "a").unwrap();
        w.set("b", "parent", "a").unwrap();
        assert_eq!(w.set("a", "parent", "b"), Err("loop: parent".to_string()));
        assert_eq!(w.set("a", "is_a", "a"), Err("loop: is_a".to_string()));
        assert_eq!(w.kill("a"), Err("referenced: a by parent".to_string()));
        assert_eq!(w.kill("label"), Err("type_in_use: label".to_string()));
        w.kill("b").unwrap();
        w.kill("a").unwrap();
        assert!(!w.exists("a"));
        assert_eq!(w.set("minutes", "kind", "text"), Ok(()), "not in use yet");
        w.set("q", "minutes", "x").unwrap();
        assert_eq!(w.set("minutes", "kind", "int"), Err("type_in_use: minutes".to_string()));
    }

    #[test]
    fn rollback_restores_everything() {
        let mut w = world();
        w.set("a", "minutes", "1").unwrap();
        let len = w.journal.len();
        let head = w.head();
        let m = w.begin();
        w.set("a", "minutes", "2").unwrap();
        w.set("b", "minutes", "3").unwrap();
        w.del("a", "minutes").unwrap();
        w.rollback(m, "test");
        assert_eq!(w.own("a", "minutes"), Some("1"));
        assert!(!w.exists("b"));
        assert_eq!(w.journal[len - 1].hash, head);
        assert_eq!(w.journal.last().unwrap().value, "rollback: test");
        assert!(verify(&w.journal, &w.head()).is_ok());
    }

    #[test]
    fn same_value_writes_nothing_and_read_only_writes_nothing() {
        let mut w = world();
        w.set("a", "minutes", "1").unwrap();
        let n = w.journal.len();
        w.set("a", "minutes", "1").unwrap();
        assert_eq!(w.journal.len(), n);
        w.read_only = true;
        assert_eq!(w.set("a", "minutes", "2"), Err("readonly".to_string()));
        assert_eq!(w.del("a", "minutes"), Err("readonly".to_string()));
    }

    #[test]
    fn the_journal_is_a_hash_chain_that_holds_the_role() {
        let mut w = world();
        w.set("a", "minutes", "1").unwrap();
        let head = w.head();
        let mut forged = w.journal.clone();
        let last = forged.len() - 1;
        forged[last].role = String::from("gm");
        assert!(verify(&forged, &head).is_err());
        forged = w.journal.clone();
        forged[last].value = String::from("2");
        assert!(verify(&forged, &head).is_err());
        assert!(verify(&w.journal[..last], &head).is_err(), "a cut tail is seen");
        assert!(verify(&w.journal, &head).is_ok());
    }

    #[test]
    fn names_are_strict() {
        for bad in ["", "A", "1a", "-a", "a b", "a..b", "a."] {
            assert!(!valid_id(bad), "{}", bad);
        }
        for good in ["a", "tpl.t1.read", "p1", "e_no_time", "mods.saga-firm"] {
            assert!(valid_id(good), "{}", good);
        }
    }

    #[test]
    fn a_replayed_journal_gives_the_same_world() {
        let mut w = world();
        w.set("tpl", "minutes", "60").unwrap();
        w.set("x", "is_a", "tpl").unwrap();
        w.set("x", "minutes", "5").unwrap();
        w.del("x", "minutes").unwrap();
        w.note("refused: something");
        let copy = World::replay(w.journal.clone()).unwrap();
        assert_eq!(copy.head(), w.head());
        assert_eq!(copy.value("x", "minutes"), Some("60"));
        assert_eq!(copy.own("x", "minutes"), None);
        assert_eq!(copy.kind_of("minutes"), Some(Kind::Int));
        assert_eq!(copy.entities().count(), w.entities().count());
        // the replayed world goes on with the same rules
        let mut copy = copy;
        copy.current_role = String::from("clerk");
        assert!(copy.set("tpl", "minutes", "1").is_err());
        // an edited entry is refused
        let mut bad = w.journal.clone();
        let n = bad.len() - 3;
        bad[n].value = String::from("600");
        assert!(World::replay(bad).err().unwrap().starts_with("journal broken"));
    }

    // the index must always say the same as a full scan, also after undo and replay
    fn scan_refs(w: &World) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for (c, m) in &w.comps {
            if w.kind_of(c) == Some(Kind::Ref) {
                for (e, v) in m {
                    out.push((v.clone(), c.clone(), e.clone()));
                }
            }
        }
        out.sort();
        out
    }

    fn index_refs(w: &World) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for (t, set) in &w.refs {
            for (c, e) in set {
                out.push((t.clone(), c.clone(), e.clone()));
            }
        }
        out.sort();
        out
    }

    #[test]
    fn the_reference_index_agrees_with_a_scan() {
        let mut w = world();
        w.set("a", "label", "a").unwrap();
        w.set("b", "parent", "a").unwrap();
        w.set("c", "parent", "a").unwrap();
        w.set("c", "is_a", "b").unwrap();
        assert_eq!(index_refs(&w), scan_refs(&w));
        assert_eq!(w.children("a"), vec!["b", "c"]);
        assert_eq!(w.with("is_a", Some("b")), vec!["c"]);
        let m = w.begin();
        w.set("d", "parent", "b").unwrap();
        w.del("c", "is_a").unwrap();
        w.set("c", "parent", "b").unwrap();
        assert_eq!(w.children("b"), vec!["c", "d"]);
        assert_eq!(index_refs(&w), scan_refs(&w));
        w.rollback(m, "test");
        assert_eq!(index_refs(&w), scan_refs(&w));
        assert_eq!(w.children("a"), vec!["b", "c"]);
        assert!(w.children("b").is_empty());
        let r = World::replay(w.journal.clone()).unwrap();
        assert_eq!(index_refs(&r), scan_refs(&r));
        assert_eq!(index_refs(&r), index_refs(&w));
        w.kill("c").unwrap();
        w.kill("b").unwrap();
        assert_eq!(index_refs(&w), scan_refs(&w));
        assert!(!w.refs.contains_key("a"));
    }

    #[test]
    fn a_snapshot_and_the_entries_after_it_give_the_same_world() {
        let mut w = world();
        w.current_role = String::from("dev");
        w.set("stock", "kind", "int").unwrap();
        w.set("stock", "write", "dev").unwrap();
        w.set("tea", "stock", "5").unwrap();
        let snap = w.snapshot();
        let at = w.journal.len();
        w.set("tea", "stock", "4").unwrap();
        w.set("coffee", "stock", "2").unwrap();
        let after: Vec<Entry> = w.journal[at..].to_vec();
        let mut back = World::from_snapshot(&snap, after.clone()).unwrap();
        assert_eq!(back.all_values(), w.all_values());
        assert_eq!(back.head(), w.head());
        assert_eq!(back.seq(), w.seq());
        // it goes on: the next entry continues the same chain in both
        back.current_role = String::from("dev");
        w.set("tea", "stock", "3").unwrap();
        back.set("tea", "stock", "3").unwrap();
        assert_eq!(back.head(), w.head());
        // a changed snapshot and entries that do not follow it are refused
        let mut bad = snap.clone();
        bad.values[0].2 = String::from("x");
        assert!(World::from_snapshot(&bad, Vec::new()).err().unwrap_or_default().contains("changed"));
        assert!(World::from_snapshot(&snap, after[1..].to_vec()).err().unwrap_or_default().contains("broken"));
        // undo works after a snapshot too
        let m = back.begin();
        back.set("tea", "stock", "1").unwrap();
        back.rollback(m, "test");
        assert_eq!(back.value("tea", "stock"), Some("3"));
    }

    // ---------- worlds of other specs, num components, nameless entities ----------

    fn game() -> World {
        let mut w = World::with_spec(Spec::game());
        for (t, k) in [("x", "num"), ("vx", "num"), ("hp", "num"), ("label", "text")] {
            w.set(t, "kind", k).unwrap();
        }
        w
    }

    #[test]
    fn a_game_world_keeps_no_journal_and_checks_no_roles() {
        let mut w = game();
        assert!(w.journal.is_empty());
        w.current_role = String::from("anybody");
        w.set("ball", "x", "1.5").unwrap();
        w.set("ball", "label", "the ball").unwrap();
        w.note("nothing");
        assert!(w.journal.is_empty());
        assert_eq!(w.num("ball", "x"), Some(1.5));
        assert_eq!(w.value_text("ball", "x"), Some(String::from("1.5")));
        // kinds still hold
        assert_eq!(w.set("ball", "x", "fast"), Err(String::from("bad_num: x")));
        assert_eq!(w.set("ball", "nothing", "1"), Err(String::from("no_type: nothing")));
        assert!(World::replay(Vec::new()).is_ok());
    }

    #[test]
    fn num_values_live_in_columns_and_take_over_from_a_prefab() {
        let mut w = game();
        let x = w.col("x").unwrap();
        w.set("tpl", "hp", "10").unwrap();
        w.set("orc", "is_a", "tpl").unwrap();
        assert_eq!(w.num("orc", "hp"), Some(10.0));
        w.set_num("orc", x, 3.0).unwrap();
        let i = w.index_by_name("orc").unwrap();
        w.set_num_at(i, x, 4.0).unwrap();
        assert_eq!(w.column(x).values(), &[4.0]);
        assert_eq!(w.own_comps("orc"), vec!["is_a", "x"]);
        assert_eq!(w.with("x", None), vec!["orc"]);
        assert!(w.set_num("orc", x, f64::NAN).is_err());
        // a type in use keeps its kind; a num type belongs to no entity
        assert!(w.set("x", "kind", "int").is_err());
        assert!(w.set("x", "parent", "orc").is_err());
        w.kill("orc").unwrap();
        assert!(!w.exists("orc"));
        assert!(w.column(x).is_empty());
    }

    #[test]
    fn a_nameless_entity_has_a_number_and_an_old_number_misses_the_new_one() {
        let mut w = game();
        let x = w.col("x").unwrap();
        let a = w.new_anon();
        w.set_num(&a, x, 1.0).unwrap();
        let id = w.id(&a).unwrap();
        assert_eq!(w.index_of(id).and_then(|i| w.name_at(i)), Some(a.as_str()));
        w.kill(&a).unwrap();
        assert_eq!(w.index_of(id), None);
        // a gone nameless entity cannot come back by its name
        assert!(w.set_num(&a, x, 2.0).unwrap_err().starts_with("dead_entity"));
        let b = w.new_anon();
        w.set_num(&b, x, 2.0).unwrap();
        let id_b = w.id(&b).unwrap();
        assert_ne!(a, b);
        assert_eq!(id_b & 0xffff_ffff, id & 0xffff_ffff, "the index is given again");
        assert_eq!(w.index_of(id), None, "but not to the old number");
    }

    #[test]
    fn a_failed_run_undoes_num_values_whole_columns_and_kills() {
        let mut w = World::new();
        w.current_role = String::from("dev");
        for (t, k) in [("x", "num"), ("vx", "num")] {
            w.set(t, "kind", k).unwrap();
            w.set(t, "write", "dev").unwrap();
        }
        let (x, vx) = (w.col("x").unwrap(), w.col("vx").unwrap());
        w.set("a", "x", "1").unwrap();
        w.set("a", "vx", "10").unwrap();
        w.set("b", "x", "2").unwrap();
        let id_b = w.id("b").unwrap();
        let before = w.all_values();
        let journal = w.journal.len();
        let m = w.begin();
        w.set_num("a", x, 5.0).unwrap();
        w.add_cols(x, vx, 0.5).unwrap();
        w.kill("b").unwrap();
        w.set("c", "x", "9").unwrap();
        w.rollback(m, "test");
        assert_eq!(w.all_values(), before);
        assert_eq!(w.id("b"), Some(id_b), "b has its old number again");
        assert_eq!(w.journal.len(), journal + 1, "num values are not in the journal, only the note");
        // a role without the right
        w.current_role = String::from("clerk");
        assert!(w.set_num("a", x, 1.0).unwrap_err().starts_with("no_right"));
        assert!(w.add_cols(x, vx, 1.0).is_err());
    }

    #[test]
    fn without_undo_a_failed_run_stays_and_says_so() {
        let mut spec = Spec::ledger();
        spec.undo = false;
        let mut w = World::with_spec(spec);
        w.current_role = String::from("dev");
        w.set("n", "kind", "int").unwrap();
        w.set("n", "write", "dev").unwrap();
        let m = w.begin();
        w.set("a", "n", "1").unwrap();
        w.rollback(m, "test");
        assert_eq!(w.own("a", "n"), Some("1"));
        assert_eq!(w.journal.last().unwrap().value, "failed: test");
    }

    #[test]
    fn a_snapshot_keeps_num_values_and_nameless_entities() {
        let mut w = game();
        let x = w.col("x").unwrap();
        for i in 0..5 {
            let e = w.new_anon();
            w.set_num(&e, x, i as f64 * 0.1).unwrap();
        }
        w.kill("#2").unwrap();
        w.set("paddle", "x", "0.3333333333333333").unwrap();
        w.set("paddle", "label", "the paddle").unwrap();
        let snap = w.snapshot();
        let mut back = World::from_snapshot_spec(Spec::game(), &snap, Vec::new()).unwrap();
        assert_eq!(back.all_values(), w.all_values());
        assert_eq!(back.num("paddle", "x"), Some(1.0 / 3.0));
        assert_eq!(back.snapshot().state, snap.state, "the same state gives the same hash");
        // a new nameless entity after loading gets a name never given before
        assert_eq!(back.new_anon(), "#6");
        let mut bad = snap.clone();
        let last = bad.values.len() - 1;
        bad.values[last].2 = String::from("nan");
        assert!(World::from_snapshot_spec(Spec::game(), &bad, Vec::new()).is_err());
    }

    #[test]
    fn a_journal_with_nameless_entities_replays() {
        let mut w = World::new();
        w.current_role = String::from("dev");
        w.set("label", "kind", "text").unwrap();
        w.set("label", "write", "dev").unwrap();
        let a = w.new_anon();
        w.set(&a, "label", "first").unwrap();
        w.set("box", "parent", &a).unwrap_or_else(|e| panic!("{}", e));
        let copy = World::replay(w.journal.clone()).unwrap();
        assert_eq!(copy.all_values(), w.all_values());
        assert_eq!(copy.children(&a), vec!["box"]);
    }
}
