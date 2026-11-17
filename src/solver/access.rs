// the glue between a World and the math: gathers num columns into the plain arrays of
// data.rs, and writes results back. the only file of the solver that knows the World.
//
// rules (spec section 2):
// - a column is found by name once per call: world.col(name) then world.live_col; a missing
//   required column means "nothing to do" (None), never an error
// - the entities of a set are walked in the dense order of its SHORTEST required column
//   (ties: the first); every other column is read with Column::get(entity index) (columns
//   keep their own order, so no two columns may be assumed to match)
// - an entity with a missing required value, or a non-finite one, is skipped
// - a ref is a number: the f64 of ecs.id(entity) (generation << 32 | index). it is resolved
//   with World::index_of, so a killed entity (new generation) gives None and the link is
//   skipped. a value that is not an integer in 0..2^53 is skipped
// - writing: World::touch_column(col) for every column to write (first all, then the
//   writes), then World::column_mut(col).get_mut(entity index); non-finite values are never
//   written; a column that does not exist is not written (it is an optional output)
// - no allocation in steady state: the Vecs of Scratch are cleared and refilled
use super::catenary::Shape;
use super::catenary::Kind;
use super::data::{Edges, Links, Points, Segs, NONE};
use crate::ecs::World;

pub const POS: u8 = 1;
pub const OLD: u8 = 2;
pub const CONTACT: u8 = 4;

// cables: links that have catenary_a (and node_a, node_b, rest_len); needs the points
// gathered first (a and b are positions in Scratch::pts)
#[derive(Default)]
pub struct Cables {
    pub ent: Vec<u32>,
    pub a: Vec<u32>,
    pub b: Vec<u32>,
    pub len: Vec<f64>,
    // linear_density, 0 when the column is missing
    pub rho: Vec<f64>,
    // out: to be filled by the caller, written by scatter_cables. gather fills it with
    // Shape::straight()
    pub shape: Vec<Shape>,
}

// everything a call needs, kept between calls so nothing is allocated again. it is only
// scratch: every call fills it from the world first, so it never carries state
#[derive(Default)]
pub struct Scratch {
    pub pts: Points,
    pub links: Links,
    pub edges: Edges,
    pub segs: Segs,
    pub cables: Cables,
    // entity index -> position in pts, or NONE; reset by release
    slot_of: Vec<u32>,
}

// the column numbers a gather found, kept for the scatter that follows
pub struct PointCols {
    x: usize,
    y: usize,
    ox: usize,
    oy: usize,
    contact: Option<usize>,
    cnx: Option<usize>,
    cny: Option<usize>,
}
pub struct LinkCols {
    lambda: Option<usize>,
    fmax: Option<usize>,
}
pub struct EdgeCols {}
pub struct SegCols {}
pub struct CableCols {
    a: Option<usize>,
    x0: Option<usize>,
    y0: Option<usize>,
    sag: Option<usize>,
}

// ---------- small helpers ----------

// a column by name, or None when it is missing, dead or hidden
fn find(world: &World, name: &str) -> Option<usize> {
    let c = world.col(name).and_then(|c| world.live_col(c))?;
    if world.col_hidden(c) {
        None
    } else {
        Some(c)
    }
}

// all the columns of a list, or None when one is missing
fn need<const N: usize>(world: &World, names: [&str; N]) -> Option<[usize; N]> {
    let mut out = [0usize; N];
    for (slot, name) in out.iter_mut().zip(names) {
        *slot = find(world, name)?;
    }
    Some(out)
}

// the entities of the shortest column (the first on ties), in its dense order
fn shortest<'a>(world: &'a World, cols: &[usize]) -> &'a [u32] {
    let mut best: &[u32] = &[];
    for (i, &c) in cols.iter().enumerate() {
        let e = world.column(c).entities();
        if i == 0 || e.len() < best.len() {
            best = e;
        }
    }
    best
}

// the values of one entity in all the columns; None when one is missing or not finite
fn read_all<const N: usize>(world: &World, cols: &[usize; N], e: u32) -> Option<[f64; N]> {
    let mut out = [0.0f64; N];
    for (slot, &c) in out.iter_mut().zip(cols) {
        let v = world.column(c).get(e)?;
        if !v.is_finite() {
            return None;
        }
        *slot = v;
    }
    Some(out)
}

// an optional column: its value, or the default
fn opt(world: &World, col: Option<usize>, e: u32, default: f64) -> f64 {
    col.and_then(|c| world.column(c).get(e)).unwrap_or(default)
}

// a ref value as an entity number: a whole number in 0..2^53
fn ref_id(v: f64) -> Option<i64> {
    if v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v < 9007199254740992.0 {
        Some(v as i64)
    } else {
        None
    }
}

// a ref value as a position in pts, while that entity lives and is a point
fn resolve(world: &World, slot_of: &[u32], v: f64) -> Option<u32> {
    let index = world.index_of(ref_id(v)?)?;
    match slot_of.get(index as usize) {
        Some(&p) if p != NONE => Some(p),
        _ => None,
    }
}

// the two ends of a link-like entity as positions, different from each other
fn ends(world: &World, slot_of: &[u32], va: f64, vb: f64) -> Option<(u32, u32)> {
    let a = resolve(world, slot_of, va)?;
    let b = resolve(world, slot_of, vb)?;
    if a == b {
        None
    } else {
        Some((a, b))
    }
}

// layer and mask as bits: round, negative or non-finite is 0, only the low 32 bits count
pub fn to_bits(v: f64) -> u32 {
    if !v.is_finite() || v < 0.0 {
        return 0;
    }
    (v.round() as u64 & 0xffff_ffff) as u32
}

fn clean_nonneg(v: f64) -> f64 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        0.0
    }
}

// ---------- points ----------

fn push_point(s: &mut Scratch, e: u32, v: [f64; 5], opt3: [f64; 3], group: f64, damp: f64) {
    let p = &mut s.pts;
    p.ent.push(e);
    p.x.push(v[0]);
    p.y.push(v[1]);
    p.ox.push(v[2]);
    p.oy.push(v[3]);
    p.w.push(clean_nonneg(v[4]));
    p.r.push(if opt3[0].is_finite() && opt3[0] > 0.0 { opt3[0] } else { 0.0 });
    p.layer.push(to_bits(opt3[1]));
    p.mask.push(to_bits(opt3[2]));
    p.grp.push(to_bits(group));
    // only 0..1 is a speed factor; anything else (or no column) is NaN: the call's value
    p.damp.push(if (0.0..=1.0).contains(&damp) { damp } else { f64::NAN });
    p.contact.push(0.0);
    p.cnx.push(0.0);
    p.cny.push(0.0);
    let i = e as usize;
    if s.slot_of.len() <= i {
        s.slot_of.resize(i + 1, NONE);
    }
    s.slot_of[i] = (s.pts.ent.len() - 1) as u32;
}

// points: x, y, old_x, old_y, inv_mass required; radius (0), layer (1), mask (1), group (0), damping (the call's) optional.
// fills s.pts (w: negative or NaN becomes 0; contact, cnx, cny are the values in the world, 0 when missing) and s.slot_of.
// None when a required column is missing
pub fn gather_points(world: &World, s: &mut Scratch) -> Option<PointCols> {
    release(s);
    s.pts.clear();
    let req = need(world, ["x", "y", "old_x", "old_y", "inv_mass"])?;
    let (r, l, m) = (find(world, "radius"), find(world, "layer"), find(world, "mask"));
    let (grp, damp) = (find(world, "group"), find(world, "damping"));
    let (cc, cx, cy) = (find(world, "contact"), find(world, "contact_nx"), find(world, "contact_ny"));
    for &e in shortest(world, &req) {
        if let Some(v) = read_all(world, &req, e) {
            push_point(s, e, v, [opt(world, r, e, 0.0), opt(world, l, e, 1.0), opt(world, m, e, 1.0)], opt(world, grp, e, 0.0), opt(world, damp, e, f64::NAN));
            // the contact of an earlier call stays (project_edges, then project_segments in
            // one substep must not wipe each other); only integrate_verlet clears it
            let k = s.pts.contact.len() - 1;
            s.pts.contact[k] = opt(world, cc, e, 0.0);
            s.pts.cnx[k] = opt(world, cx, e, 0.0);
            s.pts.cny[k] = opt(world, cy, e, 0.0);
        }
    }
    Some(PointCols { x: req[0], y: req[1], ox: req[2], oy: req[3], contact: cc, cnx: cx, cny: cy })
}

// the position of an entity index in the gathered points, if it is one
pub fn slot(s: &Scratch, e: u32) -> Option<usize> {
    match s.slot_of.get(e as usize) {
        Some(&k) if k != NONE => Some(k as usize),
        _ => None,
    }
}

// the entity -> position table is cleared again (only the entries gather_points set)
pub fn release(s: &mut Scratch) {
    for &e in &s.pts.ent {
        if let Some(slot) = s.slot_of.get_mut(e as usize) {
            *slot = NONE;
        }
    }
}

// one column to write: its number, the entities, and the value of each
type Job<'a> = (Option<usize>, &'a [u32], &'a dyn Fn(usize) -> f64);

// first every column is touched (refusal writes nothing), then the finite values are written
fn scatter(world: &mut World, jobs: &[Job]) -> Result<(), String> {
    for &(col, _, _) in jobs {
        if let Some(c) = col {
            world.touch_column(c)?;
        }
    }
    for &(col, ents, val) in jobs {
        let Some(c) = col else { continue };
        for (i, &e) in ents.iter().enumerate() {
            let v = val(i);
            if !v.is_finite() {
                continue;
            }
            if let Some(slot) = world.column_mut(c).get_mut(e) {
                *slot = v;
            }
        }
    }
    Ok(())
}

// what: POS (x, y), OLD (old_x, old_y), CONTACT (contact, contact_nx, contact_ny), or-ed
pub fn scatter_points(world: &mut World, s: &Scratch, c: &PointCols, what: u8) -> Result<(), String> {
    let p = &s.pts;
    let (fx, fy, fox, foy) = (|i: usize| p.x[i], |i: usize| p.y[i], |i: usize| p.ox[i], |i: usize| p.oy[i]);
    let (fc, fnx, fny) = (|i: usize| p.contact[i], |i: usize| p.cnx[i], |i: usize| p.cny[i]);
    let pick = |bit: u8, col: Option<usize>| if what & bit != 0 { col } else { None };
    let e = &p.ent[..];
    let jobs: [Job; 7] = [
        (pick(POS, Some(c.x)), e, &fx),
        (pick(POS, Some(c.y)), e, &fy),
        (pick(OLD, Some(c.ox)), e, &fox),
        (pick(OLD, Some(c.oy)), e, &foy),
        (pick(CONTACT, c.contact), e, &fc),
        (pick(CONTACT, c.cnx), e, &fnx),
        (pick(CONTACT, c.cny), e, &fny),
    ];
    scatter(world, &jobs)
}

// ---------- links ----------

fn link_mode(v: f64) -> u8 {
    if !v.is_finite() {
        return 0;
    }
    match v.round() as i64 {
        1 => 1,
        2 => 2,
        _ => 0,
    }
}

// links: node_a, node_b, rest_len required; compliance (0), mode (0, rounded: 1 and 2 as
// they are, anything else 0), lambda (0), force_max (0) optional. needs gather_points first.
// skips links whose ends do not resolve to points, a == b, rest_len not finite or < 0
pub fn gather_links(world: &World, s: &mut Scratch) -> Option<LinkCols> {
    s.links.clear();
    let req = need(world, ["node_a", "node_b", "rest_len"])?;
    let (comp, mode) = (find(world, "compliance"), find(world, "mode"));
    let (lam, fmax) = (find(world, "lambda"), find(world, "force_max"));
    for &e in shortest(world, &req) {
        let Some([va, vb, rest]) = read_all(world, &req, e) else { continue };
        let Some((a, b)) = ends(world, &s.slot_of, va, vb) else { continue };
        if rest < 0.0 {
            continue;
        }
        let k = s.links.add(a as usize, b as usize, rest, clean_nonneg(opt(world, comp, e, 0.0)), link_mode(opt(world, mode, e, 0.0)));
        s.links.ent[k] = e;
        s.links.lambda[k] = 0.0;
        s.links.fmax[k] = opt(world, fmax, e, 0.0);
    }
    Some(LinkCols { lambda: lam, fmax })
}

// writes lambda and force_max (if those columns exist)
pub fn scatter_links(world: &mut World, s: &Scratch, c: &LinkCols) -> Result<(), String> {
    let l = &s.links;
    let (fl, ff) = (|i: usize| l.lambda[i], |i: usize| l.fmax[i]);
    let jobs: [Job; 2] = [(c.lambda, &l.ent, &fl), (c.fmax, &l.ent, &ff)];
    scatter(world, &jobs)
}

// ---------- edges, segments ----------

// edges: node_a, node_b, layer required (layer as bits); friction (0), radius (0), group (0) optional. needs
// gather_points first
pub fn gather_edges(world: &World, s: &mut Scratch) -> Option<EdgeCols> {
    s.edges.clear();
    let req = need(world, ["node_a", "node_b", "layer"])?;
    let (fric, rad, grp) = (find(world, "friction"), find(world, "radius"), find(world, "group"));
    for &e in shortest(world, &req) {
        let Some([va, vb, layer]) = read_all(world, &req, e) else { continue };
        let Some((a, b)) = ends(world, &s.slot_of, va, vb) else { continue };
        let f = opt(world, fric, e, 0.0);
        let k = s.edges.add(a as usize, b as usize, to_bits(layer), if f.is_finite() { f } else { 0.0 });
        s.edges.ent[k] = e;
        s.edges.rad[k] = clean_nonneg(opt(world, rad, e, 0.0));
        s.edges.grp[k] = to_bits(opt(world, grp, e, 0.0));
    }
    Some(EdgeCols {})
}

// segments: x1, y1, x2, y2 required; friction (0), layer (1), radius (0) optional. no points needed
pub fn gather_segs(world: &World, s: &mut Scratch) -> Option<SegCols> {
    s.segs.clear();
    let req = need(world, ["x1", "y1", "x2", "y2"])?;
    let (fric, layer, rad) = (find(world, "friction"), find(world, "layer"), find(world, "radius"));
    for &e in shortest(world, &req) {
        let Some([x1, y1, x2, y2]) = read_all(world, &req, e) else { continue };
        let f = opt(world, fric, e, 0.0);
        let k = s.segs.add(x1, y1, x2, y2, if f.is_finite() { f } else { 0.0 }, to_bits(opt(world, layer, e, 1.0)));
        s.segs.ent[k] = e;
        s.segs.rad[k] = clean_nonneg(opt(world, rad, e, 0.0));
    }
    Some(SegCols {})
}

// ---------- cables ----------

// cables: node_a, node_b, rest_len, catenary_a required; linear_density (0) optional. needs
// gather_points first
pub fn gather_cables(world: &World, s: &mut Scratch) -> Option<CableCols> {
    s.cables.ent.clear();
    s.cables.a.clear();
    s.cables.b.clear();
    s.cables.len.clear();
    s.cables.rho.clear();
    s.cables.shape.clear();
    let req = need(world, ["node_a", "node_b", "rest_len", "catenary_a"])?;
    let rho = find(world, "linear_density");
    for &e in shortest(world, &req) {
        let Some([va, vb, len, _]) = read_all(world, &req, e) else { continue };
        let Some((a, b)) = ends(world, &s.slot_of, va, vb) else { continue };
        if len <= 0.0 {
            continue;
        }
        let cb = &mut s.cables;
        cb.ent.push(e);
        cb.a.push(a);
        cb.b.push(b);
        cb.len.push(len);
        cb.rho.push(clean_nonneg(opt(world, rho, e, 0.0)));
        cb.shape.push(Shape::straight());
    }
    Some(CableCols { a: Some(req[3]), x0: find(world, "catenary_x0"), y0: find(world, "catenary_y0"), sag: find(world, "sag") })
}

// writes catenary_a, catenary_x0, catenary_y0, sag from s.cables.shape (the columns that exist)
pub fn scatter_cables(world: &mut World, s: &Scratch, c: &CableCols) -> Result<(), String> {
    let cb = &s.cables;
    let (fa, fx, fy, fs) = (|i: usize| cb.shape[i].a, |i: usize| cb.shape[i].x0, |i: usize| cb.shape[i].y0, |i: usize| cb.shape[i].sag);
    let jobs: [Job; 4] = [(c.a, &cb.ent, &fa), (c.x0, &cb.ent, &fx), (c.y0, &cb.ent, &fy), (c.sag, &cb.ent, &fs)];
    scatter(world, &jobs)
}

// one cable by entity number (ecs.id), for sample_catenary: its ends (x, y of node_a and
// node_b, read directly from the columns), rest_len, and the shape stored in catenary_a,
// catenary_x0, catenary_y0, sag: Curve when a != 0, Vertical when a == 0 and sag > 0,
// otherwise Straight. None when the entity or an end is gone or a column is missing
pub struct CableRead {
    pub shape: Shape,
    pub ends: [f64; 4],
    pub len: f64,
}

// x and y of the entity a ref value points to
fn end_xy(world: &World, cx: usize, cy: usize, v: f64) -> Option<[f64; 2]> {
    let i = world.index_of(ref_id(v)?)?;
    let xy = [world.column(cx).get(i)?, world.column(cy).get(i)?];
    if xy[0].is_finite() && xy[1].is_finite() {
        Some(xy)
    } else {
        None
    }
}

// the stored shape: a, x0, y0, sag (the last three optional)
fn stored_shape(world: &World, e: u32, a: f64) -> Option<Shape> {
    let get = |name: &str| opt(world, find(world, name), e, 0.0);
    let (x0, y0, sag) = (get("catenary_x0"), get("catenary_y0"), get("sag"));
    if !(x0.is_finite() && y0.is_finite() && sag.is_finite()) {
        return None;
    }
    let kind = if a != 0.0 {
        Kind::Curve
    } else if sag > 0.0 {
        Kind::Vertical
    } else {
        Kind::Straight
    };
    Some(Shape { kind, a, x0, y0, sag })
}

pub fn read_cable(world: &World, id: i64) -> Option<CableRead> {
    if id < 0 {
        return None;
    }
    let e = world.index_of(id)?;
    let req = need(world, ["node_a", "node_b", "rest_len", "catenary_a"])?;
    let [na, nb, len, a] = read_all(world, &req, e)?;
    let (cx, cy) = (find(world, "x")?, find(world, "y")?);
    let (pa, pb) = (end_xy(world, cx, cy, na)?, end_xy(world, cx, cy, nb)?);
    let shape = stored_shape(world, e, a)?;
    Some(CableRead { shape, ends: [pa[0], pa[1], pb[0], pb[1]], len })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::Spec;

    const ALL: [&str; 18] = [
        "x", "y", "old_x", "old_y", "inv_mass", "radius", "layer", "mask", "contact", "contact_nx", "contact_ny", "node_a", "node_b",
        "rest_len", "lambda", "force_max", "mode", "compliance",
    ];

    fn world_with(spec: Spec, names: &[&str]) -> World {
        let mut w = World::with_spec(spec);
        w.current_role = String::from("core");
        for n in names {
            w.set(n, "kind", "num").unwrap();
            w.set(n, "write", "dev").unwrap();
        }
        w.current_role = String::from("dev");
        w
    }

    fn game(names: &[&str]) -> World {
        world_with(Spec::game(), names)
    }

    fn put(w: &mut World, e: &str, c: &str, v: f64) {
        let col = w.col(c).unwrap();
        w.set_num(e, col, v).unwrap();
    }

    fn put_point(w: &mut World, e: &str, x: f64, y: f64, m: f64) {
        for (c, v) in [("x", x), ("y", y), ("old_x", x), ("old_y", y), ("inv_mass", m)] {
            put(w, e, c, v);
        }
    }

    fn val(w: &World, e: &str, c: &str) -> Option<f64> {
        w.num(e, c)
    }

    fn pos_of(w: &World, s: &Scratch, e: &str) -> usize {
        let i = w.index_by_name(e).unwrap();
        s.slot_of[i as usize] as usize
    }

    fn id(w: &World, e: &str) -> f64 {
        w.id(e).unwrap() as f64
    }

    fn link(w: &mut World, e: &str, a: f64, b: f64, rest: f64) {
        put(w, e, "node_a", a);
        put(w, e, "node_b", b);
        put(w, e, "rest_len", rest);
    }

    fn three_points() -> World {
        let mut w = game(&ALL);
        for (e, x) in [("a", 1.0), ("b", 2.0), ("c", 3.0)] {
            put(&mut w, e, "x", x);
        }
        for (e, y) in [("c", 30.0), ("b", 20.0), ("a", 10.0)] {
            put(&mut w, e, "y", y);
        }
        for (e, v) in [("b", 2.5), ("c", 3.5), ("a", 1.5)] {
            put(&mut w, e, "old_x", v);
        }
        for (e, v) in [("a", 11.0), ("c", 31.0), ("b", 21.0)] {
            put(&mut w, e, "old_y", v);
        }
        for (e, v) in [("c", 0.3), ("a", 0.1), ("b", 0.2)] {
            put(&mut w, e, "inv_mass", v);
        }
        w
    }

    #[test]
    fn bits() {
        assert_eq!(to_bits(2.0), 2);
        assert_eq!(to_bits(2.6), 3);
        assert_eq!(to_bits(-1.0), 0);
        assert_eq!(to_bits(f64::NAN), 0);
        assert_eq!(to_bits(f64::INFINITY), 0);
        assert_eq!(to_bits(4294967296.0 + 5.0), 5);
    }

    #[test]
    fn points_with_different_dense_orders() {
        let w = three_points();
        let mut s = Scratch::default();
        assert!(gather_points(&w, &mut s).is_some());
        assert_eq!(s.pts.len(), 3);
        for (e, x, y, ox, oy, m) in [("a", 1.0, 10.0, 1.5, 11.0, 0.1), ("b", 2.0, 20.0, 2.5, 21.0, 0.2), ("c", 3.0, 30.0, 3.5, 31.0, 0.3)] {
            let p = pos_of(&w, &s, e);
            assert_eq!(s.pts.ent[p], w.index_by_name(e).unwrap());
            assert_eq!((s.pts.x[p], s.pts.y[p], s.pts.ox[p], s.pts.oy[p], s.pts.w[p]), (x, y, ox, oy, m));
            assert_eq!((s.pts.r[p], s.pts.layer[p], s.pts.mask[p], s.pts.contact[p]), (0.0, 1, 1, 0.0));
        }
    }

    #[test]
    fn missing_required_value_skips_the_entity() {
        let mut w = three_points();
        put(&mut w, "d", "x", 4.0);
        put(&mut w, "d", "y", 4.0);
        let mut s = Scratch::default();
        gather_points(&w, &mut s).unwrap();
        assert_eq!(s.pts.len(), 3);
        let d = w.index_by_name("d").unwrap() as usize;
        assert!(s.slot_of.get(d).is_none_or(|&p| p == NONE));
    }

    #[test]
    fn inv_mass_and_radius_are_cleaned() {
        let mut w = game(&ALL);
        put_point(&mut w, "neg", 0.0, 0.0, -2.0);
        put_point(&mut w, "zero", 0.0, 0.0, 0.0);
        put_point(&mut w, "ok", 0.0, 0.0, 0.5);
        put(&mut w, "neg", "radius", -1.0);
        put(&mut w, "ok", "radius", 0.25);
        put(&mut w, "ok", "layer", 2.0);
        put(&mut w, "ok", "mask", 6.0);
        let mut s = Scratch::default();
        gather_points(&w, &mut s).unwrap();
        let (n, z, o) = (pos_of(&w, &s, "neg"), pos_of(&w, &s, "zero"), pos_of(&w, &s, "ok"));
        assert_eq!((s.pts.w[n], s.pts.w[z], s.pts.w[o]), (0.0, 0.0, 0.5));
        assert_eq!((s.pts.r[n], s.pts.r[o]), (0.0, 0.25));
        assert_eq!((s.pts.layer[o], s.pts.mask[o]), (2, 6));
    }

    #[test]
    fn empty_and_missing_columns() {
        let w = World::with_spec(Spec::game());
        let mut s = Scratch::default();
        assert!(gather_points(&w, &mut s).is_none());
        assert!(gather_links(&w, &mut s).is_none());
        assert!(gather_edges(&w, &mut s).is_none());
        assert!(gather_segs(&w, &mut s).is_none());
        assert!(gather_cables(&w, &mut s).is_none());
        assert!(read_cable(&w, 0).is_none());
        let w = game(&ALL);
        assert!(gather_points(&w, &mut s).is_some());
        assert!(s.pts.is_empty());
    }

    #[test]
    fn release_clears_the_table() {
        let w = three_points();
        let mut s = Scratch::default();
        gather_points(&w, &mut s).unwrap();
        release(&mut s);
        assert!(s.slot_of.iter().all(|&p| p == NONE));
        gather_points(&w, &mut s).unwrap();
        assert_eq!(s.pts.len(), 3);
        assert!(s.slot_of.iter().filter(|&&p| p != NONE).count() == 3);
    }

    #[test]
    fn links_resolve_through_ids() {
        let mut w = three_points();
        for (e, a, b, r) in [("l1", "a", "b", 1.0), ("l2", "c", "b", 2.0)] {
            let (ia, ib) = (id(&w, a), id(&w, b));
            link(&mut w, e, ia, ib, r);
        }
        put(&mut w, "l2", "mode", 2.0);
        put(&mut w, "l2", "compliance", -1.0);
        put(&mut w, "l1", "mode", 7.0);
        put(&mut w, "l1", "force_max", 4.0);
        let mut s = Scratch::default();
        gather_points(&w, &mut s).unwrap();
        gather_links(&w, &mut s).unwrap();
        assert_eq!(s.links.len(), 2);
        for k in 0..2 {
            let name = w.name_at(s.links.ent[k]).unwrap();
            let (a, b) = if name == "l1" { ("a", "b") } else { ("c", "b") };
            assert_eq!((s.links.a[k] as usize, s.links.b[k] as usize), (pos_of(&w, &s, a), pos_of(&w, &s, b)));
            let (mode, fmax) = if name == "l1" { (0, 4.0) } else { (2, 0.0) };
            assert_eq!((s.links.mode[k], s.links.fmax[k], s.links.comp[k], s.links.lambda[k]), (mode, fmax, 0.0, 0.0));
        }
    }

    #[test]
    fn bad_refs_and_killed_entities_skip_links() {
        let mut w = three_points();
        let (ia, ib) = (id(&w, "a"), id(&w, "b"));
        link(&mut w, "ok", ia, ib, 1.0);
        link(&mut w, "frac", ia + 0.5, ib, 1.0);
        link(&mut w, "neg", -1.0, ib, 1.0);
        link(&mut w, "same", ia, ia, 1.0);
        link(&mut w, "badlen", ia, ib, -1.0);
        link(&mut w, "gone", ia, ib, 1.0);
        link(&mut w, "huge", 1e300, ib, 1.0);
        let (ic, iok) = (id(&w, "c"), id(&w, "ok"));
        link(&mut w, "dead", ic, ib, 1.0);
        link(&mut w, "nopt", iok, ib, 1.0);
        w.kill("c").unwrap();
        let mut s = Scratch::default();
        gather_points(&w, &mut s).unwrap();
        gather_links(&w, &mut s).unwrap();
        let names: Vec<&str> = s.links.ent.iter().map(|&e| w.name_at(e).unwrap()).collect();
        assert!(names.contains(&"ok") && names.contains(&"gone"));
        assert_eq!(names.len(), 2, "{:?}", names);
    }

    #[test]
    fn killed_point_makes_its_links_disappear() {
        let mut w = three_points();
        let (ia, ib) = (id(&w, "a"), id(&w, "b"));
        link(&mut w, "l", ia, ib, 1.0);
        w.kill("b").unwrap();
        let mut s = Scratch::default();
        gather_points(&w, &mut s).unwrap();
        gather_links(&w, &mut s).unwrap();
        assert!(s.links.is_empty());
        // the index is used again by a new point: the old number must not find it
        put_point(&mut w, "n", 5.0, 5.0, 1.0);
        release(&mut s);
        gather_points(&w, &mut s).unwrap();
        gather_links(&w, &mut s).unwrap();
        assert!(s.links.is_empty());
    }

    #[test]
    fn scatter_points_writes_only_the_set() {
        let mut w = three_points();
        put(&mut w, "other", "x", 99.0);
        put(&mut w, "a", "contact", 5.0);
        let mut s = Scratch::default();
        let c = gather_points(&w, &mut s).unwrap();
        let p = pos_of(&w, &s, "a");
        s.pts.x[p] = 100.0;
        s.pts.y[p] = f64::NAN;
        s.pts.ox[p] = 7.0;
        s.pts.contact[p] = 1.0;
        scatter_points(&mut w, &s, &c, POS).unwrap();
        assert_eq!(val(&w, "a", "x"), Some(100.0));
        assert_eq!(val(&w, "a", "y"), Some(10.0), "non-finite is not written");
        assert_eq!(val(&w, "a", "old_x"), Some(1.5));
        assert_eq!(val(&w, "a", "contact"), Some(5.0));
        assert_eq!(val(&w, "other", "x"), Some(99.0));
        assert_eq!(val(&w, "other", "y"), None);
        scatter_points(&mut w, &s, &c, OLD | CONTACT).unwrap();
        assert_eq!(val(&w, "a", "old_x"), Some(7.0));
        assert_eq!(val(&w, "a", "contact"), Some(1.0));
        assert_eq!(val(&w, "b", "contact"), None, "no value is created");
    }

    #[test]
    fn read_only_world_refuses() {
        let mut w = three_points();
        let mut s = Scratch::default();
        let c = gather_points(&w, &mut s).unwrap();
        s.pts.x[0] = -5.0;
        w.read_only = true;
        assert!(scatter_points(&mut w, &s, &c, POS | OLD).is_err());
        w.read_only = false;
        for e in ["a", "b", "c"] {
            assert_ne!(val(&w, e, "x"), Some(-5.0));
        }
    }

    #[test]
    fn role_without_right_refuses_and_a_failed_run_restores() {
        let mut w = world_with(Spec::ledger(), &ALL);
        for e in ["a", "b"] {
            put_point(&mut w, e, 1.0, 1.0, 1.0);
        }
        let mut s = Scratch::default();
        let c = gather_points(&w, &mut s).unwrap();
        s.pts.x.iter_mut().for_each(|v| *v = 8.0);
        let m = w.begin();
        scatter_points(&mut w, &s, &c, POS).unwrap();
        assert_eq!(val(&w, "a", "x"), Some(8.0));
        w.rollback(m, "test");
        assert_eq!(val(&w, "a", "x"), Some(1.0));
        assert_eq!(val(&w, "b", "x"), Some(1.0));
        w.current_role = String::from("clerk");
        assert!(scatter_points(&mut w, &s, &c, POS).unwrap_err().starts_with("no_right"));
        assert_eq!(val(&w, "a", "x"), Some(1.0));
    }

    #[test]
    fn scatter_links_writes_lambda_and_force_max() {
        let mut w = three_points();
        let (ia, ib) = (id(&w, "a"), id(&w, "b"));
        link(&mut w, "l", ia, ib, 1.0);
        put(&mut w, "l", "lambda", 0.5);
        let mut s = Scratch::default();
        gather_points(&w, &mut s).unwrap();
        let c = gather_links(&w, &mut s).unwrap();
        s.links.lambda[0] = -3.0;
        s.links.fmax[0] = 9.0;
        scatter_links(&mut w, &s, &c).unwrap();
        assert_eq!(val(&w, "l", "lambda"), Some(-3.0));
        assert_eq!(val(&w, "l", "force_max"), None, "the column has no value for l before; none is made");
    }

    #[test]
    fn edges_and_segments() {
        let mut w = game(&["x", "y", "old_x", "old_y", "inv_mass", "node_a", "node_b", "layer", "friction", "x1", "y1", "x2", "y2"]);
        put_point(&mut w, "p", 0.0, 0.0, 1.0);
        put_point(&mut w, "q", 1.0, 0.0, 1.0);
        let (ip, iq) = (id(&w, "p"), id(&w, "q"));
        put(&mut w, "e", "node_a", ip);
        put(&mut w, "e", "node_b", iq);
        put(&mut w, "e", "layer", 4.0);
        put(&mut w, "e", "friction", 0.5);
        for (c, v) in [("x1", 0.0), ("y1", 1.0), ("x2", 2.0), ("y2", 1.0)] {
            put(&mut w, "sg", c, v);
        }
        let mut s = Scratch::default();
        gather_points(&w, &mut s).unwrap();
        gather_edges(&w, &mut s).unwrap();
        gather_segs(&w, &mut s).unwrap();
        assert_eq!(s.edges.len(), 1);
        assert_eq!((s.edges.layer[0], s.edges.fric[0]), (4, 0.5));
        assert_eq!(s.segs.len(), 1);
        assert_eq!((s.segs.fric[0], s.segs.layer[0], s.segs.x2[0]), (0.0, 1, 2.0));
    }

    #[test]
    fn cables_gather_and_scatter() {
        let mut names = ALL.to_vec();
        names.extend(["catenary_a", "catenary_x0", "catenary_y0", "sag", "linear_density"]);
        let mut w = game(&names);
        put_point(&mut w, "p", 0.0, 0.0, 1.0);
        put_point(&mut w, "q", 4.0, 0.0, 1.0);
        let (ip, iq) = (id(&w, "p"), id(&w, "q"));
        link(&mut w, "c1", ip, iq, 5.0);
        put(&mut w, "c1", "catenary_a", 0.0);
        put(&mut w, "c1", "linear_density", -2.0);
        link(&mut w, "c0", ip, iq, 0.0);
        put(&mut w, "c0", "catenary_a", 0.0);
        let mut s = Scratch::default();
        gather_points(&w, &mut s).unwrap();
        let c = gather_cables(&w, &mut s).unwrap();
        assert_eq!(s.cables.ent.len(), 1, "a zero length is skipped");
        assert_eq!((s.cables.len[0], s.cables.rho[0]), (5.0, 0.0));
        s.cables.shape[0] = Shape { kind: Kind::Curve, a: 2.0, x0: 1.0, y0: -1.0, sag: 0.5 };
        scatter_cables(&mut w, &s, &c).unwrap();
        assert_eq!(val(&w, "c1", "catenary_a"), Some(2.0));
        assert_eq!(val(&w, "c1", "catenary_x0"), None, "no value is created");
        assert_eq!(val(&w, "c0", "catenary_a"), Some(0.0));
    }

    fn cable_world() -> World {
        let mut names = ALL.to_vec();
        names.extend(["catenary_a", "catenary_x0", "catenary_y0", "sag"]);
        let mut w = game(&names);
        put_point(&mut w, "p", 0.0, 1.0, 1.0);
        put_point(&mut w, "q", 4.0, 2.0, 1.0);
        let (ip, iq) = (id(&w, "p"), id(&w, "q"));
        for e in ["curve", "vert", "line"] {
            link(&mut w, e, ip, iq, 5.0);
        }
        put(&mut w, "curve", "catenary_a", -3.0);
        put(&mut w, "curve", "catenary_x0", 2.0);
        put(&mut w, "curve", "catenary_y0", 7.0);
        put(&mut w, "vert", "catenary_a", 0.0);
        put(&mut w, "vert", "sag", 1.5);
        put(&mut w, "line", "catenary_a", 0.0);
        w
    }

    #[test]
    fn read_cable_kinds() {
        let w = cable_world();
        let r = read_cable(&w, w.id("curve").unwrap()).unwrap();
        assert_eq!(r.shape.kind, Kind::Curve);
        assert_eq!((r.shape.a, r.shape.x0, r.shape.y0, r.len), (-3.0, 2.0, 7.0, 5.0));
        assert_eq!(r.ends, [0.0, 1.0, 4.0, 2.0]);
        let r = read_cable(&w, w.id("vert").unwrap()).unwrap();
        assert_eq!((r.shape.kind, r.shape.sag), (Kind::Vertical, 1.5));
        let r = read_cable(&w, w.id("line").unwrap()).unwrap();
        assert_eq!(r.shape.kind, Kind::Straight);
        assert!(read_cable(&w, -4).is_none());
        assert!(read_cable(&w, w.id("p").unwrap()).is_none(), "not a cable");
    }

    #[test]
    fn read_cable_with_a_gone_end() {
        let mut w = cable_world();
        let id_curve = w.id("curve").unwrap();
        w.kill("q").unwrap();
        assert!(read_cable(&w, id_curve).is_none());
    }
}
