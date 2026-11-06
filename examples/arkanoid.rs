// an arcade game in one world of the game kind (Spec::game): no journal, no roles, no undo,
// a lua that remembers between ticks, positions as num components. no window: the frames
// are simulated and the state is printed, so it runs anywhere.
//   - the bricks are made without names (ecs.spawn{...}); the ball and the paddle have names
//   - the frame time jumps between 1/144 and 1/30 s, and dt follows it (Clock::variable)
//   - the paddle is moved by an operation, as input would be (anyone may run it: no roles)
//   - after 15 seconds the game is saved as a snapshot, the world is dropped, loaded from
//     the file into a new world, and the game goes on; the top of the tick code runs again
//     there and finds its column numbers again
//   cargo run --release --example arkanoid
use lust::clock::Clock;
use lust::ecs::{Spec, World};
use lust::host::Host;
use lust::store;
use std::time::Instant;

// the types, the code of a tick, the start and the input
fn setup(host: &Host) {
    let mut w = host.world.borrow_mut();
    for (e, c, v) in [
        ("src", "kind", "code"),
        ("params", "kind", "text"),
        ("tick", "kind", "bool"),
        ("x", "kind", "num"),
        ("y", "kind", "num"),
        ("vx", "kind", "num"),
        ("vy", "kind", "num"),
        ("w", "kind", "num"),
        ("h", "kind", "num"),
        ("hp", "kind", "num"),
        ("score", "kind", "num"),
        ("lives", "kind", "num"),
        ("sys.physics", "tick", "true"),
        ("op.move", "params", "x:int"),
        ("op.move", "src", "return function(p) ecs.set('paddle', 'x', math.max(20, math.min(300, p.x))) end"),
        ("op.start", "src", START),
    ] {
        w.set(e, c, v).unwrap();
    }
    w.set("sys.physics", "src", PHYSICS).unwrap();
}

const START: &str = "
return function(p)
    for row = 1, 5 do
        for col = 1, 10 do
            local hp = 1
            if row <= 2 then hp = 2 end
            ecs.spawn({x = col * 30 - 5, y = 20 + row * 12, w = 28, h = 10, hp = hp})
        end
    end
    ecs.spawn('paddle', {x = 160, y = 220, w = 40})
    ecs.spawn('ball', {x = 160, y = 150, vx = 70, vy = -110})
    ecs.spawn('game', {score = 0, lives = 3})
end
";

// the top runs once in this lua (the load of the code): the column numbers, the size of the
// field. after loading a saved game it runs again in the new world
const PHYSICS: &str = "
X, Y, VX, VY, W, H, HP = ecs.col('x'), ecs.col('y'), ecs.col('vx'), ecs.col('vy'), ecs.col('w'), ecs.col('h'), ecs.col('hp')
WIDTH, HEIGHT = 320, 240

local function walls(ball)
    local x, y = ecs.get(ball, X), ecs.get(ball, Y)
    if x < 0 then
        ecs.set(ball, X, -x)
        ecs.set(ball, VX, math.abs(ecs.get(ball, VX)))
    end
    if x > WIDTH then
        ecs.set(ball, X, 2 * WIDTH - x)
        ecs.set(ball, VX, -math.abs(ecs.get(ball, VX)))
    end
    if y < 0 then
        ecs.set(ball, Y, -y)
        ecs.set(ball, VY, math.abs(ecs.get(ball, VY)))
    end
end

local function paddle(ball)
    local bx, by = ecs.get(ball, X), ecs.get(ball, Y)
    local px, py, pw = ecs.get('paddle', X), ecs.get('paddle', Y), ecs.get('paddle', W)
    if ecs.get(ball, VY) > 0 and by >= py and by <= py + 8 and math.abs(bx - px) <= pw / 2 then
        ecs.set(ball, VY, -math.abs(ecs.get(ball, VY)))
        ecs.set(ball, VX, ecs.get(ball, VX) + (bx - px) * 2)
    end
end

-- the first brick the ball is inside: it bounces, the brick loses a point of hp
local function bricks(ball)
    local bx, by = ecs.get(ball, X), ecs.get(ball, Y)
    local n, ids, hps, xs, ys, ws, hs = ecs.query(HP, X, Y, W, H)
    for i = 1, n do
        if math.abs(bx - xs[i]) <= ws[i] / 2 and math.abs(by - ys[i]) <= hs[i] / 2 then
            ecs.set(ball, VY, -ecs.get(ball, VY))
            if hps[i] <= 1 then ecs.kill(ids[i]) else ecs.set(ids[i], HP, hps[i] - 1) end
            ecs.set('game', 'score', ecs.get('game', 'score') + 10)
            return
        end
    end
end

local function lost(ball)
    if ecs.get(ball, Y) > HEIGHT then
        ecs.set(ball, X, WIDTH / 2)
        ecs.set(ball, Y, HEIGHT / 2)
        ecs.set(ball, VY, -math.abs(ecs.get(ball, VY)))
        ecs.set('game', 'lives', ecs.get('game', 'lives') - 1)
    end
end

return function(dt)
    -- everything that has a speed moves: here only the ball, but it is one call for many
    ecs.add(X, VX, dt)
    ecs.add(Y, VY, dt)
    ecs.clamp(VX, -250, 250)
    walls('ball')
    paddle('ball')
    bricks('ball')
    lost('ball')
end
";

// the frame times of a real machine jump; this makes them jump the same way every run
struct Jitter(u64);

impl Jitter {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let r = (self.0 >> 33) as f64 / (1u64 << 31) as f64;
        1.0 / 144.0 + r * (1.0 / 30.0 - 1.0 / 144.0)
    }
}

fn state(host: &Host) -> String {
    let w = host.world.borrow();
    let num = |e: &str, c: &str| w.num(e, c).unwrap_or(0.0);
    format!(
        "score {:>4}  lives {}  bricks {:>2}  ball at {:>6.1}, {:>6.1}",
        num("game", "score"),
        num("game", "lives"),
        w.col("hp").map_or(0, |c| w.column(c).len()),
        num("ball", "x"),
        num("ball", "y")
    )
}

// plays for `seconds` of game time; the player moves the paddle towards the ball, but not
// faster than 220 points a second, so it sometimes misses
fn play(host: &Host, clock: &mut Clock, jitter: &mut Jitter, seconds: f64, times: &mut Vec<f64>) -> f64 {
    let mut played = 0.0;
    while played < seconds {
        let frame = jitter.next();
        let t = Instant::now();
        let (n, dt) = clock.advance(frame);
        for _ in 0..n {
            host.tick(dt).unwrap();
        }
        let (ball, pad) = {
            let w = host.world.borrow();
            (w.num("ball", "x").unwrap_or(160.0), w.num("paddle", "x").unwrap_or(160.0))
        };
        let step = (ball - pad).clamp(-220.0 * frame, 220.0 * frame);
        host.run("player", "move", &[("x", &((pad + step).round() as i64).to_string())]).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        played += frame;
        let w = host.world.borrow();
        if w.num("game", "lives").unwrap_or(0.0) <= 0.0 || w.col("hp").is_some_and(|c| w.column(c).is_empty()) {
            break;
        }
    }
    played
}

fn main() {
    let save = std::env::temp_dir().join("lust-arkanoid.save.json");
    let host = Host::with_spec(Spec::game()).unwrap();
    setup(&host);
    host.run("player", "start", &[]).unwrap();
    println!("start            {}", state(&host));

    let mut clock = Clock::variable(0.05);
    let mut jitter = Jitter(7);
    let mut times = Vec::new();
    let played = play(&host, &mut clock, &mut jitter, 15.0, &mut times);
    println!("after {:>5.1} s     {}   ({} ticks, dt from {:.4} to {:.4} s)", played, state(&host), host.ticks(), 1.0 / 144.0, 1.0 / 30.0);

    // save, drop the world, load it into a new one
    let t = Instant::now();
    store::save_snapshot(&save, &host.world.borrow().snapshot()).unwrap();
    let saved = state(&host);
    drop(host);
    let snap = store::load_snapshot(&save).unwrap().unwrap();
    let host = Host::with_world(World::from_snapshot_spec(Spec::game(), &snap, Vec::new()).unwrap()).unwrap();
    println!("saved and loaded {}   ({} values, {:.2} ms, {})", state(&host), snap.values.len(), t.elapsed().as_secs_f64() * 1000.0, if state(&host) == saved { "the same" } else { "DIFFERENT" });

    let more = play(&host, &mut clock, &mut jitter, 45.0, &mut times);
    println!("after {:>5.1} s     {}", played + more, state(&host));
    let _ = std::fs::remove_file(&save);

    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = times.len().max(1);
    println!(
        "{} frames: mean {:.3} ms, p99 {:.3} ms, worst {:.3} ms; the journal has {} entries",
        times.len(),
        times.iter().sum::<f64>() / n as f64,
        times[(n * 99 / 100).min(n - 1)],
        times[n - 1],
        host.world.borrow().journal.len()
    );
}
