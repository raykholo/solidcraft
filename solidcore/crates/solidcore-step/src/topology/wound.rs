//! Faces that wind round their axis more than a turn (a thread's flank: a cylinder face bounded
//! by two helices that go round it several times). The face is on its form (`form::surface_within`)
//! run on from quarter to quarter, which does not close on itself: the same point of the surface
//! is a point of the form once a turn, four parameters apart. SMLib's trims do not take such a
//! form for a face's: a closest point on it may be on any of its sheets, and a trim made that way
//! hops between them. A use's pcurve here is the 3D edge's own angle about the axis, followed
//! continuously (so the turns it makes are counted), and its height, as a cubic Hermite spline in
//! the form's parameters through exact values and slopes, refined where its lift is further than
//! a tenth of the tolerance from the surface point the edge's curve is over. Each loop's uses
//! are then moved by whole turns (four parameters each) so that every one starts where the one
//! before it ends and the loop lies on the form.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_8, TAU};

use crate::kernel::{CurveEval, form};
use crate::vec::Vec3Geom as _;
use solidcore_record::Vec3;
use solidcore_record::brep::{Curve as BCurve, Curve2, Surface as BSurface};

use super::{R, unsupported};

/// A turn round the axis in the parameter of a form on the quarter grid.
const FORM_TURN: f64 = 4.0;
/// How far, in that parameter, a loop's range may lie off its form's range and still be on it:
/// a boundary lies off its surface by its edges' tolerances, far under a turn.
const FORM_SLACK: f64 = 0.5;
/// A pcurve's lift is accepted this far (as a fraction of the tolerance) from the surface.
const LIFT: f64 = 0.1;
/// Pieces one pcurve may have.
const MAX_PIECES: usize = 4000;
/// Times a piece may be halved.
const MAX_HALVINGS: u32 = 30;

/// The angle about the axis, the height along it, and their first derivatives in the curve's
/// parameter, at `t`.
struct Spot {
    theta: f64,
    height: f64,
    d_theta: f64,
    d_height: f64,
}

/// The axis of a cylinder or cone: where it starts, its x and y, its direction, and its radius at
/// height `h`.
struct Axis {
    origin: Vec3,
    x: Vec3,
    y: Vec3,
    z: Vec3,
    radius: Box<dyn Fn(f64) -> f64>,
}

fn axis(label: &BSurface) -> R<Axis> {
    let frame = form::frame_of(label).ok_or_else(|| unsupported("a face that winds round a surface with no axis"))?;
    let (origin, x, y) = (Vec3::from(frame.origin), Vec3::from(frame.x_axis), Vec3::from(frame.y_axis));
    let radius: Box<dyn Fn(f64) -> f64> = match label {
        BSurface::Cylinder { radius, .. } => {
            let r = *radius;
            Box::new(move |_| r)
        }
        BSurface::Cone { radius_at_origin, half_angle, .. } => {
            let (r, slope) = (*radius_at_origin, half_angle.tan());
            Box::new(move |h| r + h * slope)
        }
        _ => return Err(unsupported("a face that winds round a surface that is not a cylinder or a cone")),
    };
    let z = x.cross(y).normalized().ok_or("a degenerate frame")?;
    Ok(Axis { origin, x, y, z, radius })
}

impl Axis {
    fn spot(&self, c: &CurveEval, t: f64) -> R<Spot> {
        let (p, d) = c.derivs(t).ok_or("an edge point")?;
        let r = p - self.origin;
        let (x, y) = (r.dot(self.x), r.dot(self.y));
        let round = x * x + y * y;
        if !(round > 1e-18) {
            return Err(unsupported("an edge of a face that winds round its axis on the axis"));
        }
        Ok(Spot { theta: y.atan2(x), height: r.dot(self.z), d_theta: (x * d.dot(self.y) - y * d.dot(self.x)) / round, d_height: d.dot(self.z) })
    }

    /// The surface point at form parameters `u` (angle by the form's quarters) and height `h`.
    fn lift(&self, u: f64, h: f64) -> Vec3 {
        let (s, c) = form::angle(u).sin_cos();
        self.origin + self.x * ((self.radius)(h) * c) + self.y * ((self.radius)(h) * s) + self.z * h
    }
}

/// The form parameter at the angle `theta` and its derivative in the angle.
fn along(theta: f64) -> (f64, f64) {
    let q = (theta / FRAC_PI_2).floor();
    let psi = (theta - q * FRAC_PI_2).clamp(0.0, FRAC_PI_2);
    let eighth = FRAC_PI_8;
    let t = (0.5 * psi - eighth).tan();
    (form::param(theta), 0.25 * (1.0 + t * t) / eighth.tan())
}

/// A piece of the pcurve: its parameters, and at each end the angle followed from the start (not
/// wrapped), the height, and the derivatives of the form parameter and the height.
#[derive(Clone, Copy)]
struct End {
    t: f64,
    theta: f64,
    u: f64,
    v: f64,
    du: f64,
    dv: f64,
    d_theta: f64,
}

/// The cubic Hermite value at `s` in `[0, 1]` from values `p` and slopes `m` (per unit of `s`).
fn hermite(p: [f64; 2], m: [f64; 2], s: f64) -> f64 {
    let (s2, s3) = (s * s, s * s * s);
    (2.0 * s3 - 3.0 * s2 + 1.0) * p[0] + (s3 - 2.0 * s2 + s) * m[0] + (-2.0 * s3 + 3.0 * s2) * p[1] + (s3 - s2) * m[1]
}

/// `theta` (wrapped) moved by whole turns to the one nearest `near`.
fn nearest(theta: f64, near: f64) -> f64 {
    theta + ((near - theta) / TAU).round() * TAU
}

impl Axis {
    fn end(&self, c: &CurveEval, t: f64, near: Option<f64>) -> R<End> {
        let s = self.spot(c, t)?;
        let theta = near.map_or(s.theta, |n| nearest(s.theta, n));
        let (u, scale) = along(theta);
        Ok(End { t, theta, u, v: s.height, du: scale * s.d_theta, dv: s.d_height, d_theta: s.d_theta })
    }
}

/// The pcurve of the edge on `curve` over `interval` in the form's parameters, at `tol`.
///
/// # Errors
/// An edge on the axis, one whose curve cannot be evaluated, or more pieces than a pcurve takes.
pub(super) fn pcurve(label: &BSurface, curve: &BCurve, [a, b]: [f64; 2], tol: f64) -> R<Curve2> {
    let ax = axis(label)?;
    let c = CurveEval::new(curve)?;
    let first = ax.end(&c, a, None)?;
    // The pieces, in order: enough that none turns more than an eighth of a turn, as the
    // derivative says (so a wrap is never taken for a turn).
    let mut total = 0.0;
    let mut last = ax.spot(&c, a)?.d_theta;
    for i in 1..=64 {
        let now = ax.spot(&c, a + (b - a) * f64::from(i) / 64.0)?.d_theta;
        total += 0.5 * (last.abs() + now.abs()) * (b - a) / 64.0;
        last = now;
    }
    let n = ((total / FRAC_PI_8).ceil() as usize).clamp(16, MAX_PIECES / 4);
    let mut ends = vec![first];
    for i in 1..=n {
        let t = a + (b - a) * i as f64 / n as f64;
        let prev = ends.last().copied().ok_or("pcurve ends")?;
        let guess = prev.theta + 0.5 * (b - a) / n as f64 * (prev.d_theta + ax.spot(&c, t)?.d_theta);
        ends.push(ax.end(&c, t, Some(guess))?);
    }
    let mut pieces: Vec<[End; 2]> = ends.windows(2).map(|w| [w[0], w[1]]).collect();
    let mut done: Vec<[End; 2]> = Vec::new();
    let mut halvings = vec![0u32; pieces.len()];
    pieces.reverse();
    halvings.reverse();
    let target = LIFT * tol;
    while let Some([p, q]) = pieces.pop() {
        let depth = halvings.pop().unwrap_or(0);
        let h = q.t - p.t;
        // The exact point at s of the piece (angle followed from the piece's own by the
        // Hermite estimate) against the Hermite spline there.
        let mut worst = if (q.theta - p.theta).abs() > FRAC_PI_8 { f64::INFINITY } else { 0.0 };
        let mut mid = None;
        for s in [0.25, 0.5, 0.75] {
            let theta_h = hermite([p.theta, q.theta], [h * p.d_theta, h * q.d_theta], s);
            let exact = ax.end(&c, p.t + h * s, Some(theta_h))?;
            if s == 0.5 {
                mid = Some(exact);
            }
            let u_h = hermite([p.u, q.u], [h * p.du, h * q.du], s);
            let v_h = hermite([p.v, q.v], [h * p.dv, h * q.dv], s);
            worst = worst.max(ax.lift(u_h, v_h).dist(ax.lift(exact.u, exact.v)));
        }
        if worst <= target {
            done.push([p, q]);
            continue;
        }
        if depth >= MAX_HALVINGS || done.len() + pieces.len() >= MAX_PIECES {
            return Err(unsupported(format!("an edge of a face that winds round its axis, whose pcurve takes more than {MAX_PIECES} pieces")));
        }
        let mid = mid.ok_or("pcurve midpoint")?;
        pieces.push([mid, q]);
        halvings.push(depth + 1);
        pieces.push([p, mid]);
        halvings.push(depth + 1);
    }
    // Bezier pieces: a cubic from each Hermite piece, its interior knots threefold.
    let mut knots = vec![a; 4];
    let mut points = vec![[first.u, first.v]];
    for (k, [p, q]) in done.iter().enumerate() {
        let h = q.t - p.t;
        points.push([p.u + h * p.du / 3.0, p.v + h * p.dv / 3.0]);
        points.push([q.u - h * q.du / 3.0, q.v - h * q.dv / 3.0]);
        points.push([q.u, q.v]);
        if k + 1 < done.len() {
            knots.extend([q.t; 3]);
        }
    }
    knots.extend([b; 4]);
    Ok(Curve2::Spline { knots, points })
}

/// The loops of a face that winds round its axis on a form that does not close on itself: each
/// use's pcurve moved by whole turns so that it starts where the one before it ends, the loop
/// then moved by whole turns to start on the form's range `range` in u. A pcurve moved a turn is
/// moved four parameters: the form's control points are the same on each. A loop that does not
/// close once unwrapped, or that runs further than the form does, is refused.
pub(super) fn unwind(edge_loops: &[Vec<(u32, bool)>], pcs: &mut [Vec<Curve2>], range: [f64; 2]) -> R<()> {
    let at = |c: &Curve2, end: bool| -> R<[f64; 2]> {
        let [a, b] = c.interval().ok_or("pcurve interval")?;
        solidcore_ops::pcurve::eval(c, if end { b } else { a }).ok_or_else(|| "pcurve end".to_string())
    };
    for (l, loop_pcs) in edge_loops.iter().zip(pcs.iter_mut()) {
        let mut previous: Option<f64> = None;
        let mut first = 0.0;
        let mut moves = Vec::with_capacity(loop_pcs.len());
        for (&(_, forward), c) in l.iter().zip(loop_pcs.iter()) {
            let (s, e) = (at(c, false)?[0], at(c, true)?[0]);
            let (start, end) = if forward { (s, e) } else { (e, s) };
            let by = previous.map_or(0.0, |p| ((p - start) / FORM_TURN).round() * FORM_TURN);
            if previous.is_none() {
                first = start;
            }
            previous = Some(end + by);
            moves.push(by);
        }
        // The last use ends where the first starts, to within a turn's fraction.
        if let Some(p) = previous
            && (p - first).abs() > 0.5 * FORM_TURN
        {
            return Err(unsupported(format!("a loop that winds {:.1} turns about its axis and does not close", (p - first) / FORM_TURN)));
        }
        // The loop's extent in u, from the points each pcurve passes through.
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for (c, by) in loop_pcs.iter().zip(&moves) {
            let [a, b] = c.interval().ok_or("pcurve interval")?;
            for k in 0..=32 {
                let q = solidcore_ops::pcurve::eval(c, a + (b - a) * f64::from(k) / 32.0).ok_or("pcurve point")?;
                lo = lo.min(q[0] + by);
                hi = hi.max(q[0] + by);
            }
        }
        let whole = ((range[0] - FORM_SLACK - lo) / FORM_TURN).ceil() * FORM_TURN;
        if !(hi + whole <= range[1] + FORM_SLACK) {
            return Err(unsupported("a loop that winds further round its axis than its face's form"));
        }
        for (c, by) in loop_pcs.iter_mut().zip(moves) {
            shift_u(c, by + whole);
        }
    }
    Ok(())
}

/// `c` moved by `by` along u.
fn shift_u(c: &mut Curve2, by: f64) {
    match c {
        Curve2::Spline { points, .. } => points.iter_mut().for_each(|p| p[0] += by),
        Curve2::RationalSpline { homogeneous, .. } => {
            for h in homogeneous {
                let w = h[2];
                h[0] += by * w;
            }
        }
    }
}
