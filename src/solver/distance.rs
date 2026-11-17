// solve_distance: XPBD distance links, Gauss-Seidel (spec 4.2)
use super::data::{Links, Points};

// 1. lambda = 0 for every link. 2. `iterations` passes over the links in order, the XPBD
// update of the spec (modes 0, 1, 2 by clamping the summed lambda, never by skipping a link
// by its length). 3. fmax = max(fmax, |lambda| / (dt*dt)) for every link, once at the end.
// returns the number of links that were solved (not skipped as degenerate)
pub fn solve(p: &mut Points, l: &mut Links, dt: f64, iterations: usize) -> usize {
    let n_links = link_count(l);
    let n_points = p.x.len().min(p.y.len()).min(p.w.len());

    // nan marks "never solved" while we work; it is read as 0 and cleaned at the end.
    // this counts the solved links without any allocation
    for lam in l.lambda.iter_mut().take(n_links) {
        *lam = f64::NAN;
    }

    let dt2 = dt * dt;
    for _ in 0..iterations {
        for i in 0..n_links {
            solve_link(p, l, i, n_points, dt);
        }
    }
    // links never solved still hold nan: they count nothing and get lambda 0
    let mut count = 0;
    for i in 0..n_links {
        let lam = l.lambda[i];
        if lam.is_nan() {
            l.lambda[i] = 0.0;
        } else {
            count += 1;
        }
        let force = if dt2 > 0.0 { l.lambda[i].abs() / dt2 } else { 0.0 };
        if force.is_finite() && force > l.fmax[i] {
            l.fmax[i] = force;
        }
    }
    count
}

// the smallest length of all link columns, so a broken glue cannot make us index out of range
fn link_count(l: &Links) -> usize {
    l.a.len()
        .min(l.b.len())
        .min(l.rest.len())
        .min(l.comp.len())
        .min(l.mode.len())
        .min(l.lambda.len())
        .min(l.fmax.len())
}

// one Gauss-Seidel step of one link; true when it changed lambda and the points
fn solve_link(p: &mut Points, l: &mut Links, i: usize, n_points: usize, dt: f64) -> bool {
    let dt2 = dt * dt;
    let a = l.a[i] as usize;
    let b = l.b[i] as usize;
    if a == b || a >= n_points || b >= n_points {
        return false;
    }
    let dx = p.x[a] - p.x[b];
    let dy = p.y[a] - p.y[b];
    let d = (dx * dx + dy * dy).sqrt();
    if !d.is_finite() || d < 1e-9 {
        return false;
    }
    let nx = dx / d;
    let ny = dy / d;
    let c = d - l.rest[i];
    let at = if dt2 > 0.0 { l.comp[i] / dt2 } else { 0.0 };
    let wa = p.w[a];
    let wb = p.w[b];
    // damping of a spring: the force is k * C + c * dC/dt with beta = c / k (seconds), so the
    // constraint is C + alpha~ * lambda + beta * (n . (x - old)) / dt = 0 and gamma = beta / dt.
    // a rigid link (at = 0) is not damped: it would no longer hold its length
    let beta = l.sdamp.get(i).copied().unwrap_or(0.0);
    let gamma = if dt > 0.0 && at > 0.0 && beta > 0.0 && beta.is_finite() { beta / dt } else { 0.0 };
    let w_sum = (1.0 + gamma) * (wa + wb) + at;
    if !w_sum.is_finite() || w_sum <= 0.0 {
        return false;
    }
    let old = if l.lambda[i].is_nan() { 0.0 } else { l.lambda[i] };
    let vrel = if gamma > 0.0 { nx * ((p.x[a] - p.ox[a]) - (p.x[b] - p.ox[b])) + ny * ((p.y[a] - p.oy[a]) - (p.y[b] - p.oy[b])) } else { 0.0 };
    let dl = (-c - at * old - gamma * vrel) / w_sum;
    let sum = old + dl;
    let new = match l.mode[i] {
        1 => sum.min(0.0),
        2 => sum.max(0.0),
        _ => sum,
    };
    let dl = new - old;
    let ax = p.x[a] + wa * dl * nx;
    let ay = p.y[a] + wa * dl * ny;
    let bx = p.x[b] - wb * dl * nx;
    let by = p.y[b] - wb * dl * ny;
    if !(new.is_finite() && ax.is_finite() && ay.is_finite() && bx.is_finite() && by.is_finite())
    {
        return false;
    }
    l.lambda[i] = new;
    p.x[a] = ax;
    p.y[a] = ay;
    p.x[b] = bx;
    p.y[b] = by;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(p: &Points, a: usize, b: usize) -> f64 {
        ((p.x[a] - p.x[b]).powi(2) + (p.y[a] - p.y[b]).powi(2)).sqrt()
    }

    fn pair(xa: f64, xb: f64, wa: f64, wb: f64, rest: f64, comp: f64, mode: u8) -> (Points, Links) {
        let mut p = Points::default();
        p.add(xa, 0.0, wa, 0.0);
        p.add(xb, 0.0, wb, 0.0);
        let mut l = Links::default();
        l.add(0, 1, rest, comp, mode);
        (p, l)
    }

    // a mass on a soft spring of length 0 to a fixed point, let go 10 away: without damping it
    // swings through to about -10, with critical damping (beta = 2 sqrt(m / k)) it never passes
    #[test]
    fn a_damped_spring_does_not_overshoot() {
        let run = |beta: f64| {
            let (mut p, mut l) = pair(0.0, 10.0, 0.0, 0.02, 0.0, 0.00002, 0);
            l.sdamp[0] = beta;
            let h = 1.0 / 960.0;
            let mut least = f64::INFINITY;
            for _ in 0..960 {
                let (x, ox) = (p.x[1], p.ox[1]);
                p.ox[1] = x;
                p.x[1] = x + (x - ox);
                solve(&mut p, &mut l, h, 1);
                least = least.min(p.x[1]);
            }
            least
        };
        assert!(run(0.0) < -9.0, "{}", run(0.0));
        let beta = 2.0 * (50.0f64 * 0.00002).sqrt();
        assert!(run(beta) > -0.05, "{}", run(beta));
    }

    #[test]
    fn two_free_points_reach_rest() {
        let (mut p, mut l) = pair(0.0, 14.0, 1.0, 1.0, 10.0, 0.0, 0);
        p.y[1] = 3.0;
        let cx = (p.x[0] + p.x[1]) / 2.0;
        let cy = (p.y[0] + p.y[1]) / 2.0;
        let n = solve(&mut p, &mut l, 0.01, 50);
        assert_eq!(n, 1);
        assert!((dist(&p, 0, 1) - 10.0).abs() < 1e-9);
        assert!((cx - (p.x[0] + p.x[1]) / 2.0).abs() < 1e-9);
        assert!((cy - (p.y[0] + p.y[1]) / 2.0).abs() < 1e-9);
    }

    #[test]
    fn fixed_ends() {
        let (mut p, mut l) = pair(0.0, 14.0, 0.0, 1.0, 10.0, 0.0, 0);
        solve(&mut p, &mut l, 0.01, 10);
        assert_eq!(p.x[0], 0.0);
        assert!((p.x[1] - 10.0).abs() < 1e-9);

        let (mut p, mut l) = pair(0.0, 14.0, 0.0, 0.0, 10.0, 0.0, 0);
        let n = solve(&mut p, &mut l, 0.01, 10);
        assert_eq!(n, 0);
        assert_eq!((p.x[0], p.x[1]), (0.0, 14.0));
    }

    #[test]
    fn cable_and_strut() {
        // cable: slack does nothing
        let (mut p, mut l) = pair(0.0, 6.0, 1.0, 1.0, 10.0, 0.0, 1);
        solve(&mut p, &mut l, 0.01, 10);
        assert_eq!((p.x[0], p.x[1]), (0.0, 6.0));
        assert_eq!(l.lambda[0], 0.0);
        // cable: stretched is pulled to rest
        let (mut p, mut l) = pair(0.0, 14.0, 1.0, 1.0, 10.0, 0.0, 1);
        solve(&mut p, &mut l, 0.01, 10);
        assert!((dist(&p, 0, 1) - 10.0).abs() < 1e-9);
        assert!(l.lambda[0] < 0.0);
        // strut: stretched does nothing
        let (mut p, mut l) = pair(0.0, 14.0, 1.0, 1.0, 10.0, 0.0, 2);
        solve(&mut p, &mut l, 0.01, 10);
        assert_eq!((p.x[0], p.x[1]), (0.0, 14.0));
        assert_eq!(l.lambda[0], 0.0);
        // strut: squeezed is pushed to rest
        let (mut p, mut l) = pair(0.0, 6.0, 1.0, 1.0, 10.0, 0.0, 2);
        solve(&mut p, &mut l, 0.01, 10);
        assert!((dist(&p, 0, 1) - 10.0).abs() < 1e-9);
        assert!(l.lambda[0] > 0.0);
    }

    #[test]
    fn compliance_yields() {
        let (mut p, mut l) = pair(0.0, 12.0, 1.0, 1.0, 10.0, 0.01, 0);
        solve(&mut p, &mut l, 0.1, 20);
        let d = dist(&p, 0, 1);
        assert!(d > 10.0 && d < 12.0, "d = {d}");
        assert!(l.lambda[0].is_finite());
    }

    #[test]
    fn hanging_chain_keeps_lengths() {
        let mut p = Points::default();
        let mut l = Links::default();
        for i in 0..11 {
            let w = if i == 0 || i == 10 { 0.0 } else { 1.0 };
            p.add(0.8 * i as f64, 0.0, w, 0.0);
        }
        for i in 0..10 {
            l.add(i, i + 1, 1.0, 0.0, 0);
        }
        let dt = 1.0 / 60.0;
        for _ in 0..30 {
            for i in 0..11 {
                if p.w[i] > 0.0 {
                    p.y[i] -= 9.81 * dt * dt;
                }
            }
            solve(&mut p, &mut l, dt, 5000);
        }
        for i in 0..10 {
            assert!((dist(&p, i, i + 1) - 1.0).abs() < 1e-6, "link {i}");
        }
        assert!(p.y[5] < 0.0);
    }

    #[test]
    fn fmax_rises_and_stays() {
        let (mut p, mut l) = pair(0.0, 14.0, 1.0, 1.0, 10.0, 0.0, 0);
        solve(&mut p, &mut l, 0.5, 10);
        let first = l.fmax[0];
        assert!((first - 2.0 / 0.25).abs() < 1e-9);
        p.x[1] = p.x[0] + 10.5;
        solve(&mut p, &mut l, 0.5, 10);
        assert_eq!(l.fmax[0], first);
        p.x[1] = p.x[0] + 30.0;
        solve(&mut p, &mut l, 0.5, 10);
        assert!(l.fmax[0] > first);
    }

    #[test]
    fn same_point_link_is_skipped() {
        let mut p = Points::default();
        p.add(1.0, 2.0, 1.0, 0.0);
        let mut l = Links::default();
        l.add(0, 0, 5.0, 0.0, 0);
        l.add(0, 7, 5.0, 0.0, 0);
        let n = solve(&mut p, &mut l, 0.01, 5);
        assert_eq!(n, 0);
        assert_eq!((p.x[0], p.y[0]), (1.0, 2.0));
        assert_eq!(l.lambda[0], 0.0);
    }

    #[test]
    fn force_relation() {
        // C = 4, wa + wb = 2, no compliance: lambda = -C / 2 = -2 (stays so on more passes)
        for iters in [1, 5] {
            let (mut p, mut l) = pair(0.0, 14.0, 1.0, 1.0, 10.0, 0.0, 0);
            solve(&mut p, &mut l, 1.0, iters);
            assert!((l.lambda[0] + 2.0).abs() < 1e-12);
            assert!((l.fmax[0] - 2.0).abs() < 1e-12);
            assert!((p.x[0] - 2.0).abs() < 1e-12 && (p.x[1] - 12.0).abs() < 1e-12);
        }
    }

    #[test]
    fn bad_dt_does_not_panic() {
        let (mut p, mut l) = pair(0.0, 14.0, 1.0, 1.0, 10.0, 0.5, 0);
        solve(&mut p, &mut l, 0.0, 3);
        solve(&mut p, &mut l, f64::NAN, 3);
        assert!(p.x[0].is_finite() && p.x[1].is_finite());
        assert!(l.lambda[0].is_finite());
    }
}
