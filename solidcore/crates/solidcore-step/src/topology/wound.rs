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
/// Cells of a use's parameter in which a crossing of a ray is looked for, and the bisections that
/// place it.
const CROSSING_CELLS: usize = 64;
const BISECTIONS: u32 = 60;

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
    let frame = form::frame_of(label)
        .ok_or_else(|| unsupported("a face that winds round a surface with no axis"))?;
    let (origin, x, y) = (
        Vec3::from(frame.origin),
        Vec3::from(frame.x_axis),
        Vec3::from(frame.y_axis),
    );
    let radius: Box<dyn Fn(f64) -> f64> = match label {
        BSurface::Cylinder { radius, .. } => {
            let r = *radius;
            Box::new(move |_| r)
        }
        BSurface::Cone {
            radius_at_origin,
            half_angle,
            ..
        } => {
            let (r, slope) = (*radius_at_origin, half_angle.tan());
            Box::new(move |h| r + h * slope)
        }
        _ => {
            return Err(unsupported(
                "a face that winds round a surface that is not a cylinder or a cone",
            ));
        }
    };
    let z = x.cross(y).normalized().ok_or("a degenerate frame")?;
    Ok(Axis {
        origin,
        x,
        y,
        z,
        radius,
    })
}

impl Axis {
    fn spot(&self, c: &CurveEval, t: f64) -> R<Spot> {
        let (p, d) = c.derivs(t).ok_or("an edge point")?;
        let r = p - self.origin;
        let (x, y) = (r.dot(self.x), r.dot(self.y));
        let round = x * x + y * y;
        if !(round > 1e-18) {
            return Err(unsupported(
                "an edge of a face that winds round its axis on the axis",
            ));
        }
        Ok(Spot {
            theta: y.atan2(x),
            height: r.dot(self.z),
            d_theta: (x * d.dot(self.y) - y * d.dot(self.x)) / round,
            d_height: d.dot(self.z),
        })
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
    (2.0 * s3 - 3.0 * s2 + 1.0) * p[0]
        + (s3 - 2.0 * s2 + s) * m[0]
        + (-2.0 * s3 + 3.0 * s2) * p[1]
        + (s3 - s2) * m[1]
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
        Ok(End {
            t,
            theta,
            u,
            v: s.height,
            du: scale * s.d_theta,
            dv: s.d_height,
            d_theta: s.d_theta,
        })
    }
}

/// The knots of a spline `curve` strictly inside `(a, b)` (none for any other curve).
fn knots_within(curve: &BCurve, a: f64, b: f64) -> Vec<f64> {
    match curve {
        BCurve::Spline { knots, .. } | BCurve::RationalSpline { knots, .. } => {
            knots.iter().copied().filter(|k| *k > a && *k < b).collect()
        }
        _ => Vec::new(),
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
    // ... and each knot span of the edge's curve its own pieces (a short span between two grid
    // points would otherwise lie wholly between a piece's probes).
    let mut stops: Vec<f64> = (1..=n).map(|i| a + (b - a) * i as f64 / n as f64).collect();
    if a < b {
        stops.extend(knots_within(curve, a, b));
        stops.sort_by(f64::total_cmp);
        stops.dedup_by(|x, y| (*x - *y).abs() <= 1e-12 * (b - a));
        if let Some(last) = stops.last_mut() {
            *last = b;
        }
    }
    if stops.len() > MAX_PIECES / 4 {
        return Err(unsupported(
            "an edge of a face that winds round its axis, whose curve has too many knot spans",
        ));
    }
    let mut ends = vec![first];
    for &t in &stops {
        let prev = ends.last().copied().ok_or("pcurve ends")?;
        let guess = prev.theta + 0.5 * (t - prev.t) * (prev.d_theta + ax.spot(&c, t)?.d_theta);
        ends.push(ax.end(&c, t, Some(guess))?);
    }
    let mut pieces: Vec<[End; 2]> = ends
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some([*a, *b]),
            _ => None,
        })
        .collect();
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
        let mut worst = if (q.theta - p.theta).abs() > FRAC_PI_8 {
            f64::INFINITY
        } else {
            0.0
        };
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
            return Err(unsupported(format!(
                "an edge of a face that winds round its axis, whose pcurve takes more than {MAX_PIECES} pieces"
            )));
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
/// close once unwrapped, or that runs further than the form does, is refused. The loop that runs
/// furthest (the outer) is put on the form's first turn; each other loop (a hole) goes on the turn
/// of the form where it is inside the outer, and is refused where that is no turn or more than one.
pub(super) fn unwind(
    edge_loops: &[Vec<(u32, bool)>],
    pcs: &mut [Vec<Curve2>],
    range: [f64; 2],
) -> R<()> {
    let at = |c: &Curve2, end: bool| -> R<[f64; 2]> {
        let [a, b] = c.interval().ok_or("pcurve interval")?;
        solidcore_ops::pcurve::eval(c, if end { b } else { a })
            .ok_or_else(|| "pcurve end".to_string())
    };
    // Each loop on its own: its uses' moves to run on from one another, and its extent in u.
    let mut moves_of = Vec::with_capacity(pcs.len());
    let mut extent_of = Vec::with_capacity(pcs.len());
    for (l, loop_pcs) in edge_loops.iter().zip(pcs.iter()) {
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
            return Err(unsupported(format!(
                "a loop that winds {:.1} turns about its axis and does not close",
                (p - first) / FORM_TURN
            )));
        }
        // The loop's extent in u, from the points each pcurve passes through.
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for (c, by) in loop_pcs.iter().zip(&moves) {
            let [a, b] = c.interval().ok_or("pcurve interval")?;
            for k in 0..=32 {
                let q = solidcore_ops::pcurve::eval(c, a + (b - a) * f64::from(k) / 32.0)
                    .ok_or("pcurve point")?;
                lo = lo.min(q[0] + by);
                hi = hi.max(q[0] + by);
            }
        }
        moves_of.push(moves);
        extent_of.push((lo, hi));
    }
    // The outer loop is the one that runs furthest in u (a hole lies inside it): it is moved to
    // the form's first turn. Each other loop is a closed curve that goes round no turn, so it
    // may lie on any turn of the form: it goes on the turn where it is inside the outer loop.
    let outer = (0..extent_of.len()).max_by(|&i, &j| {
        let width = |k: usize| extent_of.get(k).map_or(0.0, |(lo, hi)| hi - lo);
        width(i).total_cmp(&width(j)).then(j.cmp(&i))
    });
    let Some(outer) = outer else { return Ok(()) };
    let (lo, hi) = *extent_of.get(outer).ok_or("loop extent")?;
    let outer_whole = ((range[0] - FORM_SLACK - lo) / FORM_TURN).ceil() * FORM_TURN;
    if !(hi + outer_whole <= range[1] + FORM_SLACK) {
        return Err(unsupported(
            "a loop that winds further round its axis than its face's form",
        ));
    }
    let outer_moves: Vec<f64> = moves_of
        .get(outer)
        .ok_or("loop moves")?
        .iter()
        .map(|by| by + outer_whole)
        .collect();
    let mut whole_of = vec![outer_whole; pcs.len()];
    for k in (0..pcs.len()).filter(|&k| k != outer) {
        let (lo, hi) = *extent_of.get(k).ok_or("loop extent")?;
        let first = ((range[0] - FORM_SLACK - lo) / FORM_TURN).ceil() * FORM_TURN;
        if !(hi + first <= range[1] + FORM_SLACK) {
            return Err(unsupported(
                "a loop that winds further round its axis than its face's form",
            ));
        }
        // Where the loop's first use starts, on each turn the form has room for.
        let start = {
            let c = pcs.get(k).and_then(|l| l.first()).ok_or("loop pcurves")?;
            let by = moves_of
                .get(k)
                .and_then(|m| m.first())
                .ok_or("loop moves")?;
            let q = at(c, false)?;
            [q[0] + by, q[1]]
        };
        let outer_pcs = pcs.get(outer).ok_or("loop pcurves")?;
        let mut on = None;
        let mut turn = 0.0;
        while hi + first + turn <= range[1] + FORM_SLACK {
            if inside(outer_pcs, &outer_moves, [start[0] + first + turn, start[1]])? {
                if on.is_some() {
                    return Err(unsupported(
                        "a loop inside the face's outer loop on more than one turn of its form",
                    ));
                }
                on = Some(first + turn);
            }
            turn += FORM_TURN;
        }
        *whole_of.get_mut(k).ok_or("loop turn")? = on.ok_or_else(|| {
            unsupported("a loop that is inside the face's outer loop on no turn of its form")
        })?;
    }
    for ((loop_pcs, moves), whole) in pcs.iter_mut().zip(moves_of).zip(whole_of) {
        for (c, by) in loop_pcs.iter_mut().zip(moves) {
            shift_u(c, by + whole);
        }
    }
    Ok(())
}

/// Whether the point `q` of the form's (u, v) is inside the loop whose uses are `loop_pcs`, each
/// moved along u by the matching entry of `moves`: the parity of the loop's crossings of the ray
/// from `q` up the v direction. Each use is cut into [`CROSSING_CELLS`] cells of its parameter;
/// a cell whose ends lie either side of `q`'s u holds a crossing, which is bisected on the
/// pcurve itself for its v.
fn inside(loop_pcs: &[Curve2], moves: &[f64], q: [f64; 2]) -> R<bool> {
    let mut crossings = 0_usize;
    for (c, by) in loop_pcs.iter().zip(moves) {
        let [a, b] = c.interval().ok_or("pcurve interval")?;
        let point = |t: f64| solidcore_ops::pcurve::eval(c, t).ok_or("pcurve point");
        let mut t0 = a;
        let mut left = point(t0)?[0] + by < q[0];
        for i in 1..=CROSSING_CELLS {
            let t1 = if i == CROSSING_CELLS {
                b
            } else {
                a + (b - a) * i as f64 / CROSSING_CELLS as f64
            };
            let now = point(t1)?[0] + by < q[0];
            if now != left {
                let (mut lo, mut hi) = (t0, t1);
                for _ in 0..BISECTIONS {
                    let mid = 0.5 * (lo + hi);
                    if (point(mid)?[0] + by < q[0]) == left {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                if point(0.5 * (lo + hi))?[1] > q[1] {
                    crossings += 1;
                }
            }
            t0 = t1;
            left = now;
        }
    }
    Ok(crossings % 2 == 1)
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// A straight pcurve from `a` to `b` in the form's (u, v).
    fn line(a: [f64; 2], b: [f64; 2]) -> Curve2 {
        Curve2::Spline {
            knots: vec![0.0, 0.0, 1.0, 1.0],
            points: vec![a, b],
        }
    }

    fn polygon(corners: &[[f64; 2]]) -> Vec<Curve2> {
        (0..corners.len())
            .map(|i| line(corners[i], corners[(i + 1) % corners.len()]))
            .collect()
    }

    /// A two-turn strip (a thread's flank: the band between two helices, v rising by 1 a turn) with
    /// a hole in its second turn. The hole's pcurves come in the first turn's u, as an angle does;
    /// it goes where the strip is: at the height it has, the strip is there on the second turn
    /// only, not on the first.
    #[test]
    fn a_hole_goes_on_the_turn_of_the_strip_it_is_inside() {
        let strip = polygon(&[[0.0, 0.0], [8.0, 2.0], [8.0, 3.0], [0.0, 1.0]]);
        let hole = polygon(&[[0.5, 1.4], [0.75, 1.4], [0.75, 1.6], [0.5, 1.6]]);
        let loops = vec![
            vec![(0, true), (1, true), (2, true), (3, true)],
            vec![(4, true), (5, true), (6, true), (7, true)],
        ];
        let mut pcs = vec![strip, hole];
        unwind(&loops, &mut pcs, [0.0, 8.0]).unwrap();
        let start = solidcore_ops::pcurve::eval(&pcs[1][0], 0.0).unwrap();
        assert!(
            (start[0] - 4.5).abs() < 1e-12 && (start[1] - 1.4).abs() < 1e-12,
            "{start:?}"
        );
        let outer = solidcore_ops::pcurve::eval(&pcs[0][0], 0.0).unwrap();
        assert!(outer[0].abs() < 1e-12, "{outer:?}");
    }

    /// The same hole low in the strip is on its first turn.
    #[test]
    fn a_hole_low_in_the_strip_stays_on_the_first_turn() {
        let strip = polygon(&[[0.0, 0.0], [8.0, 2.0], [8.0, 3.0], [0.0, 1.0]]);
        let hole = polygon(&[[0.5, 0.4], [0.75, 0.4], [0.75, 0.6], [0.5, 0.6]]);
        let loops = vec![
            vec![(0, true), (1, true), (2, true), (3, true)],
            vec![(4, true), (5, true), (6, true), (7, true)],
        ];
        let mut pcs = vec![strip, hole];
        unwind(&loops, &mut pcs, [0.0, 8.0]).unwrap();
        let start = solidcore_ops::pcurve::eval(&pcs[1][0], 0.0).unwrap();
        assert!((start[0] - 0.5).abs() < 1e-12, "{start:?}");
    }
}
