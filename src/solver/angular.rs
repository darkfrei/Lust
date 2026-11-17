// damp_angular: friction against turning, two kinds, both only slow a motion down (they never
// add energy) and both work on the speed of a Verlet point (x - old_x) by moving old_x, old_y
//
// spin (per link): the part of the relative speed of the two ends that is across the link is
//   the turning of the link. a share k of it is taken away, split between the ends by their
//   inverse masses (a fixed end does not move, the free end takes it all)
// bend (per point): at a point where two links meet, the turning of one link against the
//   other. the turning speed of a link about the point is cross(d, v_far - v_point) / |d|^2.
//   a share k of the difference of the two is taken away, by moving the far ends across
//   their links (the light end moves more); the point takes the opposite momentum, so a joint
//   never pushes the chain by itself
//
// k is a share (0 none, 1 all of it in one step). a link or a point may carry its own k
// (NaN: the value given to the call). the order is the order of the arrays, so the result is
// the same on every run
use super::data::{Links, Points};

fn share(own: f64, call: f64) -> f64 {
    let k = if (0.0..=1.0).contains(&own) { own } else { call };
    if k.is_finite() {
        k.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn inv(w: f64) -> f64 {
    if w.is_finite() && w > 0.0 {
        w
    } else {
        0.0
    }
}

// returns how many links and point pairs were changed
pub fn damp(p: &mut Points, l: &Links, spin: f64, bend: f64) -> usize {
    let mut changed = 0;
    for k in 0..l.len() {
        let ks = share(l.ang[k], spin);
        if ks <= 0.0 {
            continue;
        }
        let (a, b) = (l.a[k] as usize, l.b[k] as usize);
        let (wa, wb) = (inv(p.w[a]), inv(p.w[b]));
        if wa + wb <= 0.0 {
            continue;
        }
        let (dx, dy) = (p.x[b] - p.x[a], p.y[b] - p.y[a]);
        let len = dx.hypot(dy);
        if len < 1e-12 {
            continue;
        }
        let (nx, ny) = (dx / len, dy / len);
        let rx = (p.x[b] - p.ox[b]) - (p.x[a] - p.ox[a]);
        let ry = (p.y[b] - p.oy[b]) - (p.y[a] - p.oy[a]);
        let along = rx * nx + ry * ny;
        let (tx, ty) = (rx - along * nx, ry - along * ny);
        let (sa, sb) = (wa / (wa + wb), wb / (wa + wb));
        // a takes a share of the turning in its own direction, b gives the same back
        p.ox[a] -= sa * ks * tx;
        p.oy[a] -= sa * ks * ty;
        p.ox[b] += sb * ks * tx;
        p.oy[b] += sb * ks * ty;
        changed += 1;
    }

    // the links of every point, in the order of the links (a counting sort: no hashing)
    let n = p.len();
    let mut start = vec![0usize; n + 1];
    for k in 0..l.len() {
        start[l.a[k] as usize + 1] += 1;
        start[l.b[k] as usize + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    // for each incident link: the far end
    let mut far = vec![0u32; start[n]];
    for k in 0..l.len() {
        let (a, b) = (l.a[k] as usize, l.b[k] as usize);
        far[fill[a]] = b as u32;
        fill[a] += 1;
        far[fill[b]] = a as u32;
        fill[b] += 1;
    }
    for c in 0..n {
        let kb = share(p.bend[c], bend);
        if kb <= 0.0 {
            continue;
        }
        let ends = &far[start[c]..start[c + 1]];
        for i in 0..ends.len() {
            for j in i + 1..ends.len() {
                let (a, b) = (ends[i] as usize, ends[j] as usize);
                if a == b {
                    continue;
                }
                let (wa, wb) = (inv(p.w[a]), inv(p.w[b]));
                if wa + wb <= 0.0 {
                    continue;
                }
                let (vcx, vcy) = (p.x[c] - p.ox[c], p.y[c] - p.oy[c]);
                let (dax, day) = (p.x[a] - p.x[c], p.y[a] - p.y[c]);
                let (dbx, dby) = (p.x[b] - p.x[c], p.y[b] - p.y[c]);
                let (la, lb) = (dax * dax + day * day, dbx * dbx + dby * dby);
                if la < 1e-12 || lb < 1e-12 {
                    continue;
                }
                let wa_rate = (dax * ((p.y[a] - p.oy[a]) - vcy) - day * ((p.x[a] - p.ox[a]) - vcx)) / la;
                let wb_rate = (dbx * ((p.y[b] - p.oy[b]) - vcy) - dby * ((p.x[b] - p.ox[b]) - vcx)) / lb;
                let rel = wb_rate - wa_rate;
                // the share goes by the moment of inertia of each far end about the point
                // (|d|^2 / w), the light one turns more; with this the turning momentum is kept
                // and only energy is lost, whatever the lengths and masses are
                let (ua, ub) = ((lb * wa) / (la * wb + lb * wa), (la * wb) / (la * wb + lb * wa));
                // link a turns faster by ua * kb * rel, link b slower by ub * kb * rel; the speed
                // across a link is rate * perp(d), perp(d) = (-dy, dx); v += s means old -= s
                let da = ua * kb * rel;
                let db = -ub * kb * rel;
                let (dvax, dvay) = (da * -day, da * dax);
                let (dvbx, dvby) = (db * -dby, db * dbx);
                p.ox[a] -= dvax;
                p.oy[a] -= dvay;
                p.ox[b] -= dvbx;
                p.oy[b] -= dvby;
                // the point takes the opposite of the momentum the two ends got, so the joint does
                // not push the whole chain (a fixed point takes it without moving)
                let wc = inv(p.w[c]);
                if wc > 0.0 {
                    let (ia, ib) = (if wa > 0.0 { 1.0 / wa } else { 0.0 }, if wb > 0.0 { 1.0 / wb } else { 0.0 });
                    p.ox[c] += wc * (dvax * ia + dvbx * ib);
                    p.oy[c] += wc * (dvay * ia + dvby * ib);
                }
                changed += 1;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    // two free points joined by a link, spinning about their middle at the same speed
    fn spinner() -> (Points, Links) {
        let mut p = Points::default();
        let a = p.add(0.0, 0.0, 1.0, 0.0);
        let b = p.add(10.0, 0.0, 1.0, 0.0);
        // a moves down, b moves up: a turn about the middle (counter clockwise in y down)
        p.oy[a] = -1.0;
        p.oy[b] = 1.0;
        let mut l = Links::default();
        l.add(a, b, 10.0, 0.0, 0);
        (p, l)
    }

    fn rate(p: &Points) -> f64 {
        ((p.y[1] - p.oy[1]) - (p.y[0] - p.oy[0])) / 10.0
    }

    #[test]
    fn spin_takes_a_share_of_the_turning_and_keeps_the_drift() {
        let (mut p, l) = spinner();
        let before = rate(&p);
        damp(&mut p, &l, 0.25, 0.0);
        assert!((rate(&p) - before * 0.75).abs() < 1e-12, "{} {}", rate(&p), before);
        // the drift (the speed of the middle) is not touched
        let mid = ((p.y[1] - p.oy[1]) + (p.y[0] - p.oy[0])) / 2.0;
        assert!(mid.abs() < 1e-12);
    }

    #[test]
    fn a_pull_along_the_link_is_not_spin() {
        let mut p = Points::default();
        let a = p.add(0.0, 0.0, 1.0, 0.0);
        let b = p.add(10.0, 0.0, 1.0, 0.0);
        p.ox[b] = 9.0;
        let mut l = Links::default();
        l.add(a, b, 10.0, 0.0, 0);
        damp(&mut p, &l, 1.0, 1.0);
        assert!((p.x[b] - p.ox[b] - 1.0).abs() < 1e-12 && (p.x[a] - p.ox[a]).abs() < 1e-12);
    }

    #[test]
    fn a_fixed_end_does_not_move_and_the_free_end_takes_it_all() {
        let mut p = Points::default();
        let a = p.add(0.0, 0.0, 0.0, 0.0);
        let b = p.add(10.0, 0.0, 1.0, 0.0);
        p.oy[b] = 2.0 - 0.0; // b moved by -2 in y
        let mut l = Links::default();
        l.add(a, b, 10.0, 0.0, 0);
        damp(&mut p, &l, 1.0, 0.0);
        assert!(p.y[b] - p.oy[b] < 1e-12 && (p.oy[a]).abs() < 1e-12 && p.ox[a] == 0.0);
    }

    #[test]
    fn a_link_with_its_own_share_uses_it_and_zero_means_none() {
        let (mut p, mut l) = spinner();
        l.ang[0] = 0.0;
        let before = rate(&p);
        damp(&mut p, &l, 0.9, 0.0);
        assert!((rate(&p) - before).abs() < 1e-12);
        l.ang[0] = 0.5;
        damp(&mut p, &l, 0.0, 0.0);
        assert!((rate(&p) - before * 0.5).abs() < 1e-12);
    }

    // a joint with two links (an elbow); the far ends turn about the joint in opposite ways
    fn elbow() -> (Points, Links) {
        let mut p = Points::default();
        let c = p.add(0.0, 0.0, 0.0, 0.0);
        let a = p.add(10.0, 0.0, 1.0, 0.0);
        let b = p.add(-10.0, 0.0, 1.0, 0.0);
        // a moves down, b moves down too: about c a turns one way, b the other way
        p.oy[a] = -1.0;
        p.oy[b] = -1.0;
        let mut l = Links::default();
        l.add(c, a, 10.0, 0.0, 0);
        l.add(c, b, 10.0, 0.0, 0);
        (p, l)
    }

    #[test]
    fn bend_takes_a_share_of_the_turning_of_one_link_against_the_other() {
        let (mut p, l) = elbow();
        p.bend[0] = 0.5;
        // before: a rate +0.1 (down at +x is counter clockwise), b rate -0.1: relative -0.2
        damp(&mut p, &l, 0.0, 0.0);
        let va = p.y[1] - p.oy[1];
        let vb = p.y[2] - p.oy[2];
        // rates were +0.1 and -0.1 (relative -0.2); half of it is taken, each link gives a quarter,
        // so the far ends keep half of their speed of 1
        assert!((va - 0.5).abs() < 1e-12 && (vb - 0.5).abs() < 1e-12, "{} {}", va, vb);
    }

    #[test]
    fn bend_does_nothing_when_both_links_turn_together_and_never_adds_energy() {
        let mut p = Points::default();
        let c = p.add(0.0, 0.0, 0.0, 0.0);
        let a = p.add(10.0, 0.0, 1.0, 0.0);
        let b = p.add(-10.0, 0.0, 1.0, 0.0);
        // a straight rigid bar turning about c: a down, b up
        p.oy[a] = -1.0;
        p.oy[b] = 1.0;
        p.bend[c] = 1.0;
        let mut l = Links::default();
        l.add(c, a, 10.0, 0.0, 0);
        l.add(c, b, 10.0, 0.0, 0);
        damp(&mut p, &l, 0.0, 0.0);
        assert!((p.y[a] - p.oy[a] - 1.0).abs() < 1e-12 && (p.y[b] - p.oy[b] + 1.0).abs() < 1e-12);
        // links of very different length and masses: energy still never grows
        let mut p = Points::default();
        let c = p.add(0.0, 0.0, 0.0, 0.0);
        let a = p.add(110.0, 0.0, 0.05, 0.0);
        let b = p.add(0.0, 40.0, 2.0, 0.0);
        p.oy[a] = -1.0;
        p.ox[b] = 1.5;
        p.bend[c] = 0.7;
        let mut l = Links::default();
        l.add(c, a, 110.0, 0.0, 0);
        l.add(c, b, 40.0, 0.0, 0);
        let e = |p: &Points| (1..3).map(|i| (p.x[i] - p.ox[i]).powi(2) / p.w[i] + (p.y[i] - p.oy[i]).powi(2) / p.w[i]).sum::<f64>();
        let before = e(&p);
        damp(&mut p, &l, 0.0, 0.0);
        assert!(e(&p) < before, "{} {}", e(&p), before);
        // a random mess: energy never grows
        let (mut p, l) = elbow();
        p.bend[0] = 1.0;
        let e = |p: &Points| (1..3).map(|i| (p.x[i] - p.ox[i]).powi(2) + (p.y[i] - p.oy[i]).powi(2)).sum::<f64>();
        let before = e(&p);
        damp(&mut p, &l, 0.3, 1.0);
        assert!(e(&p) <= before + 1e-12);
    }

    #[test]
    fn bend_keeps_the_momentum_when_the_joint_is_free() {
        let mut p = Points::default();
        let c = p.add(0.0, 0.0, 0.5, 0.0);
        let a = p.add(10.0, 0.0, 1.0, 0.0);
        let b = p.add(0.0, 25.0, 0.25, 0.0);
        p.oy[a] = -1.0;
        p.ox[b] = 2.0;
        p.bend[c] = 0.6;
        let mut l = Links::default();
        l.add(c, a, 10.0, 0.0, 0);
        l.add(c, b, 25.0, 0.0, 0);
        let mom = |p: &Points| {
            let mut m = (0.0, 0.0);
            for i in 0..3 {
                m.0 += (p.x[i] - p.ox[i]) / p.w[i];
                m.1 += (p.y[i] - p.oy[i]) / p.w[i];
            }
            m
        };
        let before = mom(&p);
        assert!(damp(&mut p, &l, 0.0, 0.0) > 0);
        let after = mom(&p);
        assert!((after.0 - before.0).abs() < 1e-9 && (after.1 - before.1).abs() < 1e-9, "{:?} {:?}", before, after);
    }

    #[test]
    fn bad_numbers_and_fixed_points_do_nothing_and_do_not_panic() {
        let mut p = Points::default();
        let a = p.add(0.0, 0.0, 0.0, 0.0);
        let b = p.add(0.0, 0.0, 0.0, 0.0);
        let mut l = Links::default();
        l.add(a, b, 1.0, 0.0, 0);
        assert_eq!(damp(&mut p, &l, f64::NAN, f64::INFINITY), 0);
        assert_eq!(damp(&mut p, &l, 1.0, 1.0), 0);
    }
}
