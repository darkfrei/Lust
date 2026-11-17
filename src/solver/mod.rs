// the solver: constraints over num columns, without any knowledge of what the program is
// (points, distance links, barriers, catenary cables). see docs/dev.md and the spec.
//
// files: data (arrays), access (the glue to a World), integrate, distance, contact, query,
// catenary (the math over the arrays), and this file: the lua functions
pub mod access;
pub mod catenary;
pub mod contact;
pub mod data;
pub mod distance;
pub mod integrate;
pub mod query;
#[cfg(test)]
mod scene_tests;

use std::cell::RefCell;
use std::rc::Rc;

use crate::ecs::World;
use crate::host::Host;
use access::{Scratch, CONTACT, OLD, POS};
use data::NONE;
use mlua::{IntoLuaMulti, MultiValue, Value};

type Args<'a> = &'a [Value];

// ---------- arguments ----------

fn bad_arg(name: &str) -> mlua::Error {
    mlua::Error::runtime(format!("bad_arg: {}", name))
}

// a required finite number (an integer or a float)
fn num_arg(args: Args, i: usize, name: &str) -> mlua::Result<f64> {
    let v = match args.get(i) {
        Some(Value::Integer(n)) => *n as f64,
        Some(Value::Number(f)) => *f,
        _ => return Err(bad_arg(name)),
    };
    if v.is_finite() {
        Ok(v)
    } else {
        Err(bad_arg(name))
    }
}

// an optional number: missing or nil is None, anything else must be a finite number
fn opt_num_arg(args: Args, i: usize, name: &str) -> mlua::Result<Option<f64>> {
    match args.get(i) {
        None | Some(Value::Nil) => Ok(None),
        Some(_) => num_arg(args, i, name).map(Some),
    }
}

// a whole number 1..=64
fn iterations_arg(args: Args, i: usize) -> mlua::Result<usize> {
    let v = num_arg(args, i, "iterations")?;
    if v.fract() != 0.0 || !(1.0..=64.0).contains(&v) {
        return Err(bad_arg("iterations"));
    }
    Ok(v as usize)
}

// a positive number
fn pos_arg(args: Args, i: usize, name: &str) -> mlua::Result<f64> {
    let v = num_arg(args, i, name)?;
    if v > 0.0 {
        Ok(v)
    } else {
        Err(bad_arg(name))
    }
}

fn mask_arg(args: Args, i: usize) -> mlua::Result<Option<u32>> {
    Ok(opt_num_arg(args, i, "mask")?.map(access::to_bits))
}

fn writable(w: &World) -> mlua::Result<()> {
    if w.read_only {
        Err(mlua::Error::runtime("readonly"))
    } else {
        Ok(())
    }
}

fn fail(e: String) -> mlua::Error {
    mlua::Error::runtime(e)
}

// ---------- registration ----------

type Func<R> = fn(&mut World, &mut Scratch, Args) -> mlua::Result<R>;

// registers one function: parses nothing itself, borrows the world and the scratch for the
// call and hands the raw arguments to f
fn reg<R: IntoLuaMulti + 'static>(host: &Host, scratch: &Rc<RefCell<Scratch>>, name: &str, f: Func<R>) -> Result<(), String> {
    let world = Rc::clone(&host.world);
    let sc = Rc::clone(scratch);
    let func = host
        .lua()
        .create_function(move |_, args: MultiValue| {
            let list: Vec<Value> = args.into_iter().collect();
            let mut w = world.try_borrow_mut().map_err(|_| mlua::Error::runtime("busy"))?;
            let mut s = sc.try_borrow_mut().map_err(|_| mlua::Error::runtime("busy"))?;
            f(&mut w, &mut s, &list)
        })
        .map_err(|e| e.to_string())?;
    host.add_to_ecs(name, func)
}

// gives the world of a host the functions ecs.integrate_verlet, ecs.solve_distance,
// ecs.project_segments, ecs.project_edges, ecs.query_point, ecs.raycast_segments,
// ecs.update_catenary, ecs.apply_catenary_load and ecs.sample_catenary
pub fn attach(host: &Host) -> Result<(), String> {
    let sc = Rc::new(RefCell::new(Scratch::default()));
    reg(host, &sc, "integrate_verlet", integrate_verlet)?;
    reg(host, &sc, "solve_distance", solve_distance)?;
    reg(host, &sc, "project_segments", project_segments)?;
    reg(host, &sc, "project_edges", project_edges)?;
    reg(host, &sc, "query_point", query_point)?;
    reg(host, &sc, "raycast_segments", raycast_segments)?;
    reg(host, &sc, "update_catenary", update_catenary)?;
    reg(host, &sc, "apply_catenary_load", apply_catenary_load)?;
    reg(host, &sc, "sample_catenary", sample_catenary)?;
    Ok(())
}

// ---------- the functions ----------

// runs body between a gather and a release: the release happens on every path
fn released<R>(s: &mut Scratch, body: impl FnOnce(&mut Scratch) -> Result<R, String>) -> mlua::Result<R> {
    let r = body(s);
    access::release(s);
    r.map_err(fail)
}

fn integrate_verlet(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let dt = pos_arg(args, 0, "dt")?;
    let gx = num_arg(args, 1, "gx")?;
    let gy = num_arg(args, 2, "gy")?;
    let damping = num_arg(args, 3, "damping")?;
    writable(w)?;
    released(s, |s| {
        let Some(c) = access::gather_points(w, s) else { return Ok(0) };
        if s.pts.is_empty() {
            return Ok(0);
        }
        let moved = integrate::verlet(&mut s.pts, dt, gx, gy, damping);
        access::scatter_points(w, s, &c, POS | OLD | CONTACT)?;
        Ok(moved as i64)
    })
}

fn solve_distance(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let dt = pos_arg(args, 0, "dt")?;
    let iterations = iterations_arg(args, 1)?;
    writable(w)?;
    released(s, |s| {
        let Some(pc) = access::gather_points(w, s) else { return Ok(0) };
        let Some(lc) = access::gather_links(w, s) else { return Ok(0) };
        if s.pts.is_empty() || s.links.is_empty() {
            return Ok(0);
        }
        let n = distance::solve(&mut s.pts, &mut s.links, dt, iterations);
        access::scatter_points(w, s, &pc, POS)?;
        access::scatter_links(w, s, &lc)?;
        Ok(n as i64)
    })
}

fn project_segments(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let iterations = iterations_arg(args, 0)?;
    writable(w)?;
    released(s, |s| {
        let Some(pc) = access::gather_points(w, s) else { return Ok(0) };
        if access::gather_segs(w, s).is_none() || s.pts.is_empty() || s.segs.is_empty() {
            return Ok(0);
        }
        let n = contact::project_segments(&mut s.pts, &s.segs, iterations);
        access::scatter_points(w, s, &pc, POS | OLD | CONTACT)?;
        Ok(n as i64)
    })
}

fn project_edges(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let iterations = iterations_arg(args, 0)?;
    writable(w)?;
    released(s, |s| {
        let Some(pc) = access::gather_points(w, s) else { return Ok(0) };
        if access::gather_edges(w, s).is_none() || s.pts.is_empty() || s.edges.is_empty() {
            return Ok(0);
        }
        let n = contact::project_edges(&mut s.pts, &s.edges, iterations);
        access::scatter_points(w, s, &pc, POS | OLD | CONTACT)?;
        Ok(n as i64)
    })
}

fn query_point(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<MultiValue> {
    let x = num_arg(args, 0, "x")?;
    let y = num_arg(args, 1, "y")?;
    let max_dist = num_arg(args, 2, "max_dist")?;
    let mask = mask_arg(args, 3)?;
    let found = released(s, |s| {
        if access::gather_points(w, s).is_none() {
            return Ok(None);
        }
        let hit = query::nearest(&s.pts, x, y, max_dist, mask);
        Ok(hit.and_then(|(pos, d)| s.pts.ent.get(pos).map(|e| (w.id_at(*e), d))))
    })?;
    Ok(match found {
        Some((id, d)) => MultiValue::from_iter([Value::Integer(id), Value::Number(d)]),
        None => MultiValue::from_iter([Value::Nil]),
    })
}

fn raycast_segments(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<MultiValue> {
    let x = num_arg(args, 0, "x")?;
    let y = num_arg(args, 1, "y")?;
    let dx = num_arg(args, 2, "dx")?;
    let dy = num_arg(args, 3, "dy")?;
    let max_dist = num_arg(args, 4, "max_dist")?;
    let mask = mask_arg(args, 5)?;
    if dx == 0.0 && dy == 0.0 {
        return Err(bad_arg("direction"));
    }
    if access::gather_segs(w, s).is_none() {
        return Ok(MultiValue::from_iter([Value::Nil]));
    }
    let hit = query::raycast(&s.segs, x, y, dx, dy, max_dist, mask);
    Ok(match hit.and_then(|h| s.segs.ent.get(h.seg).map(|e| (w.id_at(*e), h))) {
        Some((id, h)) => MultiValue::from_iter([
            Value::Integer(id),
            Value::Number(h.hx),
            Value::Number(h.hy),
            Value::Number(h.nx),
            Value::Number(h.ny),
            Value::Number(h.t),
        ]),
        None => MultiValue::from_iter([Value::Nil]),
    })
}

// the four numbers of the ends of a cable, or None when an end is missing
fn cable_ends(s: &Scratch, i: usize) -> Option<[f64; 4]> {
    let a = *s.cables.a.get(i)?;
    let b = *s.cables.b.get(i)?;
    if a == NONE || b == NONE {
        return None;
    }
    let (a, b) = (a as usize, b as usize);
    Some([*s.pts.x.get(a)?, *s.pts.y.get(a)?, *s.pts.x.get(b)?, *s.pts.y.get(b)?])
}

fn update_catenary(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let gy = num_arg(args, 0, "gy")?;
    writable(w)?;
    released(s, |s| {
        if access::gather_points(w, s).is_none() {
            return Ok(0);
        }
        let Some(cc) = access::gather_cables(w, s) else { return Ok(0) };
        let n = s.cables.ent.len();
        for i in 0..n {
            let Some(ends) = cable_ends(s, i) else { continue };
            let sh = catenary::shape(ends, s.cables.len[i], gy);
            if let Some(slot) = s.cables.shape.get_mut(i) {
                *slot = sh;
            }
        }
        access::scatter_cables(w, s, &cc)?;
        Ok(n as i64)
    })
}

// moves the end point at position p by the force f over dt, when it is free and finite
fn push_end(s: &mut Scratch, p: u32, f: (f64, f64), dt: f64) {
    let p = p as usize;
    let Some(&wt) = s.pts.w.get(p) else { return };
    if wt <= 0.0 {
        return;
    }
    let nx = s.pts.x[p] + wt * f.0 * dt * dt;
    let ny = s.pts.y[p] + wt * f.1 * dt * dt;
    if nx.is_finite() && ny.is_finite() {
        s.pts.x[p] = nx;
        s.pts.y[p] = ny;
    }
}

fn apply_catenary_load(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let dt = pos_arg(args, 0, "dt")?;
    let gy = num_arg(args, 1, "gy")?;
    writable(w)?;
    released(s, |s| {
        let (Some(pc), Some(_)) = (access::gather_points(w, s), access::gather_cables(w, s)) else { return Ok(0) };
        let mut n = 0;
        for i in 0..s.cables.ent.len() {
            let rho = s.cables.rho[i];
            let Some(ends) = cable_ends(s, i) else { continue };
            if rho <= 0.0 {
                continue;
            }
            let len = s.cables.len[i];
            let sh = catenary::shape(ends, len, gy);
            let (fa, fb) = catenary::end_forces(&sh, ends, len, rho, gy);
            push_end(s, s.cables.a[i], fa, dt);
            push_end(s, s.cables.b[i], fb, dt);
            n += 1;
        }
        access::scatter_points(w, s, &pc, POS)?;
        Ok(n)
    })
}

fn sample_catenary(w: &mut World, _s: &mut Scratch, args: Args) -> mlua::Result<MultiValue> {
    let id = num_arg(args, 0, "id")?;
    if id.fract() != 0.0 || id.abs() > 9.0e15 {
        return Err(bad_arg("id"));
    }
    let t = num_arg(args, 1, "t")?;
    let Some(c) = access::read_cable(w, id as i64) else { return Ok(MultiValue::from_iter([Value::Nil])) };
    let (x, y) = catenary::sample(&c.shape, c.ends, c.len, t);
    Ok(MultiValue::from_iter([Value::Number(x), Value::Number(y)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::Spec;

    fn host() -> Host {
        let h = Host::with_spec(Spec::game()).unwrap();
        attach(&h).unwrap();
        h
    }

    const NAMES: [&str; 9] = [
        "integrate_verlet",
        "solve_distance",
        "project_segments",
        "project_edges",
        "query_point",
        "raycast_segments",
        "update_catenary",
        "apply_catenary_load",
        "sample_catenary",
    ];

    #[test]
    fn registers_all_nine() {
        let h = host();
        for n in NAMES {
            assert_eq!(h.probe(&format!("return type(ecs.{})", n)).unwrap(), "function", "{}", n);
        }
    }

    #[test]
    fn attaching_twice_fails() {
        let h = host();
        let e = attach(&h).unwrap_err();
        assert!(e.starts_with("taken"), "{}", e);
    }

    fn bad(h: &Host, call: &str) {
        let e = h.probe(&format!("return tostring(ecs.{})", call)).unwrap_err();
        assert!(e.contains("bad_arg"), "{} -> {}", call, e);
    }

    #[test]
    fn bad_arguments() {
        let h = host();
        bad(&h, "integrate_verlet(nil, 0, 0, 1)");
        bad(&h, "integrate_verlet('1', 0, 0, 1)");
        bad(&h, "integrate_verlet(0/0, 0, 0, 1)");
        bad(&h, "integrate_verlet(0, 0, 0, 1)");
        bad(&h, "integrate_verlet(1, 0, 0)");
        bad(&h, "integrate_verlet(1, 0, 1/0, 1)");
        bad(&h, "solve_distance(0, 4)");
        bad(&h, "solve_distance(0.1, 0)");
        bad(&h, "solve_distance(0.1, 65)");
        bad(&h, "solve_distance(0.1, 1.5)");
        bad(&h, "project_segments(0)");
        bad(&h, "project_edges(65)");
        bad(&h, "query_point(0, 0, nil)");
        bad(&h, "query_point(0, 0, 1, 'a')");
        bad(&h, "raycast_segments(0, 0, 0, 0, 10)");
        bad(&h, "raycast_segments(0, 0, 1, 0)");
        bad(&h, "update_catenary(nil)");
        bad(&h, "apply_catenary_load(0, 9.8)");
        bad(&h, "sample_catenary(1.5, 0.5)");
        bad(&h, "sample_catenary(1, nil)");
    }

    #[test]
    fn read_only_refuses_writers() {
        let h = host();
        h.world.borrow_mut().read_only = true;
        for call in ["integrate_verlet(1, 0, 0, 1)", "solve_distance(0.1, 4)", "update_catenary(9.8)"] {
            let e = h.probe(&format!("return tostring(ecs.{})", call)).unwrap_err();
            assert!(e.contains("readonly"), "{} -> {}", call, e);
        }
        h.world.borrow_mut().read_only = false;
    }
}
