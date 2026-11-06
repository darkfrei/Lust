// the world: entities, components and the journal.
//
// an entity is only a name. a component is a key-value store: entity -> value,
// and every entry of one component has the same rights. so the world is stored
// component by component.
//
// component types are entities too: the entity "minute" with the components
// kind, desc and write describes the component "minute". a type has components
// of its own, and the developer edits types, rules, code and texts with the same
// calls as everything else. two extensions over a plain ECS:
//   - is_a: an entity takes the values of its prefab unless it has an own value
//     (a ticket of a party is_a the ticket template the developer wrote)
//   - a type that has a parent belongs to that entity: its component may be set
//     only on that entity, its children and their instances
//
// every change is a journal entry; the entries form a hash chain

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Text,  // 1..TEXT_MAX characters
    Code,  // lua source, up to CODE_MAX bytes
    Int,   // a whole number
    Bool,  // true or false
    Ref,   // the name of an existing entity
    Name,  // a name (same rules as an entity name), not checked for existence
    Names, // names separated by spaces
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
        }
    }
}

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

struct Undo {
    entity: String,
    comp: String,
    before: Option<String>,
}

pub struct World {
    // component -> entity -> value (canonical text)
    comps: BTreeMap<String, BTreeMap<String, String>>,
    // entity -> number of own components; an entity exists while it has one
    counts: BTreeMap<String, usize>,
    // index of every ref value: target -> (component, entity) that point at it.
    // children, instances and the check before a removal read it instead of scanning
    refs: BTreeMap<String, BTreeSet<(String, String)>>,
    values: usize,
    pub journal: Vec<Entry>,
    // what each journal entry replaced, so a failed operation can be undone
    undo: Vec<Option<Undo>>,
    pub current_role: String,
    pub current_op: String,
    // set while read-only code runs (a view, a prompt card): nothing may change
    pub read_only: bool,
    // set while something is built for a reader that may not see private values: the
    // values of types marked local_ai (the text of a file, a wage) read as nothing
    pub hide_private: bool,
    // only the measurements in bench.rs lift the limits
    unlimited: bool,
    // values a reader sees instead of the stored ones while the world is read-only:
    // (entity, component, value or None for no value). the host sets them around a
    // read-only run, for example one screen of several drawn from one world; never
    // journaled, the world does not change
    pub overlay: Vec<(String, String, Option<String>)>,
    // pairs the host hands to the code (ecs.seen), for example what a person sees now.
    // not part of the world, not journaled
    pub seen: Vec<(String, String)>,
}

// the types every world starts with. only the developer may change them;
// the content pack widens the rights of parent and is_a for the roles that build
const GENESIS_TYPES: &[(&str, &str, &str, bool)] = &[
    ("kind", "name", "the type of a component: text, code, int, bool, ref, name or names", true),
    ("desc", "text", "what an entity is, in one sentence", false),
    ("write", "names", "the roles that may write a component", true),
    ("local", "bool", "the values of a component are not taken over by instances", true),
    ("parent", "ref", "the entity this one belongs to", true),
    ("is_a", "ref", "the prefab: values not set here are taken from it", true),
    ("ord", "int", "the place among siblings", false),
];

impl World {
    // a world with nothing in it, not even the genesis types
    fn empty() -> World {
        World {
            comps: BTreeMap::new(),
            counts: BTreeMap::new(),
            refs: BTreeMap::new(),
            values: 0,
            journal: Vec::new(),
            undo: Vec::new(),
            current_role: String::from("-"),
            current_op: String::from("-"),
            read_only: false,
            hide_private: false,
            unlimited: false,
            overlay: Vec::new(),
            seen: Vec::new(),
        }
    }

    pub fn new() -> World {
        let mut w = World::empty();
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

    // the world a saved journal describes. every entry must follow the one before
    // (seq, prev, hash); the values were checked when they were written, so they are
    // put back as they are. a journal edited by hand breaks the chain and is refused
    pub fn replay(entries: Vec<Entry>) -> Result<World, String> {
        let mut w = World::empty();
        for e in entries {
            let seq = w.journal.len() as u64 + 1;
            let prev = w.head();
            if e.seq != seq || e.prev != prev || e.hash != entry_hash(&prev, seq, &e.role, &e.op, &e.entity, &e.comp, &e.value) {
                return Err(format!("journal broken at entry {}", seq));
            }
            if e.comp != "-" {
                if !valid_id(&e.entity) || !valid_id(&e.comp) {
                    return Err(format!("journal entry {} names no entity", seq));
                }
                let v = if e.value.is_empty() { None } else { Some(e.value.as_str()) };
                w.store(&e.entity, &e.comp, v);
            }
            w.journal.push(e);
            w.undo.push(None);
        }
        Ok(w)
    }

    // a world without the limits of size, for measurements only (radost's bench.rs)
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
            None => GENESIS.to_string(),
        }
    }

    fn push(&mut self, entity: &str, comp: &str, value: &str, undo: Option<Undo>) {
        let seq = self.journal.len() as u64 + 1;
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
        self.undo.push(undo);
    }

    // a value from a crystal: checked when it was first written, put back as it is, as the
    // role and operation set now (crystal.rs). None removes it
    pub fn crystal_write(&mut self, e: &str, c: &str, v: Option<&str>) {
        let before = self.store(e, c, v);
        self.push(e, c, v.unwrap_or(""), Some(Undo { entity: e.to_string(), comp: c.to_string(), before }));
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
        let mut out = BTreeMap::new();
        for (c, map) in &self.comps {
            for (e, v) in map {
                out.insert((e.clone(), c.clone()), v.clone());
            }
        }
        out
    }

    // an entry that changes nothing: a refusal or a note
    pub fn note(&mut self, text: &str) {
        self.push("-", "-", text, None);
    }

    // writes without any check: genesis and undo only
    fn raw_set(&mut self, e: &str, c: &str, v: &str) {
        let before = self.store(e, c, Some(v));
        self.push(e, c, v, Some(Undo { entity: e.to_string(), comp: c.to_string(), before }));
    }

    // puts a value (or removes it with None) and keeps the counts and the
    // reference index; returns the old value. every change of the world passes here
    fn store(&mut self, e: &str, c: &str, v: Option<&str>) -> Option<String> {
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
                *self.counts.entry(e.to_string()).or_insert(0) += 1;
                self.values += 1;
            }
            (Some(_), None) => {
                let n = self.counts.get_mut(e).map(|n| {
                    *n -= 1;
                    *n
                });
                if n == Some(0) {
                    self.counts.remove(e);
                }
                self.values -= 1;
            }
            _ => {}
        }
        old
    }

    // ---------- reading ----------

    pub fn exists(&self, e: &str) -> bool {
        self.counts.contains_key(e)
    }

    pub fn entities(&self) -> impl Iterator<Item = &String> {
        self.counts.keys()
    }

    // the blocks of the screens that have no name (ru, their own or of their prefab): every
    // block is named, so that the person and an AI can say which one is meant
    pub fn own(&self, e: &str, c: &str) -> Option<&str> {
        if let Some(v) = self.viewed(e, c) {
            return v;
        }
        self.comps.get(c).and_then(|m| m.get(e)).map(|s| s.as_str())
    }

    // the value an overlay gives while the world is read-only: Some(value or none)
    fn viewed(&self, e: &str, c: &str) -> Option<Option<&str>> {
        if !self.read_only {
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
    // and the entity it comes from
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

    // the components an entity has itself
    pub fn own_comps(&self, e: &str) -> Vec<String> {
        self.comps.iter().filter(|(_, m)| m.contains_key(e)).map(|(c, _)| c.clone()).collect()
    }

    // own components and those taken over from the prefab chain: (comp, value, from)
    pub fn all_comps(&self, e: &str) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for c in self.comps.keys() {
            if let Some((v, from)) = self.get(e, c) {
                out.push((c.clone(), v.to_string(), from.to_string()));
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
        if let Some(x) = v {
            if self.kind_of(c) == Some(Kind::Ref) {
                return self.pointing(x, c);
            }
        }
        match self.comps.get(c) {
            None => Vec::new(),
            Some(m) => m.iter().filter(|(_, val)| v.map_or(true, |x| x == val.as_str())).map(|(e, _)| e.clone()).collect(),
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
        role == CORE_ROLE || self.own(c, "write").map_or(false, |w| w.split(' ').any(|r| r == role))
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
                if !valid_id(raw) {
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
        if !valid_id(e) {
            return Err(String::from("bad_entity"));
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

    // sets one component of one entity; creates the entity if it had no component yet
    pub fn set(&mut self, e: &str, c: &str, raw: &str) -> Result<(), String> {
        let kind = self.check_write(e, c)?;
        let v = self.canonical(e, c, kind, raw)?;
        if c == "kind" {
            let new = Kind::parse(&v).ok_or_else(|| String::from("bad_kind"))?;
            if self.comps.contains_key(e) && self.kind_of(e) != Some(new) {
                return Err(format!("type_in_use: {}", e));
            }
        }
        if self.own(e, c) == Some(v.as_str()) {
            return Ok(());
        }
        if !self.exists(e) && self.counts.len() >= ENTITIES_MAX && !self.unlimited {
            return Err(String::from("too_many_entities"));
        }
        if self.own(e, c).is_none() && self.values >= VALUES_MAX && !self.unlimited {
            return Err(String::from("too_many_values"));
        }
        let before = self.store(e, c, Some(&v));
        self.push(e, c, &v, Some(Undo { entity: e.to_string(), comp: c.to_string(), before }));
        Ok(())
    }

    // names that point at e: refs to it, or e used as a component type
    fn uses_of(&self, e: &str) -> Option<String> {
        if self.comps.contains_key(e) {
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
        if self.own(e, c).is_none() {
            return Ok(());
        }
        if c == "kind" && self.comps.contains_key(e) {
            return Err(format!("type_in_use: {}", e));
        }
        // the last component: the entity disappears, nothing may point at it
        if self.counts.get(e) == Some(&1) {
            if let Some(why) = self.uses_of(e) {
                return Err(why);
            }
        }
        let before = self.store(e, c, None);
        self.push(e, c, "", Some(Undo { entity: e.to_string(), comp: c.to_string(), before }));
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

    // undoes every entry after len and writes one entry with the reason
    pub fn rollback(&mut self, len: usize, reason: &str) {
        while self.journal.len() > len {
            self.journal.pop();
            if let Some(Some(u)) = self.undo.pop() {
                self.store(&u.entity, &u.comp, u.before.as_deref());
            }
        }
        self.note(&format!("rollback: {}", reason));
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
        w.set("a", "minutes", "2").unwrap();
        w.set("b", "minutes", "3").unwrap();
        w.del("a", "minutes").unwrap();
        w.rollback(len, "test");
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
        let len = w.journal.len();
        w.set("d", "parent", "b").unwrap();
        w.del("c", "is_a").unwrap();
        w.set("c", "parent", "b").unwrap();
        assert_eq!(w.children("b"), vec!["c", "d"]);
        assert_eq!(index_refs(&w), scan_refs(&w));
        w.rollback(len, "test");
        assert_eq!(index_refs(&w), scan_refs(&w));
        assert_eq!(w.children("a"), vec!["b", "c"]);
        assert!(w.children("b").is_empty());
        let r = World::replay(w.journal.clone()).unwrap();
        assert_eq!(index_refs(&r), scan_refs(&r));
        assert_eq!(index_refs(&r), index_refs(&w));
        w.kill("c").unwrap();
        w.kill("b").unwrap();
        assert_eq!(index_refs(&w), scan_refs(&w));
        assert!(w.refs.get("a").is_none());
    }
}
