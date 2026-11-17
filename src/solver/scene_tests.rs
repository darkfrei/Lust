// end to end: the solver driven from Lua on a world, as a program does it. the math has its
// own tests in each file; these catch what only shows when the parts work together
use super::attach;
use crate::ecs::{Spec, World};
use crate::host::Host;

const COLS: [&str; 27] = [
    "x", "y", "old_x", "old_y", "inv_mass", "radius", "layer", "mask", "contact", "contact_nx", "contact_ny", "node_a", "node_b",
    "rest_len", "compliance", "mode", "lambda", "force_max", "x1", "y1", "x2", "y2", "friction", "catenary_a", "catenary_x0",
    "catenary_y0", "sag",
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
