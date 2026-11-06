// saving and loading the progress of a game. lust saves nothing but the journal: every
// change, one JSON line each, appended after each move. loading is replaying it. the rules
// (the code of the operations) are in the journal too, so a saved game keeps the rules it
// was played with.
//   cargo run --example save          the first time: a new game, three moves, saved
//   cargo run --example save          again: the game is loaded and goes on
//   cargo run --example save -- new   the old save is moved aside, a new game begins
// next to the save, the hash of its last entry is kept in a file of its own: the chain shows
// a changed or cut entry, and the kept hash shows a save rewritten as a whole (the chain
// alone has no secret; a real game would keep it elsewhere or sign it)
use lust::ecs::World;
use lust::host::Host;
use lust::store::{self, Store};
use std::path::PathBuf;

fn new_game() -> Result<Host, String> {
    let host = Host::new().map_err(|e| e.to_string())?;
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in [
            ("roles", "kind", "names"), ("roles", "write", "dev"),
            ("params", "kind", "text"), ("params", "write", "dev"),
            ("src", "kind", "code"), ("src", "write", "dev"),
            ("gold", "kind", "int"), ("gold", "write", "dev player"),
            ("day", "kind", "int"), ("day", "write", "dev player"),
            ("role.player", "desc", "plays the game"),
            ("op.work", "roles", "player"),
            ("op.work", "src", "return function()
                ecs.set('hero', 'day', (ecs.get('hero', 'day') or 0) + 1)
                ecs.set('hero', 'gold', (ecs.get('hero', 'gold') or 0) + 10)
            end"),
        ] {
            w.set(e, c, v)?;
        }
    }
    Ok(host)
}

// the game from its save, checked: the chain of the journal and the kept hash
fn load(save: &PathBuf, head: &PathBuf) -> Result<Option<(Host, usize)>, String> {
    let (entries, warning) = match store::load(save)? {
        Some(x) => x,
        None => return Ok(None),
    };
    if let Some(w) = warning {
        println!("warning: {}", w);
    }
    let n = entries.len();
    let world = World::replay(entries)?;
    let kept = std::fs::read_to_string(head).unwrap_or_default();
    if kept.trim() != world.head() {
        return Err(format!("{} does not end where it was saved: changed outside the game", save.display()));
    }
    Ok(Some((Host::with_world(world).map_err(|e| e.to_string())?, n)))
}

fn main() -> Result<(), String> {
    let dir = std::env::temp_dir();
    let save = dir.join("lust-save.jsonl");
    let head = dir.join("lust-save.head");
    if std::env::args().nth(1).as_deref() == Some("new") {
        if let Some(old) = store::archive(&save)? {
            println!("the old game is kept as {}", old.display());
        }
    }
    let (host, saved) = match load(&save, &head)? {
        Some((h, n)) => {
            println!("loaded {} entries from {}", n, save.display());
            (h, n)
        }
        None => {
            println!("a new game");
            (new_game()?, 0)
        }
    };
    // the store knows how many entries are on disk already and appends only the new ones
    let mut st = Store::new(save.clone(), saved);
    st.append(&host.world.borrow().journal)?;
    for _ in 0..3 {
        host.run("player", "work", &[])?;
        // after every move: the new entries and the hash of the last one
        st.append(&host.world.borrow().journal)?;
        std::fs::write(&head, host.world.borrow().head()).map_err(|e| e.to_string())?;
    }
    let w = host.world.borrow();
    println!("day {}, gold {} ({} entries saved)", w.value("hero", "day").unwrap_or("0"), w.value("hero", "gold").unwrap_or("0"), w.journal.len());
    Ok(())
}
