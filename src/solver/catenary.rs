// the shape of a hanging cable and the forces it puts on its ends (spec 4.7, 4.8)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    // a line between the ends (taut, or no gravity, or bad input)
    Straight,
    // both ends at the same x: a line down to the lowest point and up again
    Vertical,
    // y = a*cosh((x - x0)/a) + y0
    Curve,
}

// world coordinates (y as given, not mirrored): for Curve a is signed (negative when the
// cable hangs toward +y, that is when gy > 0) so that y = a*cosh((x-x0)/a) + y0 always holds.
// Straight: a = 0, sag = 0. Vertical: a = 0, sag > 0, y0 is the lowest point (world y)
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub kind: Kind,
    pub a: f64,
    pub x0: f64,
    pub y0: f64,
    // the biggest distance of the curve from the chord, in the direction of gravity
    // (Vertical: how far the lowest point is below the lower end)
    pub sag: f64,
}

impl Shape {
    pub fn straight() -> Shape {
        Shape { kind: Kind::Straight, a: 0.0, x0: 0.0, y0: 0.0, sag: 0.0 }
    }
}

// the shape of a cable of length `len` hung between A (xa, ya) and B (xb, yb) under gravity
// gy (its sign says which way it sags: gy > 0 toward +y). see the spec for the method
// ends = [xa, ya, xb, yb]
pub fn shape(ends: [f64; 4], len: f64, gy: f64) -> Shape {
    let bad = ends.iter().any(|v| !v.is_finite()) || !len.is_finite() || !gy.is_finite();
    if bad || gy == 0.0 || len <= 0.0 {
        return Shape::straight();
    }
    let d = (ends[2] - ends[0]).hypot(ends[3] - ends[1]);
    let slack = len > d * (1.0 + 1e-9);
    if !slack {
        return Shape::straight();
    }
    let sgn = if gy > 0.0 { -1.0 } else { 1.0 };
    let (mut xl, mut yl, mut xr, mut yr) = (ends[0], sgn * ends[1], ends[2], sgn * ends[3]);
    if xl > xr {
        std::mem::swap(&mut xl, &mut xr);
        std::mem::swap(&mut yl, &mut yr);
    }
    let dx = xr - xl;
    let dy = yr - yl;
    let sh = if dx < 1e-6 {
        let y0f = (yl + yr - len) / 2.0;
        Shape {
            kind: Kind::Vertical,
            a: 0.0,
            x0: (xl + xr) / 2.0,
            y0: sgn * y0f,
            sag: yl.min(yr) - y0f,
        }
    } else {
        curve_shape(xl, yl, dx, dy, len, d, sgn)
    };
    let ok = sh.a.is_finite() && sh.x0.is_finite() && sh.y0.is_finite() && sh.sag.is_finite();
    if ok {
        sh
    } else {
        Shape::straight()
    }
}

// sinh(u)/u - 1, with a series for small u
fn sinhm1(u: f64) -> f64 {
    if u < 0.1 {
        let u2 = u * u;
        u2 / 6.0 * (1.0 + u2 / 20.0 * (1.0 + u2 / 42.0 * (1.0 + u2 / 72.0)))
    } else {
        u.sinh() / u - 1.0
    }
}

// derivative of sinhm1
fn sinhm1_d(u: f64) -> f64 {
    if u < 0.1 {
        let u2 = u * u;
        u / 3.0 * (1.0 + u2 / 10.0 * (1.0 + u2 / 28.0))
    } else {
        (u * u.cosh() - u.sinh()) / (u * u)
    }
}

// solve sinhm1(u) = km1 for u > 0 (bracketed newton)
fn solve_u(km1: f64) -> f64 {
    let mut lo = 0.0_f64;
    let mut hi = (6.0 * km1).sqrt().min(2.0 * (2.0 * (km1 + 1.0)).ln());
    for _ in 0..60 {
        if sinhm1(hi) >= km1 {
            break;
        }
        hi *= 2.0;
    }
    let mut u = hi;
    for _ in 0..100 {
        let g = sinhm1(u) - km1;
        if g < 0.0 {
            lo = u;
        } else {
            hi = u;
        }
        let dg = sinhm1_d(u);
        let mut next = u - g / dg;
        if !next.is_finite() || next <= lo || next >= hi {
            next = (lo + hi) / 2.0;
        }
        let step = (next - u).abs();
        u = next;
        if step <= 1e-13 * u.max(1e-8) {
            break;
        }
    }
    u
}

// the curve in the frame with y up; returns the shape in world terms
fn curve_shape(xl: f64, yl: f64, dx: f64, dy: f64, len: f64, d: f64, sgn: f64) -> Shape {
    let s = (len * len - dy * dy).sqrt();
    let km1 = (len - d) * (len + d) / (dx * (s + dx));
    let u = solve_u(km1);
    let aa = dx / (2.0 * u);
    let x0 = (xl + xl + dx) / 2.0 - aa * (dy / len).atanh();
    let y0 = yl - aa * ((xl - x0) / aa).cosh();
    let xs = x0 + aa * (dy / dx).asinh();
    let ys = aa * ((xs - x0) / aa).cosh() + y0;
    let chord = yl + dy / dx * (xs - xl);
    let sag = (chord - ys).max(0.0);
    Shape { kind: Kind::Curve, a: sgn * aa, x0, y0: sgn * y0, sag }
}

// the point at t in 0..1 along x from A to B (Curve), along the arc length (Vertical), or on
// the line (Straight). t is clamped to 0..1
pub fn sample(sh: &Shape, ends: [f64; 4], len: f64, t: f64) -> (f64, f64) {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    let [xa, ya, xb, yb] = ends;
    let line = (xa + (xb - xa) * t, ya + (yb - ya) * t);
    let p = match sh.kind {
        Kind::Straight => line,
        Kind::Curve => {
            let x = xa + (xb - xa) * t;
            (x, sh.a * ((x - sh.x0) / sh.a).cosh() + sh.y0)
        }
        Kind::Vertical => {
            let la = (ya - sh.y0).abs();
            let lb = (yb - sh.y0).abs();
            let total = la + lb;
            let pos = (t * total).min(total);
            let _ = len;
            if pos <= la {
                (xa, ya + (sh.y0 - ya).signum() * pos.min(la))
            } else {
                (xb, sh.y0 + (yb - sh.y0).signum() * (pos - la).min(lb))
            }
        }
    };
    if p.0.is_finite() && p.1.is_finite() {
        p
    } else {
        line
    }
}

// the forces the cable puts on its ends, (force on A, force on B), for a cable of linear
// density rho under gravity gy (rho*|gy| is the weight per length). Curve: H = rho*|gy|*|a|
// capped at 20*rho*|gy|*len, left end (H, -s*H*sinh(ul)), right end (-H, s*H*sinh(ur)) with
// s = sign(gy), ul and ur = (x - x0)/|a| of the left and the right end (the end with the
// smaller x is the left one), and the forces are given back to the real A and B. Vertical:
// each end gets rho*|gy|*(arc length from it to the lowest point) in the direction of gy.
// Straight: each end gets half the weight in the direction of gy. gy == 0 or rho <= 0: zeros
pub fn end_forces(sh: &Shape, ends: [f64; 4], len: f64, rho: f64, gy: f64) -> ((f64, f64), (f64, f64)) {
    let zero = ((0.0, 0.0), (0.0, 0.0));
    let bad = ends.iter().any(|v| !v.is_finite()) || !len.is_finite() || !rho.is_finite();
    if bad || !gy.is_finite() || gy == 0.0 || rho <= 0.0 {
        return zero;
    }
    let g = gy.abs();
    let s = gy.signum();
    let w = rho * g;
    let out = match sh.kind {
        Kind::Straight => ((0.0, s * w * len / 2.0), (0.0, s * w * len / 2.0)),
        Kind::Vertical => {
            let la = (ends[1] - sh.y0).abs();
            let lb = (ends[3] - sh.y0).abs();
            ((0.0, s * w * la), (0.0, s * w * lb))
        }
        Kind::Curve => {
            let aabs = sh.a.abs();
            let h = (w * aabs).min(20.0 * w * len);
            let a_left = ends[0] <= ends[2];
            let (xl, xr) = if a_left { (ends[0], ends[2]) } else { (ends[2], ends[0]) };
            let ul = (xl - sh.x0) / aabs;
            let ur = (xr - sh.x0) / aabs;
            let fl = (h, -s * h * ul.sinh());
            let fr = (-h, s * h * ur.sinh());
            if a_left {
                (fl, fr)
            } else {
                (fr, fl)
            }
        }
    };
    let all = [out.0 .0, out.0 .1, out.1 .0, out.1 .1];
    if all.iter().all(|v| v.is_finite()) {
        out
    } else {
        zero
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // arc length of the curve between the ends by simpson, 2000 intervals
    fn arc(sh: &Shape, ends: [f64; 4]) -> f64 {
        let n = 2000;
        let (xa, xb) = (ends[0], ends[2]);
        let h = (xb - xa) / n as f64;
        let f = |x: f64| (1.0 + ((x - sh.x0) / sh.a).sinh().powi(2)).sqrt();
        let mut sum = f(xa) + f(xb);
        for i in 1..n {
            let w = if i % 2 == 1 { 4.0 } else { 2.0 };
            sum += w * f(xa + h * i as f64);
        }
        (sum * h / 3.0).abs()
    }

    fn check_curve(ends: [f64; 4], len: f64, gy: f64) -> Shape {
        let sh = shape(ends, len, gy);
        assert_eq!(sh.kind, Kind::Curve);
        assert!(gy > 0.0 && sh.a < 0.0 || gy < 0.0 && sh.a > 0.0);
        let l = arc(&sh, ends);
        assert!((l - len).abs() <= 1e-6 * len, "arc {} len {}", l, len);
        let scale = 1.0 + ends.iter().fold(0.0_f64, |m, v| m.max(v.abs())) + len;
        let p0 = sample(&sh, ends, len, 0.0);
        let p1 = sample(&sh, ends, len, 1.0);
        assert!((p0.0 - ends[0]).abs() < 1e-9 * scale && (p0.1 - ends[1]).abs() < 1e-9 * scale);
        assert!((p1.0 - ends[2]).abs() < 1e-9 * scale && (p1.1 - ends[3]).abs() < 1e-9 * scale);
        sh
    }

    // length for a given k = sqrt(L^2 - dy^2)/dx
    fn len_for_k(dx: f64, dy: f64, k: f64) -> f64 {
        ((k * dx).powi(2) + dy * dy).sqrt()
    }

    #[test]
    fn arc_length_both_gravity_signs() {
        for gy in [980.0, -980.0] {
            check_curve([0.0, 0.0, 100.0, 0.0], 120.0, gy);
            check_curve([100.0, 10.0, 0.0, -5.0], 130.0, gy);
        }
    }

    #[test]
    fn arc_length_k_values() {
        for gy in [9.8, -9.8] {
            check_curve([0.0, 0.0, 10.0, 0.0], len_for_k(10.0, 0.0, 1.0001), gy);
            check_curve([0.0, 0.0, 10.0, 0.0], len_for_k(10.0, 0.0, 50.0), gy);
            check_curve([0.0, 0.0, 10.0, 3.0], len_for_k(10.0, 3.0, 50.0), gy);
            check_curve([0.0, 0.0, 10.0, 0.0], len_for_k(10.0, 0.0, 1.0 + 1e-8), gy);
        }
    }

    #[test]
    fn different_heights() {
        for gy in [1.0, -1.0] {
            check_curve([0.0, 0.0, 10.0, 80.0], 100.0, gy);
            check_curve([0.0, 90.0, 10.0, 0.0], 95.0, gy);
        }
    }

    #[test]
    fn hangs_on_the_right_side() {
        let ends = [0.0, 0.0, 100.0, 0.0];
        let down = shape(ends, 120.0, 1.0);
        let m = sample(&down, ends, 120.0, 0.5);
        assert!(m.1 > 0.0);
        assert!(down.sag > 0.0);
        let up = shape(ends, 120.0, -1.0);
        let m = sample(&up, ends, 120.0, 0.5);
        assert!(m.1 < 0.0);
    }

    #[test]
    fn straight_cases() {
        let ends = [0.0, 0.0, 30.0, 40.0];
        assert_eq!(shape(ends, 50.0, 1.0).kind, Kind::Straight);
        assert_eq!(shape(ends, 40.0, 1.0).kind, Kind::Straight);
        assert_eq!(shape(ends, 80.0, 0.0).kind, Kind::Straight);
        assert_eq!(shape(ends, f64::NAN, 1.0).kind, Kind::Straight);
        let sh = shape(ends, 50.0, 1.0);
        assert_eq!(sample(&sh, ends, 50.0, 0.5), (15.0, 20.0));
    }

    #[test]
    fn vertical_shape() {
        let ends = [5.0, 0.0, 5.0, 10.0];
        for gy in [1.0, -1.0] {
            let sh = shape(ends, 30.0, gy);
            assert_eq!(sh.kind, Kind::Vertical);
            assert!(sh.sag > 0.0);
            let a = sample(&sh, ends, 30.0, 0.0);
            let b = sample(&sh, ends, 30.0, 1.0);
            assert!((a.0 - 5.0).abs() < 1e-12 && a.1.abs() < 1e-12);
            assert!((b.0 - 5.0).abs() < 1e-12 && (b.1 - 10.0).abs() < 1e-12);
            // lowest point is 10 beyond the lower end: y0 = -10 (gy < 0 means y down... see below)
            let expect = if gy > 0.0 { 20.0 } else { -10.0 };
            assert!((sh.y0 - expect).abs() < 1e-9, "y0 {}", sh.y0);
            let la = (0.0 - sh.y0).abs();
            let mid = sample(&sh, ends, 30.0, la / (la + (10.0 - sh.y0).abs()));
            assert!((mid.1 - sh.y0).abs() < 1e-9);
        }
    }

    #[test]
    fn forces_curve() {
        let rho = 2.0;
        for gy in [9.8_f64, -9.8] {
            let s = gy.signum();
            let ends = [0.0, 0.0, 40.0, 0.0];
            let len = 50.0;
            let sh = shape(ends, len, gy);
            let (fa, fb) = end_forces(&sh, ends, len, rho, gy);
            let weight = rho * 9.8 * len;
            assert!(((fa.1 + fb.1) - s * weight).abs() < 1e-6 * weight);
            assert!((fa.0 + fb.0).abs() < 1e-9 * weight);
            assert!(fa.0 > 0.0 && fb.0 < 0.0);
            // symmetric hanging gives symmetric forces
            assert!((fa.1 - fb.1).abs() < 1e-9 * weight);
            // A is pulled toward the cable: along gy and toward +x
            assert!(fa.1 * s > 0.0);
        }
    }

    #[test]
    fn forces_swapped_ends_and_heights() {
        let rho = 1.5;
        let gy = 9.8;
        let ends = [60.0, 5.0, 0.0, -20.0];
        let len = 90.0;
        let sh = shape(ends, len, gy);
        let (fa, fb) = end_forces(&sh, ends, len, rho, gy);
        let weight = rho * gy * len;
        assert!(((fa.1 + fb.1) - weight).abs() < 1e-6 * weight);
        // A is the right end: pulled toward -x
        assert!(fa.0 < 0.0 && fb.0 > 0.0);
        assert!((fa.0 + fb.0).abs() < 1e-9 * weight);
    }

    #[test]
    fn forces_other_kinds() {
        let rho = 3.0;
        let gy = -10.0;
        let ends = [0.0, 0.0, 0.0, 10.0];
        let sh = shape(ends, 30.0, gy);
        let (fa, fb) = end_forces(&sh, ends, 30.0, rho, gy);
        assert!(((fa.1 + fb.1) + rho * 10.0 * 30.0).abs() < 1e-9);
        assert!(fa.0 == 0.0 && fb.0 == 0.0);
        let st = Shape::straight();
        let (fa, fb) = end_forces(&st, [0.0, 0.0, 30.0, 40.0], 50.0, 2.0, 5.0);
        assert_eq!(fa, (0.0, 250.0));
        assert_eq!(fb, (0.0, 250.0));
        let z = end_forces(&sh, ends, 30.0, 0.0, gy);
        assert_eq!(z, ((0.0, 0.0), (0.0, 0.0)));
    }

    #[test]
    fn forces_mirror() {
        let ends = [0.0, 0.0, 40.0, 7.0];
        let len = 55.0;
        let sp = shape(ends, len, 9.8);
        let sn = shape([0.0, 0.0, 40.0, -7.0], len, -9.8);
        let (fa, fb) = end_forces(&sp, ends, len, 1.0, 9.8);
        let (ga, gb) = end_forces(&sn, [0.0, 0.0, 40.0, -7.0], len, 1.0, -9.8);
        assert!((fa.0 - ga.0).abs() < 1e-9 && (fa.1 + ga.1).abs() < 1e-9);
        assert!((fb.0 - gb.0).abs() < 1e-9 && (fb.1 + gb.1).abs() < 1e-9);
    }

    // a cable pulls its ends toward each other and toward the way gravity points; it never
    // pushes them apart or against gravity, for every slack, height and gravity sign
    #[test]
    fn a_cable_only_pulls_never_pushes() {
        for gy in [980.0, -980.0] {
            for dy in [-80.0, 0.0, 60.0] {
                for k in [1.0005, 1.01, 1.1, 1.5, 3.0] {
                    let chord = (200.0f64 * 200.0 + dy * dy).sqrt();
                    let len = chord * k;
                    for ends in [[0.0, 0.0, 200.0, dy], [200.0, dy, 0.0, 0.0]] {
                        let sh = shape(ends, len, gy);
                        let (fa, fb) = end_forces(&sh, ends, len, 0.5, gy);
                        // toward each other along x
                        let (left, right) = if ends[0] < ends[2] { (fa, fb) } else { (fb, fa) };
                        assert!(left.0 >= 0.0 && right.0 <= 0.0, "pushes apart: gy {} dy {} k {} {:?} {:?}", gy, dy, k, fa, fb);
                        // the weight of the cable pulls the ends with gravity, in total
                        assert!((fa.1 + fb.1) * gy >= 0.0, "total vertical against gravity: gy {} dy {} k {}", gy, dy, k);
                    }
                }
            }
        }
    }
}
