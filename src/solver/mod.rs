// the solver: constraints over num columns, without any knowledge of what the program is
// (points, distance links, barriers, catenary cables). see docs/dev.md and the spec.
//
// files: data (arrays), access (the glue to a World), integrate, distance, contact, query,
// catenary (the math over the arrays), and this file: the lua functions
pub mod access;
pub mod angular;
pub mod catenary;
pub mod contact;
pub mod data;
pub mod distance;
pub mod integrate;
pub mod path;
pub mod points;
pub mod query;
#[cfg(test)]
mod scene_tests;
#[cfg(test)]
mod bench;

use std::cell::RefCell;
use std::rc::Rc;

use crate::ecs::World;
use crate::host::Host;
use access::{Scratch, CONTACT, OLD, POS, SPIN};
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

// registers one lua api function as ecs.<name>: parses nothing itself, borrows the world and the scratch for the
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

// gives the world of a host the lua api functions ecs.solve_tick, ecs.integrate_verlet,
// ecs.solve_distance, ecs.damp_angular, ecs.project_segments, ecs.project_edges, ecs.project_points,
// ecs.path, ecs.distances, ecs.query_point, ecs.raycast_segments, ecs.update_catenary,
// ecs.apply_catenary_load and ecs.sample_catenary (each handler below is marked "lua api:")
pub fn attach(host: &Host) -> Result<(), String> {
    let sc = Rc::new(RefCell::new(Scratch::default()));
    reg(host, &sc, "integrate_verlet", integrate_verlet)?;
    reg(host, &sc, "solve_distance", solve_distance)?;
    reg(host, &sc, "damp_angular", damp_angular)?;
    reg(host, &sc, "solve_tick", solve_tick)?;
    reg(host, &sc, "project_segments", project_segments)?;
    reg(host, &sc, "project_edges", project_edges)?;
    reg(host, &sc, "project_points", project_points)?;
    reg(host, &sc, "path", path)?;
    reg(host, &sc, "distances", distances)?;
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

// lua api: ecs.integrate_verlet(dt, gx, gy, damping) -> moved
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

// lua api: ecs.solve_distance(dt, iterations) -> links solved
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

// lua api: ecs.damp_angular(spin, bend) -> changed
fn damp_angular(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let spin = num_arg(args, 0, "spin")?;
    let bend = num_arg(args, 1, "bend")?;
    writable(w)?;
    released(s, |s| {
        let Some(pc) = access::gather_points(w, s) else { return Ok(0) };
        let Some(_) = access::gather_links(w, s) else { return Ok(0) };
        if s.pts.is_empty() || s.links.is_empty() {
            return Ok(0);
        }
        let n = angular::damp(&mut s.pts, &s.links, spin, bend);
        access::scatter_points(w, s, &pc, OLD)?;
        Ok(n as i64)
    })
}

// lua api: ecs.solve_tick(dt, substeps, gx, gy, damping [, spin, bend]) -> steps
// one whole tick in one call: the columns are read once, `substeps` steps of dt / substeps run on
// the arrays, the columns are written once. each step does what a program would call one by one:
// integrate_verlet(h, gx, gy, damping), apply_catenary_load(h, gy), solve_distance(h, 1),
// damp_angular(spin, bend), project_edges(1), project_segments(1). the result is the same as those
// calls in that order (the same numbers, the same order of work); reading and writing the world
// 6 times per step is what this saves. returns the number of steps
fn solve_tick(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let dt = pos_arg(args, 0, "dt")?;
    let n = num_arg(args, 1, "substeps")?;
    if n.fract() != 0.0 || !(1.0..=256.0).contains(&n) {
        return Err(bad_arg("substeps"));
    }
    let n = n as usize;
    let gx = num_arg(args, 2, "gx")?;
    let gy = num_arg(args, 3, "gy")?;
    let damping = num_arg(args, 4, "damping")?;
    let spin = opt_num_arg(args, 5, "spin")?.unwrap_or(0.0);
    let bend = opt_num_arg(args, 6, "bend")?.unwrap_or(0.0);
    writable(w)?;
    released(s, |s| {
        let Some(pc) = access::gather_points(w, s) else { return Ok(0) };
        if s.pts.is_empty() {
            return Ok(0);
        }
        let lc = access::gather_links(w, s);
        let has_links = lc.is_some() && !s.links.is_empty();
        let has_edges = access::gather_edges(w, s).is_some() && !s.edges.is_empty();
        let has_segs = access::gather_segs(w, s).is_some() && !s.segs.is_empty();
        let has_cables = access::gather_cables(w, s).is_some() && s.cables.rho.iter().any(|&r| r > 0.0);
        let mut adj = angular::Adj::default();
        let damp = has_links && (angular::wants_spin(&s.links, spin) || angular::wants_bend(&s.pts, bend));
        if damp && angular::wants_bend(&s.pts, bend) {
            angular::adjacency(s.pts.len(), &s.links, &mut adj);
        }
        let h = dt / n as f64;
        for r in s.pts.roll.iter_mut() {
            *r *= h;
        }
        for _ in 0..n {
            integrate::verlet(&mut s.pts, h, gx, gy, damping);
            if has_cables {
                for i in 0..s.cables.ent.len() {
                    let rho = s.cables.rho[i];
                    let Some(ends) = cable_ends(s, i) else { continue };
                    if rho <= 0.0 {
                        continue;
                    }
                    let len = s.cables.len[i];
                    let sh = catenary::shape_loaded(ends, len, gy, s.cables.slack[i]);
                    let (fa, fb) = catenary::end_forces(&sh, ends, len, rho, gy);
                    push_end(s, s.cables.a[i], fa, h);
                    push_end(s, s.cables.b[i], fb, h);
                }
            }
            if has_links {
                distance::solve(&mut s.pts, &mut s.links, h, 1);
            }
            if damp {
                angular::damp_with(&mut s.pts, &s.links, &adj, spin, bend);
            }
            if has_edges {
                contact::project_edges(&mut s.pts, &s.edges, 1);
            }
            if has_segs {
                contact::project_segments(&mut s.pts, &s.segs, 1);
            }
        }
        for r in s.pts.roll.iter_mut() {
            *r /= h;
        }
        access::scatter_points(w, s, &pc, POS | OLD | CONTACT | SPIN)?;
        if let (true, Some(lc)) = (has_links, lc) {
            access::scatter_links(w, s, &lc)?;
        }
        Ok(n as i64)
    })
}

// lua api: ecs.project_segments(iterations [, dt]) -> pushes
fn project_segments(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let iterations = iterations_arg(args, 0)?;
    // the step (optional): with it a point with a spin turns its rim; without it nothing turns
    let step = match opt_num_arg(args, 1, "dt")? {
        Some(v) if v > 0.0 => v,
        Some(_) => return Err(bad_arg("dt")),
        None => 0.0,
    };
    writable(w)?;
    released(s, |s| {
        let Some(pc) = access::gather_points(w, s) else { return Ok(0) };
        for r in s.pts.roll.iter_mut() {
            *r *= step;
        }
        if access::gather_segs(w, s).is_none() || s.pts.is_empty() || s.segs.is_empty() {
            return Ok(0);
        }
        let n = contact::project_segments(&mut s.pts, &s.segs, iterations);
        let back = if step > 0.0 { SPIN } else { 0 };
        for r in s.pts.roll.iter_mut() {
            *r = if step > 0.0 { *r / step } else { f64::NAN };
        }
        access::scatter_points(w, s, &pc, POS | OLD | CONTACT | back)?;
        Ok(n as i64)
    })
}

// lua api: ecs.project_edges(iterations [, dt]) -> pushes
fn project_edges(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let iterations = iterations_arg(args, 0)?;
    // the step (optional): with it a point with a spin turns its rim; without it nothing turns
    let step = match opt_num_arg(args, 1, "dt")? {
        Some(v) if v > 0.0 => v,
        Some(_) => return Err(bad_arg("dt")),
        None => 0.0,
    };
    writable(w)?;
    released(s, |s| {
        let Some(pc) = access::gather_points(w, s) else { return Ok(0) };
        for r in s.pts.roll.iter_mut() {
            *r *= step;
        }
        if access::gather_edges(w, s).is_none() || s.pts.is_empty() || s.edges.is_empty() {
            return Ok(0);
        }
        let n = contact::project_edges(&mut s.pts, &s.edges, iterations);
        let back = if step > 0.0 { SPIN } else { 0 };
        for r in s.pts.roll.iter_mut() {
            *r = if step > 0.0 { *r / step } else { f64::NAN };
        }
        access::scatter_points(w, s, &pc, POS | OLD | CONTACT | back)?;
        Ok(n as i64)
    })
}

// lua api: ecs.project_points(iterations) -> pushes
// every pair of round points that overlap is pushed apart (see points.rs)
fn project_points(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<i64> {
    let iterations = iterations_arg(args, 0)?;
    writable(w)?;
    released(s, |s| {
        let Some(pc) = access::gather_points(w, s) else { return Ok(0) };
        if s.pts.len() < 2 {
            return Ok(0);
        }
        // links only tell which pairs are joined: no links is fine
        let _ = access::gather_links(w, s);
        let n = points::project_points(&mut s.pts, &s.links, iterations);
        access::scatter_points(w, s, &pc, POS | OLD | CONTACT)?;
        Ok(n as i64)
    })
}

// the cost of a link: 0 its length now (default), 1 its rest length, 2 one per link
fn cost_arg(args: Args, i: usize) -> mlua::Result<path::Cost> {
    Ok(match opt_num_arg(args, i, "cost")? {
        None | Some(0.0) => path::Cost::Length,
        Some(1.0) => path::Cost::Rest,
        Some(2.0) => path::Cost::Hops,
        Some(_) => return Err(bad_arg("cost")),
    })
}

// a point given by its id (the number of ecs.id); None when it is not a point
fn point_arg(w: &World, s: &Scratch, args: Args, i: usize, name: &str) -> mlua::Result<Option<usize>> {
    let v = num_arg(args, i, name)?;
    if v.fract() != 0.0 {
        return Err(bad_arg(name));
    }
    Ok(w.index_of(v as i64).and_then(|e| access::slot(s, e)))
}

// lua api: ecs.path(from, to [, cost]) -> ids, cost or nil
// ecs.path(from, to [, cost]) -> ids of the points on the cheapest way (both ends included)
// and its cost, or nil. A* with a straight-line guide when the cost is the length
fn path(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<(Option<Vec<i64>>, Option<f64>)> {
    let cost = cost_arg(args, 2)?;
    released(s, |s| {
        if access::gather_points(w, s).is_none() {
            return Ok((None, None));
        }
        let (Some(a), Some(b)) = (point_arg(w, s, args, 0, "from").map_err(|e| e.to_string())?, point_arg(w, s, args, 1, "to").map_err(|e| e.to_string())?) else {
            return Ok((None, None));
        };
        let _ = access::gather_links(w, s);
        let g = path::Graph::build(&s.pts, &s.links, cost);
        Ok(match path::path(&g, &s.pts, a, b, cost == path::Cost::Length) {
            Some((way, c)) => (Some(way.iter().map(|&k| w.id_at(s.pts.ent[k as usize])).collect()), Some(c)),
            None => (None, None),
        })
    })
}

// lua api: ecs.distances(from [, cost]) -> ids, costs
// ecs.distances(from [, cost]) -> ids, distances: every point that can be reached, with its cost
fn distances(w: &mut World, s: &mut Scratch, args: Args) -> mlua::Result<(Vec<i64>, Vec<f64>)> {
    let cost = cost_arg(args, 1)?;
    released(s, |s| {
        if access::gather_points(w, s).is_none() {
            return Ok((Vec::new(), Vec::new()));
        }
        let Some(a) = point_arg(w, s, args, 0, "from").map_err(|e| e.to_string())? else {
            return Ok((Vec::new(), Vec::new()));
        };
        let _ = access::gather_links(w, s);
        let g = path::Graph::build(&s.pts, &s.links, cost);
        let d = path::distances(&g, a);
        let (mut ids, mut ds) = (Vec::new(), Vec::new());
        for (k, &v) in d.iter().enumerate() {
            if v.is_finite() {
                ids.push(w.id_at(s.pts.ent[k]));
                ds.push(v);
            }
        }
        Ok((ids, ds))
    })
}

// lua api: ecs.query_point(x, y, max_dist [, mask]) -> id, dist or nil
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

// lua api: ecs.raycast_segments(x, y, dx, dy, max_dist [, mask]) -> id, hx, hy, nx, ny, t or nil
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

// lua api: ecs.update_catenary(gy) -> cables
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
            let sh = catenary::shape_loaded(ends, s.cables.len[i], gy, s.cables.slack[i]);
            // the tension at the end that pulls harder: the length of the force on that end
            let (fa, fb) = catenary::end_forces(&sh, ends, s.cables.len[i], s.cables.rho[i], gy);
            let t = fa.0.hypot(fa.1).max(fb.0.hypot(fb.1));
            s.cables.tension[i] = if t.is_finite() { t } else { 0.0 };
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

// lua api: ecs.apply_catenary_load(dt, gy) -> cables
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
            let sh = catenary::shape_loaded(ends, len, gy, s.cables.slack[i]);
            let (fa, fb) = catenary::end_forces(&sh, ends, len, rho, gy);
            push_end(s, s.cables.a[i], fa, dt);
            push_end(s, s.cables.b[i], fb, dt);
            n += 1;
        }
        access::scatter_points(w, s, &pc, POS)?;
        Ok(n)
    })
}

// lua api: ecs.sample_catenary(id, t) -> x, y
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

    const NAMES: [&str; 14] = [
        "integrate_verlet",
        "solve_distance",
        "damp_angular",
        "solve_tick",
        "project_segments",
        "project_edges",
        "project_points",
        "path",
        "distances",
        "query_point",
        "raycast_segments",
        "update_catenary",
        "apply_catenary_load",
        "sample_catenary",
    ];

    #[test]
    fn registers_all_twelve() {
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
