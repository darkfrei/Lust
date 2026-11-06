// two worlds of the same kind with different specs. the kingdom (Spec::ledger: journal,
// roles, undo) keeps how many soldiers each side has; a battle (Spec::game: none of these),
// made for one battle and dropped after it, moves and fights a thousand soldiers tick by
// tick. the kingdom gets one result, as one operation of the role "battle", and its journal
// a handful of entries instead of every tick.
//   1. a battle: the battle world is seeded from the kingdom, runs in a game loop with a
//      fixed step, commits its result, the program gives it to the kingdom and drops it
//   2. blue gets new soldiers; then the same, but reinforcements reach the kingdom while
//      the battle runs: the result is stale and refused; a new battle starts from the new
//      numbers
//   cargo run --release --example battle
use lust::clock::Clock;
use lust::ecs::{Spec, World};
use lust::host::Host;
use lust::link::{commit_to, Seed};
use std::time::Instant;

fn kingdom() -> Host {
    let host = Host::new().unwrap();
    {
        let mut w = host.world.borrow_mut();
        w.current_role = String::from("dev");
        for (e, c, v) in [
            ("roles", "kind", "names"), ("roles", "write", "dev"),
            ("params", "kind", "text"), ("params", "write", "dev"),
            ("src", "kind", "code"), ("src", "write", "dev"),
            ("units", "kind", "int"), ("units", "write", "dev battle king"),
            ("power", "kind", "int"), ("power", "write", "dev"),
            ("role.battle", "desc", "ends a battle with its result"),
            ("role.king", "desc", "sends reinforcements"),
            ("op.battle_result", "roles", "battle"),
            ("op.battle_result", "params", "red:int blue:int ticks:int"),
            ("op.battle_result", "desc", "the soldiers left after a battle"),
            ("op.battle_result", "src", "return function(p)
                if p.red < 0 or p.blue < 0 or p.red > ecs.get('red', 'units') or p.blue > ecs.get('blue', 'units') then
                    error('impossible_result', 0)
                end
                ecs.set('red', 'units', p.red)
                ecs.set('blue', 'units', p.blue)
            end"),
            ("op.reinforce", "roles", "king"),
            ("op.reinforce", "params", "side:id n:int"),
            ("op.reinforce", "src", "return function(p) ecs.set(p.side, 'units', ecs.get(p.side, 'units') + p.n) end"),
            ("red", "units", "600"), ("red", "power", "3"),
            ("blue", "units", "500"), ("blue", "power", "4"),
        ] {
            w.set(e, c, v).unwrap();
        }
    }
    host
}

// the code of the battle. RED, BLUE, RED_POWER, BLUE_POWER are put in front of it
const BATTLE: &str = "
X, VX, HP, SIDE = ecs.col('x'), ecs.col('vx'), ecs.col('hp'), ecs.col('side')
for i = 1, RED do ecs.spawn({[X] = -60 - (i % 40), [VX] = 12, [HP] = 10, [SIDE] = 1}) end
for i = 1, BLUE do ecs.spawn({[X] = 60 + (i % 40), [VX] = -12, [HP] = 10, [SIDE] = 2}) end

-- the front of a side: its soldier nearest to the enemy, and where that is
local function front(n, ids, xs, sides, side)
    local best, at = nil, nil
    for i = 1, n do
        if sides[i] == side then
            if at == nil or (side == 1 and xs[i] > at) or (side == 2 and xs[i] < at) then
                best, at = i, xs[i]
            end
        end
    end
    return best, at
end

-- how many soldiers of a side are close enough to the enemy front to strike it
local function strikers(n, xs, sides, side, enemy_at)
    local k = 0
    for i = 1, n do
        if sides[i] == side and math.abs(xs[i] - enemy_at) <= 6 then k = k + 1 end
    end
    return k
end

-- no soldier walks through the enemy front
local function hold(n, xs, sides, red_at, blue_at)
    for i = 1, n do
        if sides[i] == 1 and xs[i] > blue_at - 1 then xs[i] = blue_at - 1 end
        if sides[i] == 2 and xs[i] < red_at + 1 then xs[i] = red_at + 1 end
    end
end

local function survivors(n, sides)
    local red, blue = 0, 0
    for i = 1, n do
        if sides[i] == 1 then red = red + 1 else blue = blue + 1 end
    end
    return red, blue
end

local function hit(ids, hps, i, damage)
    local hp = hps[i] - damage
    if hp <= 0 then ecs.kill(ids[i]) else ecs.set(ids[i], HP, hp) end
end

done = false

return function(dt)
    if done then return end
    ecs.run('march', dt)
    local n, ids, xs, sides, hps = ecs.query(X, SIDE, HP)
    local r, red_at = front(n, ids, xs, sides, 1)
    local b, blue_at = front(n, ids, xs, sides, 2)
    if r == nil or b == nil then
        local red, blue = survivors(n, sides)
        ecs.commit({red = red, blue = blue, ticks = ecs.tick()})
        done = true
        return
    end
    hold(n, xs, sides, red_at, blue_at)
    ecs.write(X, ids, xs, n)
    hit(ids, hps, b, strikers(n, xs, sides, 1, blue_at) * RED_POWER * dt)
    hit(ids, hps, r, strikers(n, xs, sides, 2, red_at) * BLUE_POWER * dt)
end
";

// a new world for one battle: its types, its tick (the code above, with the numbers read
// from the kingdom in front of it) and a rust system for marching
fn make_battle(kingdom: &Host, seed: &mut Seed) -> Host {
    let head = {
        let w = kingdom.world.borrow();
        let mut num = |e: &str, c: &str| seed.read(&w, e, c).unwrap_or_default();
        let (r, b, rp, bp) = (num("red", "units"), num("blue", "units"), num("red", "power"), num("blue", "power"));
        format!("RED, BLUE, RED_POWER, BLUE_POWER = {}, {}, {}, {}\n", r, b, rp, bp)
    };
    let battle = Host::with_spec(Spec::game()).unwrap();
    {
        let mut w = battle.world.borrow_mut();
        for (e, c, v) in [("src", "kind", "code"), ("tick", "kind", "bool"), ("x", "kind", "num"), ("vx", "kind", "num"), ("hp", "kind", "num"), ("side", "kind", "num"), ("sys.battle", "tick", "true")] {
            w.set(e, c, v).unwrap();
        }
        w.set("sys.battle", "src", &format!("{}{}", head, BATTLE)).unwrap();
    }
    // marching is the same for every soldier: a rust system over two whole columns
    battle
        .add_system("march", Box::new(|w: &mut World, dt| {
            if let (Some(x), Some(vx)) = (w.col("x"), w.col("vx")) {
                let _ = w.add_cols(x, vx, dt);
            }
        }))
        .unwrap();
    battle
}

// a game loop: frames of 1/60 s, the battle in steps of 1/240 s (four per frame). the frame
// time is measured, not slept, so the example runs as fast as it can
fn fight(f: &Host) -> (Vec<(String, String)>, usize, f64, f64) {
    let mut clock = Clock::fixed(1.0 / 240.0, 8);
    let mut frames = 0usize;
    let mut worst = 0.0f64;
    let mut total = 0.0f64;
    loop {
        let t = Instant::now();
        let (n, dt) = clock.advance(1.0 / 60.0);
        for _ in 0..n {
            f.tick(dt).unwrap();
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        frames += 1;
        total += ms;
        worst = worst.max(ms);
        if let Some(r) = f.take_result() {
            return (r, frames, total / frames as f64, worst);
        }
        if frames > 60 * 600 {
            panic!("the battle does not end");
        }
    }
}

fn show(host: &Host, what: &str) {
    let w = host.world.borrow();
    println!("{:<34} red {:>4}   blue {:>4}   journal {} entries", what, w.own("red", "units").unwrap_or("-"), w.own("blue", "units").unwrap_or("-"), w.journal.len());
}

fn battle(host: &Host, reinforce: bool) {
    let mut seed = Seed::new();
    let f = make_battle(host, &mut seed);
    let (result, frames, mean, worst) = fight(&f);
    println!("  {} frames, {} ticks, {} entities left; frame mean {:.3} ms, worst {:.3} ms", frames, f.ticks(), f.world.borrow().count(), mean, worst);
    if reinforce {
        // while the battle ran, the king sent 100 soldiers to red: the battle was fought
        // with the old numbers, so its result must not be written
        host.run("king", "reinforce", &[("side", "red"), ("n", "100")]).unwrap();
        show(host, "  reinforcements during the battle");
    }
    let r = commit_to(host, "battle", "battle", "battle_result", &seed, &result);
    drop(f);
    match r {
        Ok(()) => show(host, "  result written, world dropped"),
        Err(e) => {
            println!("  result refused: {}", e);
            println!("  the world is dropped, a new battle starts from the new numbers");
            battle(host, false);
        }
    }
}

fn main() {
    let host = kingdom();
    show(&host, "before");
    println!("1. a battle");
    battle(&host, false);
    host.run("king", "reinforce", &[("side", "blue"), ("n", "450")]).unwrap();
    show(&host, "the king sends 450 to blue");
    println!("2. a battle with reinforcements arriving while it runs");
    battle(&host, true);
    let w = host.world.borrow();
    lust::journal::verify(&w.journal, &w.head()).unwrap();
    println!("the journal is a valid chain; what the battles added:");
    for e in w.journal.iter().filter(|e| e.role == "battle" || e.role == "king") {
        println!("  {:>3} {:<7} {:<15} {:<5} {}", e.seq, e.role, e.op, e.comp, e.value);
    }
}
