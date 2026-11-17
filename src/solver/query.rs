// query_point and raycast_segments (spec 4.5, 4.6)
use super::data::{Points, Segs};

// the nearest point to (x, y) not farther than max_dist; with Some(mask) only points whose
// layer & mask != 0. on equal distance the smaller entity index (p.ent) wins. returns
// (position in p, distance)
pub fn nearest(p: &Points, x: f64, y: f64, max_dist: f64, mask: Option<u32>) -> Option<(usize, f64)> {
    if !max_dist.is_finite() || max_dist < 0.0 || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let limit = max_dist * max_dist;
    let mut best: Option<(usize, f64)> = None;
    for i in 0..p.len() {
        if let Some(m) = mask {
            if p.layer[i] & m == 0 {
                continue;
            }
        }
        let dx = p.x[i] - x;
        let dy = p.y[i] - y;
        let d2 = dx * dx + dy * dy;
        if !d2.is_finite() || d2 > limit {
            continue;
        }
        let better = match best {
            None => true,
            Some((j, bd)) => d2 < bd || (d2 == bd && p.ent[i] < p.ent[j]),
        };
        if better {
            best = Some((i, d2));
        }
    }
    best.map(|(i, d2)| (i, d2.sqrt()))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    // position in the Segs arrays
    pub seg: usize,
    // distance along the ray
    pub t: f64,
    pub hx: f64,
    pub hy: f64,
    // the normal of the segment, turned to face the ray
    pub nx: f64,
    pub ny: f64,
}

// the first segment on the ray from (x, y) in direction (dx, dy) (any length > 0, it is
// normalized) within max_dist; with Some(mask) only segments whose layer & mask != 0
pub fn raycast(s: &Segs, x: f64, y: f64, dx: f64, dy: f64, max_dist: f64, mask: Option<u32>) -> Option<Hit> {
    if !dx.is_finite() || !dy.is_finite() || !x.is_finite() || !y.is_finite() || max_dist.is_nan() {
        return None;
    }
    let dl = (dx * dx + dy * dy).sqrt();
    if !dl.is_finite() || dl < 1e-12 {
        return None;
    }
    let (dx, dy) = (dx / dl, dy / dl);
    let mut best: Option<(usize, f64)> = None;
    for i in 0..s.len() {
        if let Some(m) = mask {
            if s.layer[i] & m == 0 {
                continue;
            }
        }
        let t = match hit_t(s, i, x, y, dx, dy) {
            Some(t) if t <= max_dist => t,
            _ => continue,
        };
        let better = match best {
            None => true,
            Some((j, bt)) => t < bt || (t == bt && s.ent[i] < s.ent[j]),
        };
        if better {
            best = Some((i, t));
        }
    }
    let (i, t) = best?;
    let ex = s.x2[i] - s.x1[i];
    let ey = s.y2[i] - s.y1[i];
    let len = (ex * ex + ey * ey).sqrt();
    let (mut nx, mut ny) = (-ey / len, ex / len);
    if nx * dx + ny * dy > 0.0 {
        nx = -nx;
        ny = -ny;
    }
    Some(Hit { seg: i, t, hx: x + t * dx, hy: y + t * dy, nx, ny })
}

// the ray parameter t where the unit ray crosses segment i, or None
fn hit_t(s: &Segs, i: usize, x: f64, y: f64, dx: f64, dy: f64) -> Option<f64> {
    let ex = s.x2[i] - s.x1[i];
    let ey = s.y2[i] - s.y1[i];
    if !(ex * ex + ey * ey).sqrt().is_finite() || (ex * ex + ey * ey).sqrt() < 1e-12 {
        return None;
    }
    let denom = dx * ey - dy * ex;
    if denom.abs() < 1e-12 {
        return None;
    }
    let wx = s.x1[i] - x;
    let wy = s.y1[i] - y;
    let t = (wx * ey - wy * ex) / denom;
    let u = (wx * dy - wy * dx) / denom;
    if t.is_finite() && t >= 0.0 && (0.0..=1.0).contains(&u) {
        Some(t)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(list: &[(f64, f64)]) -> Points {
        let mut p = Points::default();
        for &(x, y) in list {
            p.add(x, y, 1.0, 0.0);
        }
        p
    }

    #[test]
    fn nearest_picks_closest() {
        let p = pts(&[(5.0, 0.0), (2.0, 0.0), (3.0, 0.0)]);
        let (i, d) = nearest(&p, 0.0, 0.0, 10.0, None).unwrap();
        assert_eq!(i, 1);
        assert!((d - 2.0).abs() < 1e-12);
    }

    #[test]
    fn nearest_max_dist_inclusive() {
        let p = pts(&[(3.0, 4.0)]);
        assert!(nearest(&p, 0.0, 0.0, 5.0, None).is_some());
        assert!(nearest(&p, 0.0, 0.0, 4.999, None).is_none());
        assert!(nearest(&p, 0.0, 0.0, -1.0, None).is_none());
        assert!(nearest(&p, 0.0, 0.0, f64::NAN, None).is_none());
    }

    #[test]
    fn nearest_mask() {
        let mut p = pts(&[(1.0, 0.0), (2.0, 0.0)]);
        p.layer[0] = 1;
        p.layer[1] = 2;
        assert_eq!(nearest(&p, 0.0, 0.0, 10.0, Some(2)).unwrap().0, 1);
        assert_eq!(nearest(&p, 0.0, 0.0, 10.0, Some(1)).unwrap().0, 0);
        assert!(nearest(&p, 0.0, 0.0, 10.0, Some(4)).is_none());
    }

    #[test]
    fn nearest_tie_smaller_ent() {
        let mut p = pts(&[(1.0, 0.0), (-1.0, 0.0)]);
        p.ent[0] = 9;
        p.ent[1] = 3;
        assert_eq!(nearest(&p, 0.0, 0.0, 10.0, None).unwrap().0, 1);
        p.ent[0] = 2;
        assert_eq!(nearest(&p, 0.0, 0.0, 10.0, None).unwrap().0, 0);
    }

    #[test]
    fn nearest_empty_and_nonfinite() {
        assert!(nearest(&Points::default(), 0.0, 0.0, 10.0, None).is_none());
        let p = pts(&[(f64::NAN, 0.0), (4.0, 0.0)]);
        assert_eq!(nearest(&p, 0.0, 0.0, 10.0, None).unwrap().0, 1);
    }

    fn one_seg() -> Segs {
        let mut s = Segs::default();
        s.add(-1.0, 0.0, 1.0, 0.0, 0.0, 1);
        s
    }

    #[test]
    fn raycast_from_above_and_below() {
        let s = one_seg();
        let h = raycast(&s, 0.0, 5.0, 0.0, -3.0, 100.0, None).unwrap();
        assert_eq!(h.seg, 0);
        assert!((h.t - 5.0).abs() < 1e-12);
        assert!(h.hx.abs() < 1e-12 && h.hy.abs() < 1e-12);
        assert!(h.nx.abs() < 1e-12 && (h.ny - 1.0).abs() < 1e-12);
        let h = raycast(&s, 0.0, -5.0, 0.0, 1.0, 100.0, None).unwrap();
        assert!((h.t - 5.0).abs() < 1e-12);
        assert!(h.nx.abs() < 1e-12 && (h.ny + 1.0).abs() < 1e-12);
    }

    #[test]
    fn raycast_misses_beyond_ends() {
        let s = one_seg();
        assert!(raycast(&s, 1.5, 5.0, 0.0, -1.0, 100.0, None).is_none());
        assert!(raycast(&s, -1.5, 5.0, 0.0, -1.0, 100.0, None).is_none());
        assert!(raycast(&s, 0.0, 5.0, 0.0, 1.0, 100.0, None).is_none());
        assert!(raycast(&s, 0.0, 5.0, 0.0, 0.0, 100.0, None).is_none());
    }

    #[test]
    fn raycast_max_dist() {
        let s = one_seg();
        assert!(raycast(&s, 0.0, 5.0, 0.0, -1.0, 4.9, None).is_none());
        assert!(raycast(&s, 0.0, 5.0, 0.0, -1.0, 5.0, None).is_some());
    }

    #[test]
    fn raycast_nearest_of_two() {
        let mut s = Segs::default();
        s.add(-1.0, 0.0, 1.0, 0.0, 0.0, 1);
        s.add(-1.0, 2.0, 1.0, 2.0, 0.0, 1);
        let h = raycast(&s, 0.0, 5.0, 0.0, -1.0, 100.0, None).unwrap();
        assert_eq!(h.seg, 1);
        assert!((h.t - 3.0).abs() < 1e-12);
    }

    #[test]
    fn raycast_ignores_parallel() {
        let mut s = Segs::default();
        s.add(-1.0, 0.0, 1.0, 0.0, 0.0, 1);
        assert!(raycast(&s, -5.0, 0.0, 1.0, 0.0, 100.0, None).is_none());
        assert!(raycast(&s, -5.0, 1.0, 1.0, 0.0, 100.0, None).is_none());
    }

    #[test]
    fn raycast_mask() {
        let mut s = Segs::default();
        s.add(-1.0, 0.0, 1.0, 0.0, 0.0, 1);
        s.add(-1.0, 2.0, 1.0, 2.0, 0.0, 2);
        let h = raycast(&s, 0.0, 5.0, 0.0, -1.0, 100.0, Some(1)).unwrap();
        assert_eq!(h.seg, 0);
        assert!(raycast(&s, 0.0, 5.0, 0.0, -1.0, 100.0, Some(4)).is_none());
    }

    #[test]
    fn raycast_start_on_segment() {
        let s = one_seg();
        let h = raycast(&s, 0.0, 0.0, 0.0, 1.0, 10.0, None).unwrap();
        assert_eq!(h.t, 0.0);
        assert_eq!((h.hx, h.hy), (0.0, 0.0));
    }

    #[test]
    fn raycast_zero_length_skipped() {
        let mut s = Segs::default();
        s.add(0.0, 0.0, 0.0, 0.0, 0.0, 1);
        assert!(raycast(&s, 0.0, 5.0, 0.0, -1.0, 100.0, None).is_none());
    }
}
