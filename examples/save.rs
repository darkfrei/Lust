// saving and loading a game that computes: 40 bodies pull each other every tick (every
// body against every other), so each tick changes 80 values. the journal alone would be
// a save too, but it grows with every tick for ever, and loading would replay all of it.
// so the game keeps a snapshot (the state) and a journal volume (the ticks since it):
//   every tick         the new entries go to the volume (Store::append)
//   every 25 ticks     a snapshot (World::snapshot, store::save_snapshot); the volume
//                      that ends there is moved aside (store::archive) and a new one begins
//   loading            the snapshot and only the entries of the volume after it
//                      (store::load_snapshot, store::load, World::from_snapshot)
// the hash of the last entry is kept in a file of its own: it shows a save cut or rewritten
// outside the game (the chain alone has no secret).
//   cargo run --release --example save          a new game, 60 ticks, saved
//   cargo run --release --example save          loaded, 60 more ticks
//   cargo run --release --example save -- new   everything moved aside, a new game
//   cargo run --release --example save -- 180   180 ticks instead of 60 (with or without new)
use lust::ecs::World;
use lust::host::Host;
use lust::store::{self, Store};
use std::path::{Path, PathBuf};
use std::time::Instant;

const BODIES: usize = 40;
const SNAPSHOT_EVERY: u64 = 25;

fn new_game() -> Result<Host, String> {
    let host = Host::new().map_err(|e| e.to_string())?;
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in [
            ("roles", "kind", "names"), ("roles", "write", "dev"),
            ("params", "kind", "text"), ("params", "write", "dev"),
            ("src", "kind", "code"), ("src", "write", "dev"),
            ("system", "kind", "bool"), ("system", "write", "dev"),
            ("tick", "kind", "int"), ("tick", "write", "dev clock"),
            ("x", "kind", "int"), ("x", "write", "dev system"),
            ("y", "kind", "int"), ("y", "write", "dev system"),
            ("vx", "kind", "int"), ("vx", "write", "dev system"),
            ("vy", "kind", "int"), ("vy", "write", "dev system"),
            ("role.clock", "desc", "moves time on"),
            ("op.step", "roles", "clock"),
            ("op.step", "src", "return function() ecs.set('sky', 'tick', (ecs.get('sky', 'tick') or 0) + 1) end"),
            // the computation: every body against every other, in whole numbers
            ("sys.gravity", "system", "true"),
            ("sys.gravity", "src", "return function()
                local list = ecs.with('vx')
                local pos = {}
                for i, b in ipairs(list) do pos[i] = {ecs.get(b, 'x'), ecs.get(b, 'y')} end
                for i, b in ipairs(list) do
                    local ax, ay = 0, 0
                    for j = 1, #list do
                        if j ~= i then
                            local dx, dy = pos[j][1] - pos[i][1], pos[j][2] - pos[i][2]
                            local d2 = dx * dx + dy * dy + 250000
                            ax = ax + dx * 4000000 // d2 // 1000
                            ay = ay + dy * 4000000 // d2 // 1000
                        end
                    end
                    local vx, vy = ecs.get(b, 'vx') + ax, ecs.get(b, 'vy') + ay
                    ecs.set(b, 'vx', vx)
                    ecs.set(b, 'vy', vy)
                    ecs.set(b, 'x', pos[i][1] + vx // 10)
                    ecs.set(b, 'y', pos[i][2] + vy // 10)
                end
            end"),
        ] {
            w.set(e, c, v)?;
        }
        for i in 0..BODIES {
            let id = format!("body.b{}", i);
            let a = i as f64 * 2.399963;
            let r = 2000.0 + 150.0 * i as f64;
            w.set(&id, "x", &((a.cos() * r) as i64).to_string())?;
            w.set(&id, "y", &((a.sin() * r) as i64).to_string())?;
            w.set(&id, "vx", &((-a.sin() * 40.0) as i64).to_string())?;
            w.set(&id, "vy", &((a.cos() * 40.0) as i64).to_string())?;
        }
    }
    Ok(host)
}

struct Files {
    snap: PathBuf,
    volume: PathBuf,
    head: PathBuf,
}

// the game from its save: the snapshot (if there is one), the volume after it, the kept hash
fn load(f: &Files) -> Result<Option<Host>, String> {
    let snap = store::load_snapshot(&f.snap)?;
    let after = store::load(&f.volume)?.map(|(e, _)| e).unwrap_or_default();
    if snap.is_none() && after.is_empty() {
        return Ok(None);
    }
    let n = after.len();
    let world = match &snap {
        Some(s) => World::from_snapshot(s, after)?,
        None => World::replay(after)?,
    };
    let kept = std::fs::read_to_string(&f.head).unwrap_or_default();
    if kept.trim() != world.head() {
        return Err(String::from("the save does not end where it was saved: changed outside the game"));
    }
    match &snap {
        Some(s) => println!("loaded: a snapshot at entry {} ({} values) and {} entries after it", s.seq, s.values.len(), n),
        None => println!("loaded: {} entries, no snapshot yet", n),
    }
    Host::with_world(world).map(Some).map_err(|e| e.to_string())
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn main() -> Result<(), String> {
    let dir = std::env::temp_dir();
    let f = Files { snap: dir.join("lust-sky.snap.json"), volume: dir.join("lust-sky.jsonl"), head: dir.join("lust-sky.head") };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ticks = args.iter().find_map(|a| a.parse::<u64>().ok()).unwrap_or(60);
    if args.iter().any(|a| a == "new") {
        for p in [&f.volume, &f.snap] {
            if let Some(old) = store::archive(p)? {
                println!("moved aside: {}", old.display());
            }
        }
    }
    let t = Instant::now();
    let (host, on_disk) = match load(&f)? {
        // the volume on disk holds what the world in memory holds since its snapshot
        Some(h) => {
            let n = h.world.borrow().journal.len();
            (h, n)
        }
        None => {
            println!("a new game: {} bodies", BODIES);
            (new_game()?, 0)
        }
    };
    println!("ready in {:?}", t.elapsed());

    let mut st = Store::new(f.volume.clone(), on_disk);
    st.append(&host.world.borrow().journal)?;
    for _ in 0..ticks {
        host.run("clock", "step", &[])?;
        st.append(&host.world.borrow().journal)?;
        let (tick, head) = {
            let w = host.world.borrow();
            (w.value("sky", "tick").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0), w.head())
        };
        if tick % SNAPSHOT_EVERY == 0 {
            // the state now; the volume that ends here goes aside, a new one begins after it
            store::save_snapshot(&f.snap, &host.world.borrow().snapshot())?;
            store::archive(&f.volume)?;
            st = Store::new(f.volume.clone(), host.world.borrow().journal.len());
        }
        std::fs::write(&f.head, head).map_err(|e| e.to_string())?;
    }
    let w = host.world.borrow();
    println!(
        "tick {}: body b0 at {}, {} | {} entries in all; on disk: snapshot {} KB, volume {} KB",
        w.value("sky", "tick").unwrap_or("0"),
        w.value("body.b0", "x").unwrap_or("-"),
        w.value("body.b0", "y").unwrap_or("-"),
        w.seq(),
        size(&f.snap) / 1024,
        size(&f.volume) / 1024
    );
    Ok(())
}
