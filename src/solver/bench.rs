// a benchmark, not a test of right and wrong: how long one tick of the solver takes on a scene
// of 1 000, 10 000 and 100 000 boards (links), driven from Lua like a game does it.
//   cargo test --release solver::bench -- --ignored --nocapture
// the scene is a lattice of joints (columns x 10 rows): a horizontal, a vertical and a diagonal
// beam to every joint (about 3 boards per joint), the first column fixed, the bottom row also
// road edges (layer 4) with a few wheels over it, one ground segment. a tick is 16 substeps of
// integrate_verlet, damp_angular, solve_distance, project_edges, project_segments, as the example
// game does it
use super::attach;
use crate::ecs::{Spec, World};
use crate::host::Host;
use std::time::Instant;

const COLS: [&str; 26] = [
    "x", "y", "old_x", "old_y", "inv_mass", "radius", "layer", "mask", "contact", "contact_nx", "contact_ny", "node_a", "node_b",
    "rest_len", "compliance", "mode", "lambda", "force_max", "x1", "y1", "x2", "y2", "friction", "damping", "group", "strength",
];

fn host() -> Host {
    let h = Host::with_spec(Spec::game()).unwrap();
    {
        let mut w: std::cell::RefMut<World> = h.world.borrow_mut();
        for c in COLS {
            w.set(c, "kind", "num").unwrap();
        }
    }
    attach(&h).unwrap();
    h
}

// the scene is made in pieces (a Lua call has a time limit): the joints of some columns, then the
// boards of some columns. joints are found again by name
const ROWS: usize = 10;
const CHUNK: usize = 300;

fn cols_for(boards: usize) -> usize {
    (boards / (3 * ROWS)).max(2)
}

fn joints(c0: usize, c1: usize) -> String {
    format!(
        r#"
local S = 10
for c = {c0}, {c1} - 1 do
  for r = 0, {rows} - 1 do
    ecs.spawn('j' .. c .. '_' .. r, {{x = c * S, y = 300 + r * S, old_x = c * S, old_y = 300 + r * S,
      inv_mass = c == 0 and 0 or 0.05, radius = 3, layer = 8, mask = 1, contact = 0, contact_nx = 0, contact_ny = 0,
      damping = 0.996}})
  end
end
return 'ok'
"#,
        c0 = c0,
        c1 = c1,
        rows = ROWS
    )
}

fn boards_of(c0: usize, c1: usize, cols: usize) -> String {
    format!(
        r#"
local S = 10
local cols, rows = {cols}, {rows}
local n = 0
local function link(a, b, rest, layer)
  local t = {{node_a = ecs.id(a), node_b = ecs.id(b), rest_len = rest, compliance = 0, mode = 0, lambda = 0, force_max = 0, strength = 1e9}}
  if layer then t.layer = layer t.friction = 0.8 end
  ecs.spawn(t)
  n = n + 1
end
local d = math.sqrt(2) * S
for c = {c0}, {c1} - 1 do
  for r = 0, rows - 1 do
    local a = 'j' .. c .. '_' .. r
    if c + 1 < cols then link(a, 'j' .. (c + 1) .. '_' .. r, S, r == rows - 1 and 4 or nil) end
    if r + 1 < rows then link(a, 'j' .. c .. '_' .. (r + 1), S) end
    if c + 1 < cols and r + 1 < rows then link(a, 'j' .. (c + 1) .. '_' .. (r + 1), d) end
  end
end
return n
"#,
        c0 = c0,
        c1 = c1,
        cols = cols,
        rows = ROWS
    )
}

// returns the number of boards
fn build(h: &Host, boards: usize) -> usize {
    let cols = cols_for(boards);
    h.probe("ecs.spawn('ground', {x1 = -100000, y1 = 2000, x2 = 100000, y2 = 2000, friction = 0.5, layer = 1}) return 'ok'").unwrap();
    let mut c = 0;
    while c < cols {
        let e = (c + CHUNK).min(cols);
        h.probe(&joints(c, e)).unwrap();
        c = e;
    }
    let (mut c, mut n) = (0, 0usize);
    while c < cols {
        let e = (c + CHUNK).min(cols);
        n += h.probe(&boards_of(c, e, cols)).unwrap().parse::<usize>().unwrap();
        c = e;
    }
    // a few wheels over the road
    for k in 1..=5 {
        let src = format!(
            "local x = {} * {} * 10 / 6 local y = 300 + {} * 10 + 5 ecs.spawn({{x = x, y = y, old_x = x, old_y = y, inv_mass = 0.02, radius = 12, layer = 16, mask = 23, contact = 0, contact_nx = 0, contact_ny = 0, damping = 0.96}}) return 'ok'",
            k, cols, ROWS
        );
        h.probe(&src).unwrap();
    }
    n
}

// the call is made `times` times, one probe each (a probe has a time limit); the time of the
// probe itself (a few microseconds) is in the numbers
fn time(h: &Host, call: &str, times: usize) -> f64 {
    let src = format!("{} return 'ok'", call);
    let t = Instant::now();
    for _ in 0..times {
        assert_eq!(h.probe(&src).unwrap(), "ok");
    }
    t.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore]
fn one_tick_on_1_000_10_000_and_100_000_boards() {
    for (boards, ticks) in [(1_000usize, 20usize), (10_000, 6), (100_000, 2)] {
        let h = host();
        let t = Instant::now();
        let n = build(&h, boards);
        let what = format!("{}x{} joints, {} boards", cols_for(boards), ROWS, n);
        let build = t.elapsed().as_secs_f64() * 1000.0;
        // warm up: the first calls size the arrays
        time(&h, "ecs.integrate_verlet(1/960, 0, 980, 0.999) ecs.solve_distance(1/960, 1)", 2);
        let sub = ticks * 16;
        let parts = [
            ("integrate_verlet", "ecs.integrate_verlet(1/960, 0, 980, 0.999)"),
            ("damp_angular", "ecs.damp_angular(0.00005, 0.0005)"),
            ("solve_distance", "ecs.solve_distance(1/960, 1)"),
            ("project_edges", "ecs.project_edges(1)"),
            ("project_segments", "ecs.project_segments(1)"),
        ];
        println!("--- {} ({}), scene built in {:.0} ms", boards, what, build);
        let mut total = 0.0;
        for (name, call) in parts {
            let ms = time(&h, call, sub) / ticks as f64;
            total += ms;
            println!("  {:<18} {:>9.2} ms per tick", name, ms);
        }
        println!("  {:<18} {:>9.2} ms per tick (16 substeps), {:.1} fps if only this", "all five", total, 1000.0 / total);
    }
}
