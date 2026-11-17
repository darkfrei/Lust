// project_segments and project_edges: hard pushes of points out of barriers (spec 4.3, 4.4)
use super::data::{Edges, Points, Segs};

type V2 = (f64, f64);

const EPS_LEN: f64 = 1e-12;
const EPS_DIST: f64 = 1e-9;

fn sub(a: V2, b: V2) -> V2 {
    (a.0 - b.0, a.1 - b.1)
}

fn dot(a: V2, b: V2) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

fn finite2(a: V2) -> bool {
    a.0.is_finite() && a.1.is_finite()
}

// friction clamped to 0..1, a broken number counts as 0
fn clamp_mu(mu: f64) -> f64 {
    if mu.is_finite() {
        mu.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

// a point takes part only with a finite w > 0 and a finite r > 0
fn active(p: &Points, i: usize) -> bool {
    let (w, r) = (p.w[i], p.r[i]);
    w.is_finite() && w > 0.0 && r.is_finite() && r > 0.0
}

fn pos(p: &Points, i: usize) -> V2 {
    (p.x[i], p.y[i])
}

fn old(p: &Points, i: usize) -> V2 {
    (p.ox[i], p.oy[i])
}

// the speed of the rim of a turning circle where it touches along normal n (the rim point is
// at -n * r from the centre): roll * r * (n.y, -n.x)
fn rim(roll: f64, r: f64, n: V2) -> V2 {
    if roll == 0.0 || !roll.is_finite() {
        return (0.0, 0.0);
    }
    (roll * r * n.1, -roll * r * n.0)
}

// 1 / moment of inertia of a turning point, 0 when its turning is not changed by friction
fn inv_inertia(p: &Points, i: usize) -> f64 {
    let k = p.inertia[i];
    if k.is_finite() && k > 0.0 {
        1.0 / k
    } else {
        0.0
    }
}

// friction by coulomb: the impulse along the surface that stops the sliding, but never more than
// mu times the impulse along the normal (jn). w_t is the inverse mass along the surface (with
// the turning of the rim when the point has an inertia). gives the impulse (mass * speed)
fn friction(slip: V2, jn: f64, w_t: f64, mu: f64) -> V2 {
    if w_t.is_nan() || w_t <= 0.0 || jn.is_nan() || jn <= 0.0 || mu <= 0.0 {
        return (0.0, 0.0);
    }
    let full = (-slip.0 / w_t, -slip.1 / w_t);
    let len = (full.0 * full.0 + full.1 * full.1).sqrt();
    let max = mu * jn;
    if len <= max || len == 0.0 {
        full
    } else {
        (full.0 * max / len, full.1 * max / len)
    }
}

// a friction impulse jt at the rim (at -n * r from the centre) turns a point with an inertia
fn turn_by(p: &mut Points, i: usize, n: V2, jt: V2) {
    let ii = inv_inertia(p, i);
    if ii == 0.0 {
        return;
    }
    let r = p.r[i];
    let d = (-n.0 * r, -n.1 * r);
    let dr = (d.0 * jt.1 - d.1 * jt.0) * ii;
    if dr.is_finite() {
        p.roll[i] += dr;
    }
}

// ---------------------------------------------------------------- segments

struct Seg {
    p1: V2,
    e: V2,
    len: f64,
    sn: V2,
    min: V2,
    max: V2,
    mu: f64,
    layer: u32,
}

fn seg_at(s: &Segs, j: usize) -> Option<Seg> {
    let p1 = (s.x1[j], s.y1[j]);
    let p2 = (s.x2[j], s.y2[j]);
    if !finite2(p1) || !finite2(p2) {
        return None;
    }
    let e = sub(p2, p1);
    let len = (e.0 * e.0 + e.1 * e.1).sqrt();
    let sn = if len < EPS_LEN { (0.0, -1.0) } else { (-e.1 / len, e.0 / len) };
    Some(Seg {
        p1,
        e,
        len,
        sn,
        min: (p1.0.min(p2.0), p1.1.min(p2.1)),
        max: (p1.0.max(p2.0), p1.1.max(p2.1)),
        mu: clamp_mu(s.fric[j]),
        layer: s.layer[j],
    })
}

// box of the old and new position grown by r does not touch the box of the segment
fn seg_box_miss(g: &Seg, pn: V2, po: V2, r: f64) -> bool {
    let lo = (pn.0.min(po.0) - r, pn.1.min(po.1) - r);
    let hi = (pn.0.max(po.0) + r, pn.1.max(po.1) + r);
    hi.0 < g.min.0 || lo.0 > g.max.0 || hi.1 < g.min.1 || lo.1 > g.max.1
}

// the point went through the barrier between old and new: returns the push (new pos, normal)
fn seg_crossing(g: &Seg, pn: V2, po: V2, r: f64, so: f64, sp: f64) -> Option<(V2, V2)> {
    if g.len < EPS_LEN || so * sp >= 0.0 {
        return None;
    }
    let s = so / (so - sp);
    let x = (po.0 + s * (pn.0 - po.0), po.1 + s * (pn.1 - po.1));
    let u = dot(sub(x, g.p1), g.e) / (g.len * g.len);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let n = if so > 0.0 { g.sn } else { (-g.sn.0, -g.sn.1) };
    let tf = dot(sub(pn, g.p1), g.e) / (g.len * g.len);
    let q = if tf > 0.0 && tf < 1.0 {
        (g.p1.0 + tf * g.e.0, g.p1.1 + tf * g.e.1)
    } else {
        x
    };
    Some(((q.0 + n.0 * r, q.1 + n.1 * r), n))
}

// closest point of the segment to pn (t clamped to 0..1)
fn seg_closest(g: &Seg, pn: V2) -> V2 {
    if g.len < EPS_LEN {
        return g.p1;
    }
    let t = (dot(sub(pn, g.p1), g.e) / (g.len * g.len)).clamp(0.0, 1.0);
    (g.p1.0 + t * g.e.0, g.p1.1 + t * g.e.1)
}

// the push when the point is closer than r to the segment (no crossing)
fn seg_overlap(g: &Seg, pn: V2, r: f64, so: f64, sp: f64) -> Option<(V2, V2)> {
    let q = seg_closest(g, pn);
    let delta = sub(pn, q);
    let d2 = dot(delta, delta);
    if d2 >= r * r {
        return None;
    }
    let d = d2.sqrt();
    let n = if d >= EPS_DIST {
        (delta.0 / d, delta.1 / d)
    } else {
        let side = if so != 0.0 { so } else { sp };
        if side < 0.0 {
            (-g.sn.0, -g.sn.1)
        } else {
            g.sn
        }
    };
    Some(((q.0 + n.0 * r, q.1 + n.1 * r), n))
}

// the push of point i by segment g, if any
fn seg_push(g: &Seg, pn: V2, po: V2, r: f64) -> Option<(V2, V2)> {
    let so = dot(sub(po, g.p1), g.sn);
    let sp = dot(sub(pn, g.p1), g.sn);
    seg_crossing(g, pn, po, r, so, sp).or_else(|| seg_overlap(g, pn, r, so, sp))
}

// writes the result of a push of point i; false (and nothing written) when not finite. on an
// inward hit (vn < 0) the speed of approach is taken away and friction (coulomb, mu) acts on the
// sliding of the rim; else the speed is kept
fn apply_point_push(p: &mut Points, i: usize, np: V2, n: V2, mu: f64) -> bool {
    if !finite2(np) || !finite2(n) {
        return false;
    }
    let v = sub(pos(p, i), old(p, i));
    let vn = dot(v, n);
    let wp = p.w[i];
    let mut nv = v;
    let mut jt = (0.0, 0.0);
    if vn < 0.0 && wp > 0.0 {
        let vt = (v.0 - vn * n.0, v.1 - vn * n.1);
        let rv = rim(p.roll[i], p.r[i], n);
        let slip = (vt.0 + rv.0, vt.1 + rv.1);
        let w_t = wp + p.r[i] * p.r[i] * inv_inertia(p, i);
        jt = friction(slip, -vn / wp, w_t, mu);
        nv = (vt.0 + wp * jt.0, vt.1 + wp * jt.1);
    }
    let no = (np.0 - nv.0, np.1 - nv.1);
    if !finite2(no) {
        return false;
    }
    p.x[i] = np.0;
    p.y[i] = np.1;
    p.ox[i] = no.0;
    p.oy[i] = no.1;
    turn_by(p, i, n, jt);
    p.contact[i] = 1.0;
    p.cnx[i] = n.0;
    p.cny[i] = n.1;
    true
}

fn segments_for_point(p: &mut Points, s: &Segs, i: usize) -> usize {
    let mut pushes = 0;
    for j in 0..s.len() {
        if p.mask[i] & s.layer[j] == 0 {
            continue;
        }
        let (pn, po) = (pos(p, i), old(p, i));
        if !finite2(pn) || !finite2(po) {
            return pushes;
        }
        // a capsule: the point keeps its own radius plus the thickness of the segment away
        let r = p.r[i] + s.rad[j];
        let Some(g) = seg_at(s, j) else { continue };
        if g.layer == 0 || seg_box_miss(&g, pn, po, r) {
            continue;
        }
        if let Some((np, n)) = seg_push(&g, pn, po, r) {
            if apply_point_push(p, i, np, n, g.mu) {
                pushes += 1;
            }
        }
    }
    pushes
}

// a point (w > 0, r > 0) against every static segment whose layer & the point's mask != 0.
// sets x y ox oy contact cnx cny of the points that were pushed. `iterations` passes.
// returns the number of pushes done
pub fn project_segments(p: &mut Points, s: &Segs, iterations: usize) -> usize {
    // a point whose mask meets no layer of any segment is skipped at once (most joints)
    let any = s.layer.iter().fold(0u32, |m, &l| m | l);
    let mut pushes = 0;
    for _ in 0..iterations {
        for i in 0..p.len() {
            if p.mask[i] & any != 0 && active(p, i) {
                pushes += segments_for_point(p, s, i);
            }
        }
    }
    pushes
}

// ---------------------------------------------------------------- edges

struct EdgeHit {
    a: usize,
    b: usize,
    // the distance to keep: radius of the point plus the thickness of the edge
    r: f64,
    t: f64,
    n: V2,
    d: f64,
}

fn edge_ends(p: &Points, e: &Edges, j: usize) -> Option<(usize, usize)> {
    let (a, b) = (e.a[j] as usize, e.b[j] as usize);
    if a >= p.len() || b >= p.len() {
        None
    } else {
        Some((a, b))
    }
}

fn edge_box_miss(pn: V2, pa: V2, pb: V2, r: f64) -> bool {
    pn.0 < pa.0.min(pb.0) - r
        || pn.0 > pa.0.max(pb.0) + r
        || pn.1 < pa.1.min(pb.1) - r
        || pn.1 > pa.1.max(pb.1) + r
}

// the side of the edge a-b that point i came from: sign (+1 or -1) against the OLD
// configuration of the edge (old ends), and the current perpendicular (-ey, ex)/len
fn edge_side(p: &Points, i: usize, a: usize, b: usize, perp: V2) -> f64 {
    let (oa, ob) = (old(p, a), old(p, b));
    let oab = sub(ob, oa);
    let ol = (oab.0 * oab.0 + oab.1 * oab.1).sqrt();
    let (n_old, base) = if ol < EPS_DIST || !ol.is_finite() {
        (perp, pos(p, a))
    } else {
        ((-oab.1 / ol, oab.0 / ol), oa)
    };
    if dot(sub(old(p, i), base), n_old) >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

// where the point sits against the edge now: unclamped parameter tu and the signed distance
// sp to the current line along the perpendicular
struct EdgeGeom {
    pa: V2,
    ab: V2,
    perp: V2,
    tu: f64,
    sp: f64,
    sgn: f64,
}

fn edge_geom(p: &Points, i: usize, a: usize, b: usize) -> Option<EdgeGeom> {
    let (pa, pb, pn) = (pos(p, a), pos(p, b), pos(p, i));
    let ab = sub(pb, pa);
    let l2 = dot(ab, ab);
    if l2.is_nan() || l2 < 1e-18 || !l2.is_finite() || !finite2(pn) {
        return None;
    }
    let len = l2.sqrt();
    let perp = (-ab.1 / len, ab.0 / len);
    let rel = sub(pn, pa);
    Some(EdgeGeom {
        pa,
        ab,
        perp,
        tu: dot(rel, ab) / l2,
        sp: dot(rel, perp),
        sgn: edge_side(p, i, a, b, perp),
    })
}

// the point crossed the current line from its own side, inside the edge: foot point push
fn edge_crossed(g: &EdgeGeom, a: usize, b: usize, r: f64) -> Option<EdgeHit> {
    if g.sp * g.sgn >= 0.0 || !(0.0..=1.0).contains(&g.tu) {
        return None;
    }
    let n = (g.perp.0 * g.sgn, g.perp.1 * g.sgn);
    Some(EdgeHit { a, b, r, t: g.tu, n, d: g.sp * g.sgn })
}

// closer than r to the edge: normal is the own-side normal inside the edge, delta/d at ends
fn edge_touch(p: &Points, i: usize, g: &EdgeGeom, a: usize, b: usize, r: f64) -> Option<EdgeHit> {
    let pn = pos(p, i);
    let pb = (g.pa.0 + g.ab.0, g.pa.1 + g.ab.1);
    if edge_box_miss(pn, g.pa, pb, r) {
        return None;
    }
    let t = g.tu.clamp(0.0, 1.0);
    let q = (g.pa.0 + t * g.ab.0, g.pa.1 + t * g.ab.1);
    let delta = sub(pn, q);
    let d2 = dot(delta, delta);
    if d2 >= r * r {
        return None;
    }
    let side = (g.perp.0 * g.sgn, g.perp.1 * g.sgn);
    let d = d2.sqrt();
    if g.tu > 0.0 && g.tu < 1.0 {
        return Some(EdgeHit { a, b, r, t, n: side, d: g.sp * g.sgn });
    }
    let n = if d >= EPS_DIST { (delta.0 / d, delta.1 / d) } else { side };
    Some(EdgeHit { a, b, r, t, n, d })
}

// the push of point i by the edge a-b, if any
fn edge_overlap(p: &Points, i: usize, a: usize, b: usize, rad: f64) -> Option<EdgeHit> {
    let g = edge_geom(p, i, a, b)?;
    let r = p.r[i] + rad;
    edge_crossed(&g, a, b, r).or_else(|| edge_touch(p, i, &g, a, b, r))
}

// relative velocity of the point against the edge point q, before the push
fn edge_rel_velocity(p: &Points, i: usize, h: &EdgeHit) -> (V2, V2) {
    let vp = sub(pos(p, i), old(p, i));
    let va = sub(pos(p, h.a), old(p, h.a));
    let vb = sub(pos(p, h.b), old(p, h.b));
    let vq = ((1.0 - h.t) * va.0 + h.t * vb.0, (1.0 - h.t) * va.1 + h.t * vb.1);
    (vp, vq)
}

// pushes point i and both ends. the side comes from the old configuration of the edge and a
// crossing of the current line inside the edge is caught (edge_crossed); a point that skips
// past the ends of the edge in one step is not caught (no guard there)
fn edge_push(p: &mut Points, i: usize, h: &EdgeHit, mu: f64) -> bool {
    let (wp, wa, wb) = (p.w[i], p.w[h.a], p.w[h.b]);
    let w_sum = wp + (1.0 - h.t) * (1.0 - h.t) * wa + h.t * h.t * wb;
    if !w_sum.is_finite() || w_sum <= 0.0 {
        return false;
    }
    let s = (h.r - h.d) / w_sum;
    // the speeds before the push: of the point, of both ends, of the edge point q
    let (vp, vq) = edge_rel_velocity(p, i, h);
    let va = sub(pos(p, h.a), old(p, h.a));
    let vb = sub(pos(p, h.b), old(p, h.b));
    // 1. the overlap is taken out by moving the positions; the speeds stay (old moves along), so
    //    a push out of an overlap gives no speed and no energy to anything
    let (ka, kb) = (wa * (1.0 - h.t), wb * h.t);
    let np = (p.x[i] + wp * s * h.n.0, p.y[i] + wp * s * h.n.1);
    let na = (p.x[h.a] - ka * s * h.n.0, p.y[h.a] - ka * s * h.n.1);
    let nb = (p.x[h.b] - kb * s * h.n.0, p.y[h.b] - kb * s * h.n.1);
    // 2. the speed: when the point and the edge come closer, one impulse (the same effective mass
    //    w_sum) takes the speed of approach away from both, and friction (coulomb, mu) acts on the
    //    sliding. the momentum is kept; a point with an inertia also turns by the friction
    let rel = sub(vp, vq);
    let rel_n = dot(rel, h.n);
    let (mut nvp, mut nva, mut nvb) = (vp, va, vb);
    let mut jt = (0.0, 0.0);
    if rel_n < 0.0 {
        // the sliding: the speed along the edge plus the speed of the rim of a turning point
        let rv = rim(p.roll[i], p.r[i], h.n);
        let rt = (rel.0 - rel_n * h.n.0 + rv.0, rel.1 - rel_n * h.n.1 + rv.1);
        // the impulse (mass * speed): along the normal it stops the approach, along the edge
        // friction by coulomb (at most mu times the normal impulse)
        let jn = -rel_n / w_sum;
        let w_t = w_sum + p.r[i] * p.r[i] * inv_inertia(p, i);
        jt = friction(rt, jn, w_t, mu);
        let j = (jn * h.n.0 + jt.0, jn * h.n.1 + jt.1);
        nvp = (vp.0 + wp * j.0, vp.1 + wp * j.1);
        nva = (va.0 - ka * j.0, va.1 - ka * j.1);
        nvb = (vb.0 - kb * j.0, vb.1 - kb * j.1);
    }
    let no = (np.0 - nvp.0, np.1 - nvp.1);
    let noa = (na.0 - nva.0, na.1 - nva.1);
    let nob = (nb.0 - nvb.0, nb.1 - nvb.1);
    if !(finite2(np) && finite2(na) && finite2(nb) && finite2(no) && finite2(noa) && finite2(nob)) {
        return false;
    }
    (p.x[i], p.y[i]) = np;
    (p.x[h.a], p.y[h.a]) = na;
    (p.x[h.b], p.y[h.b]) = nb;
    (p.ox[i], p.oy[i]) = no;
    (p.ox[h.a], p.oy[h.a]) = noa;
    (p.ox[h.b], p.oy[h.b]) = nob;
    turn_by(p, i, h.n, jt);
    p.contact[i] = 1.0;
    p.cnx[i] = h.n.0;
    p.cny[i] = h.n.1;
    true
}

fn edges_for_point(p: &mut Points, e: &Edges, i: usize) -> usize {
    let mut pushes = 0;
    for j in 0..e.len() {
        if p.mask[i] & e.layer[j] == 0 {
            continue;
        }
        let Some((a, b)) = edge_ends(p, e, j) else { continue };
        if a == i || b == i {
            continue;
        }
        // the vertices and the sides of one shape (the same non-zero group) pass each other
        if p.grp[i] != 0 && p.grp[i] == e.grp[j] {
            continue;
        }
        let Some(h) = edge_overlap(p, i, a, b, e.rad[j]) else { continue };
        if edge_push(p, i, &h, clamp_mu(e.fric[j])) {
            pushes += 1;
        }
    }
    pushes
}

// a point (w > 0, r > 0) against every edge (movable barrier) whose layer & mask != 0 and
// whose ends are not the point itself. moves the point and both ends of the edge. returns the
// number of pushes done
pub fn project_edges(p: &mut Points, e: &Edges, iterations: usize) -> usize {
    // a point whose mask meets no layer of any edge is skipped at once (most joints)
    let any = e.layer.iter().fold(0u32, |m, &l| m | l);
    let mut pushes = 0;
    for _ in 0..iterations {
        for i in 0..p.len() {
            if p.mask[i] & any != 0 && active(p, i) && finite2(old(p, i)) {
                pushes += edges_for_point(p, e, i);
            }
        }
    }
    pushes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor() -> Segs {
        let mut s = Segs::default();
        s.add(-1000.0, 0.0, 1000.0, 0.0, 0.0, 1);
        s
    }

    fn ball(p: &mut Points, x: f64, y: f64, ox: f64, oy: f64) -> usize {
        let i = p.add(x, y, 1.0, 10.0);
        p.ox[i] = ox;
        p.oy[i] = oy;
        i
    }

    fn verlet(p: &mut Points, i: usize, gy: f64) {
        let (x, y) = (p.x[i], p.y[i]);
        p.x[i] = 2.0 * x - p.ox[i];
        p.y[i] = 2.0 * y - p.oy[i] + gy;
        p.ox[i] = x;
        p.oy[i] = y;
    }

    // a resting point pressed into a resting free edge: the push separates them and gives no
    // speed to anything (a push out of an overlap is not a bounce), so no energy comes from it
    #[test]
    fn an_edge_pushed_by_a_resting_point_does_not_fly_away() {
        let mut p = Points::default();
        let a = p.add(-10.0, 0.0, 1.0, 0.0);
        let b = p.add(10.0, 0.0, 1.0, 0.0);
        let c = p.add(0.0, 3.0, 0.02, 5.0);
        p.mask[c] = 4;
        let mut e = Edges::default();
        e.add(a, b, 4, 0.5);
        assert_eq!(project_edges(&mut p, &e, 1), 1);
        for i in [a, b, c] {
            let (vx, vy) = (p.x[i] - p.ox[i], p.y[i] - p.oy[i]);
            assert!(vx.abs() < 1e-12 && vy.abs() < 1e-12, "point {} got speed {} {}", i, vx, vy);
        }
        assert!(p.y[c] - p.y[a] >= 5.0 - 1e-9, "not pushed apart");
    }

    // an edge moving into a resting point: after the hit they move together along the normal
    // (no bounce) and the momentum along the normal is kept
    #[test]
    fn an_edge_hitting_a_point_keeps_momentum_and_does_not_bounce() {
        let mut p = Points::default();
        let a = p.add(-10.0, 0.0, 1.0, 0.0);
        let b = p.add(10.0, 0.0, 1.0, 0.0);
        p.oy[a] = -1.0;
        p.oy[b] = -1.0;
        let c = p.add(0.0, 4.5, 0.5, 5.0);
        p.mask[c] = 4;
        let mut e = Edges::default();
        e.add(a, b, 4, 0.0);
        let mom = |p: &Points| (0..3).map(|i| (p.y[i] - p.oy[i]) / p.w[i]).sum::<f64>();
        let before = mom(&p);
        assert_eq!(project_edges(&mut p, &e, 1), 1);
        assert!((mom(&p) - before).abs() < 1e-9, "{} {}", mom(&p), before);
        let (va, vc) = (p.y[a] - p.oy[a], p.y[c] - p.oy[c]);
        assert!((va - vc).abs() < 1e-9, "the edge bounces: {} {}", va, vc);
    }

    // a turning circle on a segment with friction drives itself until it rolls: its speed plus
    // the speed of its rim at the contact goes to zero
    #[test]
    fn a_turning_circle_rolls_along_a_segment() {
        let mut s = Segs::default();
        s.add(-1000.0, 0.0, 1000.0, 0.0, 0.5, 1);
        let mut p = Points::default();
        let i = p.add(0.0, 10.0, 1.0, 10.0);
        p.roll[i] = 0.01;
        for _ in 0..200 {
            verlet(&mut p, i, -0.05);
            project_segments(&mut p, &s, 1);
        }
        let vx = p.x[i] - p.ox[i];
        // the normal is (0, 1): rim speed roll * r * (n.y, -n.x) = (0.1, 0); rolling: v + rim = 0
        assert!((vx + 0.1).abs() < 1e-6, "{}", vx);
    }

    // a turning point pressed on a free edge drives the edge the other way, momentum kept
    #[test]
    fn a_turning_point_drives_an_edge_and_keeps_momentum() {
        let mut p = Points::default();
        let a = p.add(-10.0, 0.0, 1.0, 0.0);
        let b = p.add(10.0, 0.0, 1.0, 0.0);
        let c = p.add(0.0, 4.5, 0.5, 5.0);
        p.mask[c] = 4;
        p.oy[c] = 5.0;
        p.roll[c] = 0.1;
        let mut e = Edges::default();
        e.add(a, b, 4, 0.5);
        let mom = |p: &Points| (0..3).map(|i| (p.x[i] - p.ox[i]) / p.w[i]).sum::<f64>();
        assert_eq!(project_edges(&mut p, &e, 1), 1);
        assert!(mom(&p).abs() < 1e-9, "{}", mom(&p));
        let (vc, va) = (p.x[c] - p.ox[c], p.x[a] - p.ox[a]);
        assert!(vc * va < 0.0 && vc.abs() > 1e-6, "{} {}", vc, va);
    }

    fn ground(mu: f64) -> Segs {
        let mut s = Segs::default();
        s.add(-1000.0, 0.0, 1000.0, 0.0, mu, 1);
        s
    }

    // coulomb: on ground the speed along it falls by at most mu * (the speed of approach) per
    // step: speed 1, approach 0.05; ice (0.1) takes 0.005, rubber (1) takes 0.05
    #[test]
    fn friction_is_at_most_mu_times_the_push() {
        for mu in [0.1, 1.0] {
            let s = ground(mu);
            let mut p = Points::default();
            let i = p.add(0.0, 10.0, 1.0, 10.0);
            p.ox[i] = -1.0;
            verlet(&mut p, i, -0.05);
            project_segments(&mut p, &s, 1);
            let vx = p.x[i] - p.ox[i];
            assert!((vx - (1.0 - mu * 0.05)).abs() < 1e-9, "mu {}: {}", mu, vx);
        }
    }

    // a wheel with an inertia, turning on the spot: on ice it spins and hardly moves, on rubber
    // the friction spins it down and drives it until it rolls (speed + rim speed = 0)
    #[test]
    fn a_wheel_with_inertia_spins_on_ice_and_rolls_on_rubber() {
        let run = |mu: f64| {
            let s = ground(mu);
            let mut p = Points::default();
            let i = p.add(0.0, 10.0, 1.0, 10.0);
            p.roll[i] = 0.1;
            // a ring: inertia = mass * r^2 = 100
            p.inertia[i] = 100.0;
            for _ in 0..400 {
                verlet(&mut p, i, -0.05);
                project_segments(&mut p, &s, 1);
            }
            (p.x[i] - p.ox[i], p.roll[i])
        };
        let (v_ice, roll_ice) = run(0.01);
        let (v_rub, roll_rub) = run(1.0);
        // rubber: rolling, and a ring keeps half of its turning (the rest went into the motion)
        assert!((v_rub + roll_rub * 10.0).abs() < 1e-9, "not rolling: {} {}", v_rub, roll_rub);
        assert!((roll_rub - 0.05).abs() < 1e-6, "{}", roll_rub);
        // ice: still slipping after 400 steps, the wheel turns faster than it rolls
        assert!(roll_ice > 0.08 && (v_ice + roll_ice * 10.0).abs() > 0.1, "{} {}", v_ice, roll_ice);
    }

    // friction turns a wheel with an inertia that slides: it starts to roll by itself
    #[test]
    fn a_sliding_wheel_with_inertia_starts_to_roll() {
        let s = ground(1.0);
        let mut p = Points::default();
        let i = p.add(0.0, 10.0, 1.0, 10.0);
        p.ox[i] = -1.0;
        p.inertia[i] = 50.0;
        for _ in 0..200 {
            verlet(&mut p, i, -0.05);
            project_segments(&mut p, &s, 1);
        }
        let v = p.x[i] - p.ox[i];
        assert!((v + p.roll[i] * 10.0).abs() < 1e-9 && v > 0.5, "{} {}", v, p.roll[i]);
    }

    #[test]
    fn falling_circle_rests() {
        let (s, mut p) = (floor(), Points::default());
        let i = p.add(0.0, 100.0, 1.0, 10.0);
        for _ in 0..300 {
            verlet(&mut p, i, -0.5);
            project_segments(&mut p, &s, 1);
            assert!(p.y[i] >= 10.0 - 1e-9);
        }
        assert_eq!(p.contact[i], 1.0);
        assert!(p.cny[i] > 0.99 && p.cnx[i].abs() < 1e-9);
        assert!((p.y[i] - 10.0).abs() < 1e-6);
    }

    #[test]
    fn fast_circle_does_not_tunnel() {
        let (s, mut p) = (floor(), Points::default());
        let i = ball(&mut p, 0.0, -30.0, 0.0, 20.0);
        assert_eq!(project_segments(&mut p, &s, 1), 1);
        assert!((p.y[i] - 10.0).abs() < 1e-9);
    }

    #[test]
    fn from_below_is_pushed_down() {
        let (s, mut p) = (floor(), Points::default());
        let i = ball(&mut p, 0.0, 30.0, 0.0, -20.0);
        project_segments(&mut p, &s, 1);
        assert!((p.y[i] + 10.0).abs() < 1e-9 && p.cny[i] < -0.99);
        let j = ball(&mut p, 0.0, -4.0, 0.0, -9.0);
        project_segments(&mut p, &s, 1);
        assert!((p.y[j] + 10.0).abs() < 1e-9);
    }

    #[test]
    fn layer_mask() {
        let s = floor();
        let mut p = Points::default();
        let i = ball(&mut p, 0.0, -30.0, 0.0, 20.0);
        p.mask[i] = 2;
        assert_eq!(project_segments(&mut p, &s, 1), 0);
        assert_eq!(p.y[i], -30.0);
        p.mask[i] = 3;
        assert_eq!(project_segments(&mut p, &s, 1), 1);
        assert!(p.y[i] > 0.0);
    }

    #[test]
    fn friction() {
        for (mu, tangent) in [(1.0, 0.0), (0.0, 5.0)] {
            let mut s = Segs::default();
            s.add(-1000.0, 0.0, 1000.0, 0.0, mu, 1);
            let mut p = Points::default();
            let i = ball(&mut p, 5.0, 5.0, 0.0, 15.0);
            project_segments(&mut p, &s, 1);
            assert!(((p.x[i] - p.ox[i]) - tangent).abs() < 1e-9);
            assert!((p.y[i] - 10.0).abs() < 1e-9);
        }
    }

    #[test]
    fn outward_speed_kept() {
        let mut s = Segs::default();
        s.add(-1000.0, 0.0, 1000.0, 0.0, 1.0, 1);
        let mut p = Points::default();
        let i = ball(&mut p, 5.0, 5.0, 5.0, 2.0);
        project_segments(&mut p, &s, 1);
        assert!(((p.y[i] - p.oy[i]) - 3.0).abs() < 1e-9);
        assert!(p.x[i] == p.ox[i]);
    }

    #[test]
    fn inactive_points() {
        let s = floor();
        let mut p = Points::default();
        let i = p.add(0.0, 3.0, 0.0, 10.0);
        let j = p.add(0.0, 3.0, 1.0, 0.0);
        assert_eq!(project_segments(&mut p, &s, 3), 0);
        assert_eq!((p.y[i], p.y[j]), (3.0, 3.0));
    }

    #[test]
    fn zero_length_segment_is_a_point() {
        let mut s = Segs::default();
        s.add(0.0, 0.0, 0.0, 0.0, 0.0, 1);
        let mut p = Points::default();
        let i = ball(&mut p, 3.0, 4.0, 3.0, 4.0);
        assert_eq!(project_segments(&mut p, &s, 1), 1);
        assert!((p.x[i] - 6.0).abs() < 1e-9 && (p.y[i] - 8.0).abs() < 1e-9);
    }

    #[test]
    fn past_the_end_pushes_from_endpoint() {
        let mut s = Segs::default();
        s.add(0.0, 0.0, 100.0, 0.0, 0.0, 1);
        let mut p = Points::default();
        let i = ball(&mut p, -5.0, 3.0, -5.0, 3.0);
        project_segments(&mut p, &s, 1);
        let d = 34.0_f64.sqrt();
        assert!((p.x[i] + 50.0 / d).abs() < 1e-9 && (p.y[i] - 30.0 / d).abs() < 1e-9);
    }

    fn bridge(wp: f64) -> (Points, Edges, usize) {
        let mut p = Points::default();
        let a = p.add(0.0, 0.0, 1.0, 0.0);
        let b = p.add(100.0, 0.0, 1.0, 0.0);
        let c = p.add(50.0, 5.0, wp, 10.0);
        let mut e = Edges::default();
        e.add(a, b, 1, 0.0);
        (p, e, c)
    }

    #[test]
    fn heavy_point_on_edge_keeps_momentum() {
        let (mut p, e, c) = bridge(0.1);
        let before = p.clone();
        assert_eq!(project_edges(&mut p, &e, 1), 1);
        assert!(p.y[0] < 0.0 && p.y[1] < 0.0 && p.y[c] > 5.0);
        let mut sum = (0.0, 0.0);
        for k in 0..3 {
            let m = 1.0 / p.w[k];
            sum.0 += m * (p.x[k] - before.x[k]);
            sum.1 += m * (p.y[k] - before.y[k]);
        }
        assert!(sum.0.abs() < 1e-9 && sum.1.abs() < 1e-9);
        assert!((p.cny[c] - 1.0).abs() < 1e-9 && p.contact[c] == 1.0);
    }

    #[test]
    fn edge_end_is_skipped() {
        let mut p = Points::default();
        let a = p.add(0.0, 0.0, 1.0, 10.0);
        let b = p.add(5.0, 0.0, 1.0, 10.0);
        let mut e = Edges::default();
        e.add(a, b, 1, 0.0);
        assert_eq!(project_edges(&mut p, &e, 2), 0);
        assert_eq!((p.x[a], p.x[b]), (0.0, 5.0));
    }

    #[test]
    fn edge_layer_mask() {
        let (mut p, e, c) = bridge(0.1);
        p.mask[c] = 2;
        assert_eq!(project_edges(&mut p, &e, 1), 0);
        assert_eq!(p.y[c], 5.0);
    }

    #[test]
    fn fixed_edge_acts_like_segment() {
        let (mut p, e, c) = bridge(1.0);
        p.w[0] = 0.0;
        p.w[1] = 0.0;
        let mut q = Points::default();
        let i = q.add(50.0, 5.0, 1.0, 10.0);
        let mut s = Segs::default();
        s.add(0.0, 0.0, 100.0, 0.0, 0.0, 1);
        project_edges(&mut p, &e, 1);
        project_segments(&mut q, &s, 1);
        assert!((p.y[c] - q.y[i]).abs() < 1e-9 && (p.x[c] - q.x[i]).abs() < 1e-9);
        assert_eq!((p.y[0], p.y[1]), (0.0, 0.0));
    }

    fn drop_joints(mask: u32, segs: &Segs) -> Points {
        let mut p = Points::default();
        for k in 0..200 {
            let i = p.add(k as f64 * 2.0, 20.0 + (k % 7) as f64, 1.0, 3.0);
            p.mask[i] = mask;
        }
        for _ in 0..80 {
            for i in 0..p.len() {
                p.ox[i] = p.x[i];
                p.oy[i] = p.y[i];
                p.y[i] -= 4.0;
            }
            project_segments(&mut p, segs, 2);
        }
        p
    }

    fn two_floors() -> Segs {
        let mut s = Segs::default();
        s.add(-10.0, 0.0, 1000.0, 0.0, 0.0, 1);
        s.add(-10.0, -100.0, 1000.0, -100.0, 0.0, 2);
        s
    }

    #[test]
    fn joints_stop_on_solid_and_pass_semi_solid() {
        let p = drop_joints(1, &two_floors());
        assert!(p.y.iter().all(|&y| y >= 3.0 - 1e-9));
        let mut only2 = Segs::default();
        only2.add(-10.0, -100.0, 1000.0, -100.0, 0.0, 2);
        let q = drop_joints(1, &only2);
        assert!(q.y.iter().all(|&y| y < -100.0));
    }

    #[test]
    fn wheel_masks_stop_on_both() {
        for mask in [3, 7] {
            let mut only2 = Segs::default();
            only2.add(-10.0, 0.0, 1000.0, 0.0, 0.0, 2);
            let p = drop_joints(mask, &only2);
            assert!(p.y.iter().all(|&y| y >= 3.0 - 1e-9));
            let q = drop_joints(mask, &two_floors());
            assert!(q.y.iter().all(|&y| y >= 3.0 - 1e-9));
        }
    }

    #[test]
    fn edge_layer_4_ignored_by_mask_3() {
        let (mut p, mut e, c) = bridge(1.0);
        e.layer[0] = 4;
        p.mask[c] = 3;
        assert_eq!(project_edges(&mut p, &e, 1), 0);
        assert_eq!(p.y[c], 5.0);
    }

    #[test]
    fn road_moved_up_through_wheel_keeps_wheel_on_top() {
        let mut p = Points::default();
        let a = p.add(0.0, 0.0, 1.0, 0.0);
        let b = p.add(100.0, 0.0, 1.0, 0.0);
        let c = p.add(50.0, 10.0, 1.0, 10.0);
        let mut e = Edges::default();
        e.add(a, b, 1, 0.0);
        p.y[a] = 15.0;
        p.y[b] = 15.0;
        assert_eq!(project_edges(&mut p, &e, 1), 1);
        let road = (p.y[a] + p.y[b]) / 2.0;
        assert!((p.y[c] - road - 10.0).abs() < 1e-9);
    }

    #[test]
    fn wheel_never_below_fixed_road() {
        let mut p = Points::default();
        let a = p.add(-1000.0, 0.0, 0.0, 0.0);
        let b = p.add(1000.0, 0.0, 0.0, 0.0);
        let c = p.add(0.0, 100.0, 1.0, 10.0);
        let mut e = Edges::default();
        e.add(a, b, 1, 0.0);
        for _ in 0..30 {
            p.ox[c] = p.x[c];
            p.oy[c] = p.y[c];
            p.y[c] -= 50.0;
            project_edges(&mut p, &e, 1);
            assert!(p.y[c] >= 10.0 - 1e-9);
        }
    }

    #[test]
    fn a_capsule_segment_keeps_the_point_one_thickness_further() {
        let (mut s, mut p) = (floor(), Points::default());
        s.rad[0] = 5.0;
        let i = p.add(0.0, 100.0, 1.0, 10.0);
        for _ in 0..200 {
            p.y[i] -= 2.0;
            project_segments(&mut p, &s, 1);
            p.oy[i] = p.y[i];
        }
        assert!((p.y[i] - 15.0).abs() < 1e-6, "rests at r + thickness from the line: {}", p.y[i]);
    }

    #[test]
    fn a_capsule_segment_stops_a_point_that_has_no_radius_of_its_own_only_if_it_has_one() {
        // a point with r = 0 never collides (the old rule), thickness does not change that
        let (mut s, mut p) = (floor(), Points::default());
        s.rad[0] = 5.0;
        let i = p.add(0.0, 2.0, 1.0, 0.0);
        assert_eq!(project_segments(&mut p, &s, 1), 0);
        assert_eq!(p.y[i], 2.0);
    }

    #[test]
    fn a_capsule_edge_keeps_the_point_one_thickness_away() {
        let mut p = Points::default();
        let a = p.add(-100.0, 0.0, 0.0, 0.0);
        let b = p.add(100.0, 0.0, 0.0, 0.0);
        let i = p.add(0.0, -12.0, 1.0, 10.0);
        let mut e = Edges::default();
        e.add(a, b, 1, 0.0);
        e.rad[0] = 5.0;
        for _ in 0..50 {
            p.y[i] += 1.0;
            project_edges(&mut p, &e, 1);
            p.oy[i] = p.y[i];
        }
        assert!((p.y[i] - (-15.0)).abs() < 1e-6, "{}", p.y[i]);
    }

    #[test]
    fn a_shape_does_not_push_itself() {
        // a triangle of three points and three edges, all in group 4: nothing moves
        let mut p = Points::default();
        let ids = [p.add(0.0, 0.0, 1.0, 6.0), p.add(10.0, 0.0, 1.0, 6.0), p.add(5.0, 8.0, 1.0, 6.0)];
        let mut e = Edges::default();
        for (u, v) in [(0, 1), (1, 2), (2, 0)] {
            let k = e.add(ids[u], ids[v], 1, 0.0);
            e.grp[k] = 4;
        }
        let before = p.x.clone();
        for &i in &ids {
            p.grp[i] = 4;
        }
        assert_eq!(project_edges(&mut p, &e, 3), 0);
        assert_eq!(p.x, before);
    }
}
