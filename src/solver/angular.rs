// damp_angular: friction against turning, two kinds, both only slow a motion down (they never
// add energy) and both work on the speed of a Verlet point (x - old_x) by moving old_x, old_y
//
// spin (per link): the part of the relative speed of the two ends that is across the link is
//   the turning of the link. a share k of it is taken away, split between the ends by their
//   inverse masses (a fixed end does not move, the free end takes it all)
// bend (per point): at a point where links meet, the turning of the links against each other.
//   the turning speed of a link about the point is cross(d, v_far - v_point) / |d|^2. every
//   link gives a share k of the difference between its speed and the mean of all the links of
//   the point (weighted by the inertia of the far end, |d|^2 / w), by moving its far end across
//   it; the point takes the opposite momentum, so a joint never pushes the chain by itself
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

// the links of every point (a counting sort over the links: no hashing, a fixed order). the far
// end of each link at a point; built once and reused while the links stay the same
#[derive(Default)]
pub struct Adj {
    start: Vec<usize>,
    far: Vec<u32>,
}

pub fn adjacency(n: usize, l: &Links, adj: &mut Adj) {
    adj.start.clear();
    adj.start.resize(n + 1, 0);
    for k in 0..l.len() {
        adj.start[l.a[k] as usize + 1] += 1;
        adj.start[l.b[k] as usize + 1] += 1;
    }
    for i in 0..n {
        adj.start[i + 1] += adj.start[i];
    }
    adj.far.clear();
    adj.far.resize(adj.start[n], 0);
    // the next free place of every point: start is moved and moved back after
    for k in 0..l.len() {
        let (a, b) = (l.a[k] as usize, l.b[k] as usize);
        adj.far[adj.start[a]] = b as u32;
        adj.start[a] += 1;
        adj.far[adj.start[b]] = a as u32;
        adj.start[b] += 1;
    }
    for i in (1..=n).rev() {
        adj.start[i] = adj.start[i - 1];
    }
    adj.start[0] = 0;
}

// is there any bend to do: the call's share, or a point with its own
pub fn wants_bend(p: &Points, bend: f64) -> bool {
    share(f64::NAN, bend) > 0.0 || p.bend.iter().any(|&b| (0.0..=1.0).contains(&b) && b > 0.0)
}

// is there any spin to do: the call's share, or a link with its own
pub fn wants_spin(l: &Links, spin: f64) -> bool {
    share(f64::NAN, spin) > 0.0 || l.ang.iter().any(|&a| (0.0..=1.0).contains(&a) && a > 0.0)
}

// returns how many links and point pairs were changed. builds the adjacency itself (only when
// a bend is wanted); a caller that runs many steps builds it once and calls damp_with
pub fn damp(p: &mut Points, l: &Links, spin: f64, bend: f64) -> usize {
    let mut adj = Adj::default();
    if wants_bend(p, bend) {
        adjacency(p.len(), l, &mut adj);
    }
    damp_with(p, l, &adj, spin, bend)
}

// adj must be built for these points and links when a bend is wanted (empty is no bend)
pub fn damp_with(p: &mut Points, l: &Links, adj: &Adj, spin: f64, bend: f64) -> usize {
    let mut changed = 0;
    if wants_spin(l, spin) {
        changed += spin_links(p, l, spin);
    }
    let n = p.len();
    if adj.start.len() != n + 1 {
        return changed;
    }
    let (start, far) = (&adj.start, &adj.far);
    // per point: the turning rate of every link about the point, and the mean of the rates weighted
    // by the moment of inertia of each far end (|d|^2 / w). each free far end gives a share of its
    // difference to that mean, the point takes the opposite momentum. a fixed far end (infinite
    // inertia) sets the mean alone. for two links this is the same as the share of their
    // difference; for more it is one pass over the links of the point, not all pairs
    let mut rate: Vec<f64> = Vec::new();
    for c in 0..n {
        let kb = share(p.bend[c], bend);
        if kb <= 0.0 {
            continue;
        }
        let ends = &far[start[c]..start[c + 1]];
        if ends.len() < 2 {
            continue;
        }
        let (vcx, vcy) = (p.x[c] - p.ox[c], p.y[c] - p.oy[c]);
        rate.clear();
        let (mut sum_i, mut sum_iw, mut fixed_n, mut fixed_w) = (0.0, 0.0, 0usize, 0.0);
        for &a in ends {
            let a = a as usize;
            let (dx, dy) = (p.x[a] - p.x[c], p.y[a] - p.y[c]);
            let l2 = dx * dx + dy * dy;
            if l2.is_nan() || l2 < 1e-12 || a == c {
                rate.push(f64::NAN);
                continue;
            }
            let r = (dx * ((p.y[a] - p.oy[a]) - vcy) - dy * ((p.x[a] - p.ox[a]) - vcx)) / l2;
            rate.push(r);
            let wa = inv(p.w[a]);
            if wa > 0.0 {
                let i = l2 / wa;
                sum_i += i;
                sum_iw += i * r;
            } else {
                fixed_n += 1;
                fixed_w += r;
            }
        }
        let mean = if fixed_n > 0 {
            fixed_w / fixed_n as f64
        } else if sum_i > 0.0 {
            sum_iw / sum_i
        } else {
            continue;
        };
        if !mean.is_finite() {
            continue;
        }
        let wc = inv(p.w[c]);
        let (mut rx, mut ry) = (0.0, 0.0);
        for (k, &a) in ends.iter().enumerate() {
            let a = a as usize;
            let wa = inv(p.w[a]);
            let r = rate[k];
            if wa <= 0.0 || !r.is_finite() {
                continue;
            }
            // the speed across the link is rate * perp(d), perp(d) = (-dy, dx); v += s is old -= s
            let dr = -kb * (r - mean);
            let (dx, dy) = (p.x[a] - p.x[c], p.y[a] - p.y[c]);
            let (dvx, dvy) = (dr * -dy, dr * dx);
            p.ox[a] -= dvx;
            p.oy[a] -= dvy;
            rx += dvx / wa;
            ry += dvy / wa;
            changed += 1;
        }
        // the point takes the opposite of the momentum the far ends got
        if wc > 0.0 {
            p.ox[c] += wc * rx;
            p.oy[c] += wc * ry;
        }
    }
    changed
}

fn spin_links(p: &mut Points, l: &Links, spin: f64) -> usize {
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
    fn bend_at_a_joint_of_four_keeps_momentum_and_loses_energy() {
        let mut p = Points::default();
        let c = p.add(0.0, 0.0, 0.3, 0.0);
        let ends = [(10.0, 0.0, 1.0, 0.0, -1.0), (0.0, 20.0, 0.5, 1.5, 0.0), (-15.0, 3.0, 2.0, 0.2, 0.7), (4.0, -9.0, 0.8, -0.4, 0.3)];
        let mut l = Links::default();
        for (x, y, w, vx, vy) in ends {
            let a = p.add(x, y, w, 0.0);
            p.ox[a] = x - vx;
            p.oy[a] = y - vy;
            l.add(c, a, (x * x + y * y).sqrt(), 0.0, 0);
        }
        p.bend[c] = 0.4;
        let mom = |p: &Points| {
            let (mut mx, mut my, mut e) = (0.0, 0.0, 0.0);
            for i in 0..p.len() {
                let (vx, vy) = (p.x[i] - p.ox[i], p.y[i] - p.oy[i]);
                mx += vx / p.w[i];
                my += vy / p.w[i];
                e += (vx * vx + vy * vy) / p.w[i];
            }
            (mx, my, e)
        };
        let before = mom(&p);
        assert_eq!(damp(&mut p, &l, 0.0, 0.0), 4);
        let after = mom(&p);
        assert!((after.0 - before.0).abs() < 1e-9 && (after.1 - before.1).abs() < 1e-9, "{:?} {:?}", before, after);
        assert!(after.2 < before.2, "{:?} {:?}", before, after);
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
