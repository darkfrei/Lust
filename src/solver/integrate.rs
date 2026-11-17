// integrate_verlet: one inertial step of every free point (see docs in the spec, 4.1)
use super::data::Points;

// for every point with w > 0:
//   vx = (x - ox) * damping (the point's own `damp` when it has one), vy likewise; ox = x, oy = y;
//   x += vx + gx*dt*dt, y += vy + gy*dt*dt
// points with w <= 0 do not move. contact is set to 0 for ALL points (a contact belongs to
// one step). returns the number of points that moved
pub fn verlet(p: &mut Points, dt: f64, gx: f64, gy: f64, damping: f64) -> usize {
    let n = p.len();
    let dd = dt * dt;
    let mut moved = 0;
    for i in 0..n {
        p.contact[i] = 0.0;
        p.cnx[i] = 0.0;
        p.cny[i] = 0.0;
        // NaN fails this test too, so such a point stays where it is
        if p.w[i].partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            continue;
        }
        let (x, y) = (p.x[i], p.y[i]);
        // a point with its own damping (0..1, the share of speed it keeps) uses it
        let k = if (0.0..=1.0).contains(&p.damp[i]) { p.damp[i] } else { damping };
        let vx = (x - p.ox[i]) * k;
        let vy = (y - p.oy[i]) * k;
        let nx = x + vx + gx * dd;
        let ny = y + vy + gy * dd;
        // a bad number is never written: the point keeps its old state
        if !nx.is_finite() || !ny.is_finite() {
            continue;
        }
        p.ox[i] = x;
        p.oy[i] = y;
        p.x[i] = nx;
        p.y[i] = ny;
        moved += 1;
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_with_its_own_damping_uses_it() {
        let mut p = Points::default();
        let (a, b) = (p.add(0.0, 0.0, 1.0, 0.0), p.add(0.0, 0.0, 1.0, 0.0));
        p.ox[a] = -10.0;
        p.ox[b] = -10.0;
        p.damp[b] = 0.5;
        verlet(&mut p, 0.01, 0.0, 0.0, 1.0);
        assert_eq!(p.x[a], 10.0, "no damping of its own: the call's value 1.0");
        assert_eq!(p.x[b], 5.0, "its own 0.5");
        // a broken own value falls back to the call's
        p.damp[a] = f64::NAN;
        p.damp[b] = 7.0;
        p.ox[a] = p.x[a] - 4.0;
        p.ox[b] = p.x[b] - 4.0;
        let (xa, xb) = (p.x[a], p.x[b]);
        verlet(&mut p, 0.01, 0.0, 0.0, 0.5);
        assert_eq!((p.x[a], p.x[b]), (xa + 2.0, xb + 2.0));
    }

    #[test]
    fn free_fall_matches_formula() {
        let mut p = Points::default();
        p.add(1.0, 2.0, 1.0, 0.0);
        let (dt, g, steps) = (0.01, -9.81, 50usize);
        // independent loop
        let (mut ey, mut eoy) = (2.0f64, 2.0f64);
        for _ in 0..steps {
            let ny = ey + (ey - eoy) + g * dt * dt;
            eoy = ey;
            ey = ny;
        }
        for _ in 0..steps {
            assert_eq!(verlet(&mut p, dt, 0.0, g, 1.0), 1);
        }
        assert_eq!(p.y[0], ey);
        assert_eq!(p.oy[0], eoy);
        assert_eq!(p.x[0], 1.0);
        // closed form for zero start speed: y_n = y0 + g dt^2 n (n+1) / 2
        let n = steps as f64;
        let closed = 2.0 + g * dt * dt * n * (n + 1.0) / 2.0;
        assert!((p.y[0] - closed).abs() < 1e-9);
    }

    #[test]
    fn fixed_point_does_not_move() {
        let mut p = Points::default();
        p.add(1.0, 1.0, 0.0, 0.0);
        p.ox[0] = 0.0;
        p.oy[0] = 0.0;
        assert_eq!(verlet(&mut p, 0.1, 0.0, -10.0, 1.0), 0);
        assert_eq!((p.x[0], p.y[0], p.ox[0], p.oy[0]), (1.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn negative_and_nan_mass_do_not_move() {
        let mut p = Points::default();
        p.add(1.0, 1.0, -1.0, 0.0);
        p.add(2.0, 2.0, f64::NAN, 0.0);
        assert_eq!(verlet(&mut p, 0.1, 0.0, -10.0, 1.0), 0);
        assert_eq!((p.x[0], p.y[0]), (1.0, 1.0));
        assert_eq!((p.x[1], p.y[1]), (2.0, 2.0));
    }

    #[test]
    fn damping_halves_carried_speed() {
        let mut p = Points::default();
        p.add(3.0, 0.0, 1.0, 0.0);
        p.ox[0] = 1.0;
        assert_eq!(verlet(&mut p, 0.1, 0.0, 0.0, 0.5), 1);
        // carried speed was 2, half of it is 1
        assert_eq!(p.x[0], 4.0);
        assert_eq!(p.ox[0], 3.0);
        assert_eq!(p.y[0], 0.0);
    }

    #[test]
    fn contact_is_cleared_for_all_points() {
        let mut p = Points::default();
        p.add(0.0, 0.0, 0.0, 1.0);
        p.add(0.0, 0.0, 1.0, 1.0);
        for i in 0..2 {
            p.contact[i] = 1.0;
            p.cnx[i] = 0.6;
            p.cny[i] = 0.8;
        }
        verlet(&mut p, 0.1, 0.0, 0.0, 1.0);
        for i in 0..2 {
            assert_eq!((p.contact[i], p.cnx[i], p.cny[i]), (0.0, 0.0, 0.0));
        }
    }

    #[test]
    fn nan_and_inf_are_not_written() {
        let mut p = Points::default();
        p.add(f64::NAN, 1.0, 1.0, 0.0);
        p.add(1.0, 1.0, 1.0, 0.0);
        p.add(2.0, 2.0, 1.0, 0.0);
        assert_eq!(verlet(&mut p, 0.1, f64::INFINITY, 0.0, 1.0), 0);
        assert!(p.x[0].is_nan());
        assert_eq!((p.x[1], p.y[1], p.ox[1], p.oy[1]), (1.0, 1.0, 1.0, 1.0));
        assert_eq!((p.x[2], p.y[2]), (2.0, 2.0));
        assert_eq!(verlet(&mut p, 0.1, 0.0, 0.0, f64::NAN), 0);
        assert_eq!((p.x[1], p.y[1]), (1.0, 1.0));
    }

    #[test]
    fn returns_count_of_moved() {
        let mut p = Points::default();
        p.add(0.0, 0.0, 1.0, 0.0);
        p.add(0.0, 0.0, 0.0, 0.0);
        p.add(0.0, 0.0, 2.0, 0.0);
        assert_eq!(verlet(&mut p, 0.1, 0.0, -1.0, 1.0), 2);
        assert_eq!(verlet(&mut Points::default(), 0.1, 0.0, -1.0, 1.0), 0);
    }
}
