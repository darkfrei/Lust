// how long one tick takes, and how long the worst ticks take: the average says little about
// a frame that must never be late. prints mean, p99 (99 ticks of 100 are faster) and max
//   cargo run --release --example frames
use lust::ecs::{Spec, World};
use lust::host::Host;
use lust::link::{commit_to, Seed};
use std::time::Instant;

fn stats(name: &str, times: &mut [f64]) {
    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = times.len().max(1);
    let mean = times.iter().sum::<f64>() / n as f64;
    let p99 = times[(n * 99 / 100).min(n - 1)];
    let max = times[n - 1];
    println!("{:<60} mean {:>8.3} ms   p99 {:>8.3} ms   max {:>8.3} ms", name, mean, p99, max);
}

// a ledger with one operation op.noop of the role clock
fn ledger(src: &str) -> Host {
    let host = Host::new().unwrap();
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in [
            ("roles", "kind", "names"), ("roles", "write", "dev"),
            ("params", "kind", "text"), ("params", "write", "dev"),
            ("src", "kind", "code"), ("src", "write", "dev"),
            ("role.clock", "desc", "moves time on"),
            ("op.noop", "roles", "clock"),
        ] {
            w.set(e, c, v).unwrap();
        }
        w.set("op.noop", "src", src).unwrap();
    }
    host.budget.set_run_ms(5000);
    host
}

fn ledger_empty_runs() {
    for len in [0usize, 20000] {
        // real statements, not a comment: a comment costs the compiler almost nothing
        let mut body = String::new();
        while body.len() < len {
            let i = body.len();
            body.push_str(&format!("    if p.k == {} then return {} * 2 + 1 end\n", i, i));
        }
        let host = ledger(&format!("return function(p)\n{}end", body));
        let mut times = Vec::new();
        for _ in 0..2000 {
            let t = Instant::now();
            host.run("clock", "noop", &[]).unwrap();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        stats(&format!("ledger: one empty operation, code of {} bytes", len), &mut times);
    }
}

// 10 000 entities moved in one operation of a ledger: text values (int, in the journal) or
// num values (columns: not in the journal, but rights and undo hold)
fn ledger_moving(kind: &str, ticks: usize) {
    let host = ledger("return function() for _, b in ipairs(ecs.with('vx')) do ecs.set(b, 'x', ecs.get(b, 'x') + ecs.get(b, 'vx')) end end");
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in [("x", "kind", kind), ("x", "write", "dev clock"), ("vx", "kind", kind), ("vx", "write", "dev")] {
            w.set(e, c, v).unwrap();
        }
        for i in 0..10_000 {
            w.set(&format!("b{}", i), "x", "0").unwrap();
            w.set(&format!("b{}", i), "vx", &i.to_string()).unwrap();
        }
    }
    let mut times = Vec::new();
    for _ in 0..ticks {
        let t = Instant::now();
        host.run("clock", "noop", &[]).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    stats(&format!("ledger: 10000 entities, ecs.get/ecs.set by name, kind {}", kind), &mut times);
}

fn commit() {
    let host = ledger("return function(p) ecs.set('red', 'units', p.red) ecs.set('blue', 'units', p.blue) end");
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in [("units", "kind", "int"), ("units", "write", "dev clock"), ("op.noop", "params", "red:int blue:int"), ("red", "units", "100"), ("blue", "units", "100")] {
            w.set(e, c, v).unwrap();
        }
    }
    let mut times = Vec::new();
    for i in 0..1000 {
        let mut seed = Seed::new();
        seed.read(&host.world.borrow(), "red", "units");
        let r = vec![(String::from("blue"), (i % 50).to_string()), (String::from("red"), (i % 70).to_string())];
        let t = Instant::now();
        commit_to(&host, "battle", "clock", "noop", &seed, &r).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    stats("ledger: one result of another world committed", &mut times);
}

// a game world with n nameless entities (x, vx) and one tick
fn game(n: usize, spec: Spec, tick: &str) -> Host {
    let host = Host::with_spec(spec).unwrap();
    {
        let mut w = host.world.borrow_mut();
        for (e, c, v) in [("src", "kind", "code"), ("tick", "kind", "bool"), ("x", "kind", "num"), ("vx", "kind", "num"), ("sys.main", "tick", "true")] {
            w.set(e, c, v).unwrap();
        }
        let src = format!(
            "ids, xs, vxs, ids2 = {{}}, {{}}, {{}}, {{}}
            X, VX = ecs.col('x'), ecs.col('vx')
            for i = 1, {} do ids[i] = ecs.spawn({{[X] = 0, [VX] = i}}) end
            local get, set = ecs.get, ecs.set
            return function(dt)
            {}
            end",
            n, tick
        );
        w.set("sys.main", "src", &src).unwrap();
    }
    host.add_system("move", Box::new(|w: &mut World, dt| {
        if let (Some(x), Some(vx)) = (w.col("x"), w.col("vx")) {
            let _ = w.add_cols(x, vx, dt);
        }
    }))
    .unwrap();
    host.budget.set_run_ms(5000);
    // the first tick loads the code and spawns the entities: not measured
    host.tick(0.0).unwrap();
    host
}

fn run_game(name: &str, n: usize, ticks: usize, spec: Spec, tick: &str) {
    let host = game(n, spec, tick);
    let mut times = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        let t = Instant::now();
        host.tick(0.001).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    stats(&format!("game: {} entities, {}", n, name), &mut times);
}

const PER_ENTITY: &str = "for i = 1, #ids do local e = ids[i] set(e, X, get(e, X) + get(e, VX) * dt) end";
const BY_NAME: &str = "for i = 1, #ids do local e = ids[i] set(e, 'x', get(e, 'x') + get(e, 'vx') * dt) end";
const READ_WRITE: &str = "local n = ecs.read(X, ids2, xs) ecs.read(VX, ids2, vxs) for i = 1, n do xs[i] = xs[i] + vxs[i] * dt end ecs.write(X, ids2, xs, n)";
const QUERY: &str = "local n, qi, qx, qv = ecs.query(X, VX) for i = 1, n do qx[i] = qx[i] + qv[i] * dt end ecs.write(X, qi, qx, n)";
// no call into rust at all: what lua itself costs
const PLAIN: &str = "if not px then px, pvx = {}, {} for i = 1, #ids do px[i] = 0 pvx[i] = i end end for i = 1, #px do px[i] = px[i] + pvx[i] * dt end";
const COLUMN: &str = "ecs.add(X, VX, dt)";
const SYSTEM: &str = "ecs.run('move', dt)";
// a tick that makes garbage: a small table for every entity
const GARBAGE: &str = "local n = ecs.read(X, ids2, xs) local sum = 0 for i = 1, n do local p = {x = xs[i], y = i} sum = sum + p.x end";

fn lifecycle() {
    let mut times = Vec::new();
    for _ in 0..200 {
        let t = Instant::now();
        let host = game(1000, Spec::game(), SYSTEM);
        host.tick(0.001).unwrap();
        drop(host);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    stats("game: make a world, 1000 entities, two ticks, drop it", &mut times);
}

fn main() {
    let undo = Spec { undo: true, ..Spec::game() };
    println!("-- a ledger (journal, roles, undo)");
    ledger_empty_runs();
    ledger_moving("int", 20);
    ledger_moving("num", 20);
    commit();
    println!("-- a game world (no journal, no roles, no undo): few entities, many ticks");
    run_game("one by one (get/set, column numbers)", 100, 20_000, Spec::game(), PER_ENTITY);
    run_game("rust system", 100, 20_000, Spec::game(), SYSTEM);
    println!("-- a game world: many entities, 1200 ticks (10 s at 120 per second)");
    run_game("one by one (get/set, column names)", 10_000, 1200, Spec::game(), BY_NAME);
    run_game("one by one (get/set, column numbers)", 10_000, 1200, Spec::game(), PER_ENTITY);
    run_game("one by one, with undo", 10_000, 1200, undo.clone(), PER_ENTITY);
    run_game("read/write into kept tables, lua loop", 10_000, 1200, Spec::game(), READ_WRITE);
    run_game("query/write, new tables, lua loop", 10_000, 1200, Spec::game(), QUERY);
    run_game("plain lua tables, no world calls", 10_000, 1200, Spec::game(), PLAIN);
    run_game("ecs.add over columns", 10_000, 1200, Spec::game(), COLUMN);
    run_game("ecs.add over columns, with undo", 10_000, 1200, undo, COLUMN);
    run_game("rust system", 10_000, 1200, Spec::game(), SYSTEM);
    run_game("garbage: a small table per entity", 10_000, 1200, Spec::game(), GARBAGE);
    println!("-- a world made and dropped on the fly");
    lifecycle();
}
