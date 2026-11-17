// project_points: round points (radius r) push each other apart. with project_segments and
// project_edges this covers circles, capsules and closed shapes: a shape is points joined by
// links, its sides are edges, and all of them carry one `group` so they do not push each other
use super::data::{Links, Points};
use std::cmp::Ordering;

// a pair is skipped when both layers do not let the other in (each mask must meet the
// layer of the other), when both have the same non-zero group, or when a link joins them
fn pair_allowed(p: &Points, a: usize, b: usize) -> bool {
    if p.mask[a] & p.layer[b] == 0 || p.mask[b] & p.layer[a] == 0 {
        return false;
    }
    !(p.grp[a] != 0 && p.grp[a] == p.grp[b])
}

// the pairs joined by a link, sorted, as (smaller position, bigger position)
fn linked_pairs(l: &Links) -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = (0..l.len()).map(|k| (l.a[k].min(l.b[k]), l.a[k].max(l.b[k]))).collect();
    v.sort_unstable();
    v.dedup();
    v
}

// a fixed point (w = 0) takes part as an obstacle; one with a broken mass or radius does not
fn takes_part(p: &Points, i: usize) -> bool {
    let (w, r) = (p.w[i], p.r[i]);
    w.is_finite() && w >= 0.0 && r.is_finite() && r > 0.0 && p.x[i].is_finite() && p.y[i].is_finite()
}

// the push of one overlapping pair: both move apart along the line of their centres in
// proportion to their inverse masses, and the speed they approach each other with is taken
// away (no bounce). returns false when nothing was done
fn push_pair(p: &mut Points, a: usize, b: usize) -> bool {
    let (wa, wb) = (p.w[a], p.w[b]);
    let ws = wa + wb;
    if ws.partial_cmp(&0.0) != Some(Ordering::Greater) || !ws.is_finite() {
        return false;
    }
    let (dx, dy) = (p.x[a] - p.x[b], p.y[a] - p.y[b]);
    let reach = p.r[a] + p.r[b];
    let d2 = dx * dx + dy * dy;
    if !d2.is_finite() || d2 >= reach * reach {
        return false;
    }
    let d = d2.sqrt();
    // on top of each other: the way they came from, else a fixed direction
    let n = if d > 1e-9 {
        (dx / d, dy / d)
    } else {
        let (ox, oy) = (p.ox[a] - p.ox[b], p.oy[a] - p.oy[b]);
        let od = (ox * ox + oy * oy).sqrt();
        if od > 1e-9 {
            (ox / od, oy / od)
        } else {
            (1.0, 0.0)
        }
    };
    let s = reach - d;
    let (va, vb) = ((p.x[a] - p.ox[a], p.y[a] - p.oy[a]), (p.x[b] - p.ox[b], p.y[b] - p.oy[b]));
    let rel_n = (va.0 - vb.0) * n.0 + (va.1 - vb.1) * n.1;
    let (ka, kb) = (wa / ws, wb / ws);
    let na = (p.x[a] + n.0 * s * ka, p.y[a] + n.1 * s * ka);
    let nb = (p.x[b] - n.0 * s * kb, p.y[b] - n.1 * s * kb);
    let (mut nva, mut nvb) = (va, vb);
    if rel_n < 0.0 {
        nva = (va.0 - n.0 * rel_n * ka, va.1 - n.1 * rel_n * ka);
        nvb = (vb.0 + n.0 * rel_n * kb, vb.1 + n.1 * rel_n * kb);
    }
    let (oa, ob) = ((na.0 - nva.0, na.1 - nva.1), (nb.0 - nvb.0, nb.1 - nvb.1));
    let all = [na.0, na.1, nb.0, nb.1, oa.0, oa.1, ob.0, ob.1];
    if !all.iter().all(|v| v.is_finite()) {
        return false;
    }
    (p.x[a], p.y[a], p.ox[a], p.oy[a]) = (na.0, na.1, oa.0, oa.1);
    (p.x[b], p.y[b], p.ox[b], p.oy[b]) = (nb.0, nb.1, ob.0, ob.1);
    // the normal of a contact points away from what pushed
    (p.contact[a], p.cnx[a], p.cny[a]) = (1.0, n.0, n.1);
    (p.contact[b], p.cnx[b], p.cny[b]) = (1.0, -n.0, -n.1);
    true
}

// every pair of points (w > 0, r > 0) that overlap, found by a sweep along x: sorted by the
// left side x - r, ties by position, so the order is always the same. `iterations` passes.
// returns the number of pushes done
pub fn project_points(p: &mut Points, l: &Links, iterations: usize) -> usize {
    let linked = linked_pairs(l);
    let mut order: Vec<u32> = Vec::new();
    let mut pushes = 0;
    for _ in 0..iterations {
        order.clear();
        order.extend((0..p.len()).filter(|&i| takes_part(p, i)).map(|i| i as u32));
        order.sort_by(|&i, &j| {
            let (ki, kj) = (p.x[i as usize] - p.r[i as usize], p.x[j as usize] - p.r[j as usize]);
            ki.total_cmp(&kj).then(i.cmp(&j))
        });
        for u in 0..order.len() {
            let a = order[u] as usize;
            let right = p.x[a] + p.r[a];
            for &bv in &order[u + 1..] {
                let b = bv as usize;
                if p.x[b] - p.r[b] > right {
                    break;
                }
                if (p.y[a] - p.y[b]).abs() > p.r[a] + p.r[b] || !pair_allowed(p, a, b) {
                    continue;
                }
                let key = (a.min(b) as u32, a.max(b) as u32);
                if linked.binary_search(&key).is_ok() {
                    continue;
                }
                if push_pair(p, a, b) {
                    pushes += 1;
                }
            }
        }
    }
    pushes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two(ax: f64, bx: f64) -> Points {
        let mut p = Points::default();
        p.add(ax, 0.0, 1.0, 10.0);
        p.add(bx, 0.0, 1.0, 10.0);
        p
    }

    #[test]
    fn overlapping_circles_part_to_touching() {
        let mut p = two(0.0, 15.0);
        assert_eq!(project_points(&mut p, &Links::default(), 1), 1);
        assert!(((p.x[1] - p.x[0]).abs() - 20.0).abs() < 1e-9);
        assert!((p.x[0] + p.x[1] - 15.0).abs() < 1e-9, "equal masses move equally");
        assert_eq!(p.contact[0], 1.0);
        assert!(p.cnx[0] < 0.0 && p.cnx[1] > 0.0);
    }

    #[test]
    fn apart_circles_are_left_alone() {
        let mut p = two(0.0, 25.0);
        assert_eq!(project_points(&mut p, &Links::default(), 3), 0);
        assert_eq!((p.x[0], p.x[1]), (0.0, 25.0));
    }

    #[test]
    fn a_fixed_point_does_not_move_the_other_takes_it_all() {
        let mut p = two(0.0, 15.0);
        p.w[0] = 0.0;
        project_points(&mut p, &Links::default(), 1);
        assert_eq!(p.x[0], 0.0);
        assert!((p.x[1] - 20.0).abs() < 1e-9);
    }

    #[test]
    fn two_fixed_points_do_nothing() {
        let mut p = two(0.0, 15.0);
        p.w[0] = 0.0;
        p.w[1] = 0.0;
        assert_eq!(project_points(&mut p, &Links::default(), 1), 0);
    }

    #[test]
    fn approach_speed_is_taken_away_and_the_rest_kept() {
        let mut p = two(0.0, 19.0);
        // a moves right by 3 and up by 2 per step, b stands still
        p.ox[0] = -3.0;
        p.oy[0] = -2.0;
        project_points(&mut p, &Links::default(), 1);
        let (va, vb) = ((p.x[0] - p.ox[0], p.y[0] - p.oy[0]), (p.x[1] - p.ox[1], p.y[1] - p.oy[1]));
        assert!((va.0 - vb.0).abs() < 1e-9, "no approach along x any more: {:?} {:?}", va, vb);
        assert!((va.1 - 2.0).abs() < 1e-9, "the sideways speed stays");
    }

    #[test]
    fn layers_masks_and_groups_decide_who_collides() {
        let mut p = two(0.0, 15.0);
        p.layer[1] = 2;
        p.mask[0] = 1; // a does not let layer 2 in
        assert_eq!(project_points(&mut p, &Links::default(), 1), 0);
        p.mask[0] = 3;
        p.mask[1] = 4; // b does not let layer 1 in: both must agree
        assert_eq!(project_points(&mut p, &Links::default(), 1), 0);
        p.mask[1] = 3;
        p.grp[0] = 7;
        p.grp[1] = 7;
        assert_eq!(project_points(&mut p, &Links::default(), 1), 0, "same group");
        p.grp[1] = 8;
        assert_eq!(project_points(&mut p, &Links::default(), 1), 1);
    }

    #[test]
    fn linked_points_do_not_push() {
        let mut p = two(0.0, 15.0);
        let mut l = Links::default();
        l.add(1, 0, 15.0, 0.0, 0);
        assert_eq!(project_points(&mut p, &l, 1), 0);
    }

    #[test]
    fn points_on_top_of_each_other_part_without_nan() {
        let mut p = two(5.0, 5.0);
        assert_eq!(project_points(&mut p, &Links::default(), 1), 1);
        assert!(p.x.iter().all(|v| v.is_finite()));
        assert!(((p.x[0] - p.x[1]).abs() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn a_pile_stays_apart_and_the_order_is_fixed() {
        let build = || {
            let mut p = Points::default();
            for k in 0..30 {
                p.add((k % 6) as f64 * 5.0, (k / 6) as f64 * 5.0, 1.0, 8.0);
            }
            p
        };
        let (mut a, mut b) = (build(), build());
        for _ in 0..20 {
            project_points(&mut a, &Links::default(), 1);
            project_points(&mut b, &Links::default(), 1);
        }
        assert_eq!(a.x, b.x);
        assert_eq!(a.y, b.y);
        for i in 0..30 {
            for j in i + 1..30 {
                let d = ((a.x[i] - a.x[j]).powi(2) + (a.y[i] - a.y[j]).powi(2)).sqrt();
                assert!(d > 16.0 - 1.0, "{} {} too close: {}", i, j, d);
            }
        }
    }

    #[test]
    fn a_nan_point_is_ignored() {
        let mut p = two(0.0, 15.0);
        p.x[0] = f64::NAN;
        assert_eq!(project_points(&mut p, &Links::default(), 1), 0);
    }

    #[test]
    fn joints_hit_only_the_ground_and_pass_each_other_and_wheels() {
        // the convention: a joint is layer 8 mask 1, a wheel layer 16 mask 23
        let mut p = Points::default();
        let (j1, j2, wheel, ball) = (p.add(0.0, 0.0, 1.0, 5.0), p.add(6.0, 0.0, 1.0, 5.0), p.add(3.0, 0.0, 1.0, 5.0), p.add(3.0, 3.0, 1.0, 5.0));
        for j in [j1, j2] {
            p.layer[j] = 8;
            p.mask[j] = 1;
        }
        for w in [wheel, ball] {
            p.layer[w] = 16;
            p.mask[w] = 23;
        }
        let before = (p.x[j1], p.x[j2]);
        let n = project_points(&mut p, &Links::default(), 1);
        assert_eq!(n, 1, "only the wheel and the ball push each other");
        assert_eq!((p.x[j1], p.x[j2]), before);
        assert_eq!(p.contact[j1], 0.0);
        assert_eq!(p.contact[wheel], 1.0);
    }
}
