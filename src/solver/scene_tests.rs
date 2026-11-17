// end to end: the solver driven from Lua on a world, as a program does it. the math has its
// own tests in each file; these catch what only shows when the parts work together
use super::attach;
use crate::ecs::{Spec, World};
use crate::host::Host;

const COLS: [&str; 28] = [
    "x", "y", "old_x", "old_y", "inv_mass", "radius", "layer", "mask", "contact", "contact_nx", "contact_ny", "node_a", "node_b",
    "rest_len", "compliance", "mode", "lambda", "force_max", "x1", "y1", "x2", "y2", "friction", "catenary_a", "catenary_x0",
    "catenary_y0", "sag", "group",
];

fn host() -> Host {
    let h = Host::with_spec(Spec::game()).unwrap();
    {
        let mut w = h.world.borrow_mut();
        for c in COLS {
            w.set(c, "kind", "num").unwrap();
        }
        w.set("linear_density", "kind", "num").unwrap();
    }
    attach(&h).unwrap();
    h
}

// a bridge of 10 links between two fixed points, a ball on it, ground below
const SCENE: &str = r#"
local function point(n, x, y, w, r, mask)
  ecs.spawn(n, {x = x, y = y, old_x = x, old_y = y, inv_mass = w, radius = r, layer = 1, mask = mask,
    contact = 0, contact_nx = 0, contact_ny = 0})
end
local function link(n, a, b, rest, layer)
  local c = {node_a = ecs.id(a), node_b = ecs.id(b), rest_len = rest, compliance = 0, mode = 0, lambda = 0, force_max = 0}
  if layer then c.layer = layer c.friction = 0.8 end
  ecs.spawn(n, c)
end
ecs.spawn('ground', {x1 = 0, y1 = 500, x2 = 800, y2 = 500, friction = 0.5, layer = 1})
point('p0', 100, 300, 0, 0, 1)
local prev = 'p0'
for i = 1, 9 do
  point('p' .. i, 100 + i * 40, 300, 1, 3, 1)
  link('b' .. i, prev, 'p' .. i, 40, 4)
  prev = 'p' .. i
end
point('p10', 500, 300, 0, 0, 1)
link('b10', prev, 'p10', 40, 4)
point('ball', 300, 200, 1, 12, 7)
return 'ok'
"#;

fn run(h: &Host, ticks: u32) {
    let src = format!(
        "for t = 1, {} do ecs.fill('force_max', 0) for i = 1, 16 do ecs.integrate_verlet(1/960, 0, 980, 0.999) ecs.solve_distance(1/960, 1) ecs.project_edges(1) ecs.project_segments(1) end end return 'ok'",
        ticks
    );
    assert_eq!(h.probe(&src).unwrap(), "ok");
}

// every x and y with all the digits, so two runs can be compared bit for bit
fn state(h: &Host) -> String {
    h.probe("local t = {} for i = 0, 10 do local n = 'p' .. i t[#t + 1] = string.format('%.17g,%.17g,%.17g,%.17g', ecs.get(n, 'x'), ecs.get(n, 'y'), ecs.get(n, 'old_x'), ecs.get(n, 'old_y')) end t[#t + 1] = string.format('%.17g,%.17g', ecs.get('ball', 'x'), ecs.get('ball', 'y')) return table.concat(t, ';')")
        .unwrap()
}

fn num(h: &Host, e: &str, c: &str) -> f64 {
    h.world.borrow().num(e, c).unwrap()
}

#[test]
fn a_ball_rests_on_the_bridge_and_does_not_sink_under_it() {
    let h = host();
    assert_eq!(h.probe(SCENE).unwrap(), "ok");
    run(&h, 300);
    let (by, jy) = (num(&h, "ball", "y"), num(&h, "p5", "y"));
    // the ball lies on the road: its centre one radius above the road, never below it
    assert!(by < jy && (jy - by - 12.0).abs() < 0.5, "ball {} joint {}", by, jy);
    assert!(by < 488.0);
    // the road links felt a force; the bridge held its lengths
    assert!(num(&h, "b5", "force_max") > 0.0);
    let dx = num(&h, "p6", "x") - num(&h, "p5", "x");
    let dy = num(&h, "p6", "y") - num(&h, "p5", "y");
    assert!(((dx * dx + dy * dy).sqrt() - 40.0).abs() < 0.5);
}

#[test]
fn the_contact_of_the_edges_is_not_wiped_by_the_segments_call() {
    // project_edges sets contact, project_segments runs after it in the same substep and must
    // keep it (it once overwrote it with 0)
    let h = host();
    assert_eq!(h.probe(SCENE).unwrap(), "ok");
    run(&h, 300);
    assert_eq!(num(&h, "ball", "contact"), 1.0);
    assert!(num(&h, "ball", "contact_ny") < -0.9, "the normal points up (y grows down)");
    // integrate_verlet clears it again for the next substep
    assert_eq!(h.probe("ecs.integrate_verlet(1/960, 0, 0, 1) return 'ok'").unwrap(), "ok");
    assert_eq!(num(&h, "ball", "contact"), 0.0);
}

#[test]
fn two_runs_of_the_same_scene_agree_bit_for_bit() {
    let (a, b) = (host(), host());
    for h in [&a, &b] {
        assert_eq!(h.probe(SCENE).unwrap(), "ok");
        run(h, 200);
    }
    assert_eq!(state(&a), state(&b));
}

#[test]
fn a_world_loaded_from_a_snapshot_goes_on_the_same_way() {
    let a = host();
    assert_eq!(a.probe(SCENE).unwrap(), "ok");
    run(&a, 100);
    let snap = a.world.borrow().snapshot();
    run(&a, 100);
    let w = World::from_snapshot_spec(Spec::game(), &snap, Vec::new()).unwrap();
    let b = Host::with_world(w).unwrap();
    attach(&b).unwrap();
    // numeric ids (generation and index) are not stored in a snapshot: after a load the
    // indices may differ, so a program rebuilds its link ends from the names
    let fix = "for i = 1, 10 do local n = 'b' .. i ecs.set(n, 'node_a', ecs.id(i == 1 and 'p0' or 'p' .. (i - 1))) ecs.set(n, 'node_b', ecs.id('p' .. i)) end return 'ok'";
    assert_eq!(b.probe(fix).unwrap(), "ok");
    run(&b, 100);
    // the order of the columns may change in a loaded world, so only the physics must agree
    // closely (the solving order follows the dense order, which a load may change), not to the last bit
    let (sa, sb) = (state(&a), state(&b));
    if sa != sb {
        let va: Vec<f64> = sa.split([';', ',']).map(|v| v.parse().unwrap()).collect();
        let vb: Vec<f64> = sb.split([';', ',']).map(|v| v.parse().unwrap()).collect();
        let worst = va.iter().zip(&vb).map(|(p, q)| (p - q).abs()).fold(0.0, f64::max);
        assert!(worst < 0.5, "snapshot run differs by {}", worst);
    }
}

#[test]
fn a_killed_point_makes_its_links_vanish_without_a_panic() {
    let h = host();
    assert_eq!(h.probe(SCENE).unwrap(), "ok");
    let before: i64 = h.probe("return ecs.solve_distance(1/960, 1)").unwrap().parse().unwrap();
    assert_eq!(before, 10);
    // two links end at p4
    assert_eq!(h.probe("ecs.kill('p4') return ecs.solve_distance(1/960, 1)").unwrap(), "8");
    run(&h, 20);
}

// a square body: four points, six rigid links (sides and diagonals), four edges, all in group 1.
// a ball falls on it. closed shapes, circles and the ground together
const SQUARE: &str = r#"
local function point(n, x, y, w, r, layer, mask, g)
  ecs.spawn(n, {x = x, y = y, old_x = x, old_y = y, inv_mass = w, radius = r, layer = layer, mask = mask, group = g,
    contact = 0, contact_nx = 0, contact_ny = 0})
end
ecs.spawn('ground', {x1 = 0, y1 = 500, x2 = 800, y2 = 500, friction = 0.5, layer = 1})
local c = {{300, 400}, {340, 400}, {340, 440}, {300, 440}}
for i = 1, 4 do point('s' .. i, c[i][1], c[i][2], 1, 3, 1, 7, 1) end
local function link(n, a, b, rest, edge)
  local t = {node_a = ecs.id(a), node_b = ecs.id(b), rest_len = rest, compliance = 0, mode = 0, lambda = 0, force_max = 0}
  if edge then t.layer = 4 t.group = 1 t.friction = 0.5 end
  ecs.spawn(n, t)
end
for i = 1, 4 do link('e' .. i, 's' .. i, 's' .. (i % 4 + 1), 40, true) end
link('d1', 's1', 's3', 56.5685, false)
link('d2', 's2', 's4', 56.5685, false)
point('ball', 320, 200, 1, 12, 2, 7, 0)
return 'ok'
"#;

fn run_all(h: &Host, ticks: u32) {
    let src = format!(
        "for t = 1, {} do for i = 1, 16 do ecs.integrate_verlet(1/960, 0, 980, 0.999) ecs.solve_distance(1/960, 1) ecs.project_points(1) ecs.project_edges(1) ecs.project_segments(1) end end return 'ok'",
        ticks
    );
    assert_eq!(h.probe(&src).unwrap(), "ok");
}

#[test]
fn a_closed_body_and_a_ball_rest_on_the_ground_and_on_each_other() {
    let h = host();
    assert_eq!(h.probe(SQUARE).unwrap(), "ok");
    run_all(&h, 400);
    // the body lies on the ground, keeps its shape and does not push itself apart
    for i in 1..=4 {
        let y = num(&h, &format!("s{}", i), "y");
        assert!(y <= 497.5 && y > 380.0, "corner {} at {}", i, y);
    }
    let side = ((num(&h, "s2", "x") - num(&h, "s1", "x")).powi(2) + (num(&h, "s2", "y") - num(&h, "s1", "y")).powi(2)).sqrt();
    assert!((side - 40.0).abs() < 1.0, "side {}", side);
    // the ball did not fall through the body: it is above the lowest corner and not inside the square
    let (bx, by) = (num(&h, "ball", "x"), num(&h, "ball", "y"));
    let top = ["s1", "s2", "s3", "s4"].iter().map(|n| num(&h, n, "y")).fold(f64::INFINITY, f64::min);
    let inside = (300.0..340.0).contains(&bx) && by > top && by < top + 40.0;
    assert!(!inside, "ball inside the body: ({}, {}) top {}", bx, by, top);
    assert!(by < 490.0);
}

#[test]
fn project_points_path_and_distances_work_from_lua() {
    let h = host();
    assert_eq!(h.probe(SCENE).unwrap(), "ok");
    // the bridge is a line p0 - p1 - ... - p10: 10 links of 40
    let r = h.probe("local ids, ds = ecs.distances(ecs.id('p0')) return #ids .. ' ' .. ds[#ds]").unwrap();
    assert_eq!(r, "11 400.0");
    // the way from one end to the other: 11 points, cost 400, the first and last are the ends
    let r = h.probe("local way, c = ecs.path(ecs.id('p0'), ecs.id('p10')) return #way .. ' ' .. c .. ' ' .. tostring(way[1] == ecs.id('p0')) .. ' ' .. tostring(way[#way] == ecs.id('p10'))").unwrap();
    assert_eq!(r, "11 400.0 true true");
    // by hops
    assert_eq!(h.probe("local _, c = ecs.path(ecs.id('p0'), ecs.id('p10'), 2) return c").unwrap(), "10.0");
    // cut the bridge: no way
    assert_eq!(h.probe("ecs.kill('b5') local w = ecs.path(ecs.id('p0'), ecs.id('p10')) return tostring(w)").unwrap(), "nil");
    // two overlapping balls part
    assert_eq!(h.probe("ecs.spawn('q1', {x = 700, y = 100, old_x = 700, old_y = 100, inv_mass = 1, radius = 10, layer = 2, mask = 2}) ecs.spawn('q2', {x = 710, y = 100, old_x = 710, old_y = 100, inv_mass = 1, radius = 10, layer = 2, mask = 2}) return ecs.project_points(1)").unwrap(), "1");
    assert!((num(&h, "q2", "x") - num(&h, "q1", "x") - 20.0).abs() < 1e-9);
    // a bad cost is refused
    assert!(h.probe("return ecs.distances(ecs.id('p0'), 7)").is_err());
}
