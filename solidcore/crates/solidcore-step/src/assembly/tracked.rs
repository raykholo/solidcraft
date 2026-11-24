//! A pcurve made by following the edge across its spline face, where the kernel's trims are far
//! off it.
//!
//! The kernel makes a spline face's trims by dropping each edge onto the surface. Where a cut
//! runs along a plane that stands (all but) tangent to the surface, as the flat end of a coil
//! spring wire that is cut where the wire's tube meets the plane of the turn above or below it,
//! the drop settles on another part of the surface for stretches of the edge, and the pcurve it
//! returns runs several millimetres off its edge (#20575). The edge is on the surface all the
//! same, and a nearest point found from where the edge was a moment before follows it: each point
//! of the edge is projected from the parameters the pcurve so far predicts, which stays on the
//! part of the surface the edge is on even where another part of it passes within a few
//! hundredths of a millimetre.
//!
//! Only a use whose pcurve lies [`FAR`] off its edge is made again, and only when the followed
//! pcurve lies nearer it. Its ends are the kernel's, so it meets its neighbours where the kernel's
//! did. Each piece is a cubic Hermite spline through the followed parameters and the slopes the
//! surface's derivatives give (the edge's velocity taken into parameters by least squares),
//! halved until its middle lies no farther from the edge than the nearest point of the surface
//! does, within [`FIT`] (or than its ends do: the kernel's ends may lie a little off the edge).

use crate::kernel::{CurveEval, SurfaceEval};
use crate::vec::Vec3Geom as _;
use solidcore_ops::pcurve;
use solidcore_record::Vec3;
use solidcore_record::brep::{Curve2, Surface as BSurface};

use super::{EDGE_GAP_SAMPLES, EdgeB, MAX_SPLINE_POINTS, R};

/// How far (mm) a pcurve's surface lies off its edge before it is made again.
const FAR: f64 = 0.1;
/// How much farther (mm) than the surface's nearest point to the edge a piece's middle may lie.
const FIT: f64 = 1e-3;
/// The pieces the edge is first followed in.
const FIRST_PIECES: usize = 64;
/// The most nodes a followed pcurve has, and the most times a piece is halved.
const MAX_NODES: usize = 1024;
const MAX_DEPTH: u32 = 10;
/// The farthest (mm) the followed pcurve's end may lie from the kernel's.
const END_AGREE: f64 = 0.1;
/// How far (mm) a followed point may lie from where the pcurve so far predicts it: this much of
/// the distance from the last point of the edge, and this much more.
const REACH_SHARE: f64 = 0.5;
const REACH_MORE: f64 = 0.05;
/// The most steps a nearest point takes.
const NEWTON_STEPS: usize = 40;
/// The longest step (in the surface's parameters) a nearest point takes.
const LONGEST_STEP: f64 = 0.5;

/// A point of the followed pcurve: the edge's parameter, its parameters on the surface, the
/// slope of those in the edge's parameter, and how far (mm) the surface at them lies off the edge.
#[derive(Clone, Copy)]
struct Node {
    t: f64,
    q: [f64; 2],
    m: [f64; 2],
    off: f64,
}

/// How far the surface at `pc` lies off the edge, over the samples the record check takes.
fn gap(on: &SurfaceEval, curve: &CurveEval, pc: &Curve2, [t0, t1]: [f64; 2]) -> f64 {
    let mut worst = 0.0f64;
    for k in 0..=EDGE_GAP_SAMPLES {
        let t = if k == EDGE_GAP_SAMPLES {
            t1
        } else {
            t0 + (t1 - t0) * (k as f64 / EDGE_GAP_SAMPLES as f64)
        };
        if let (Some(q), Some(p)) = (pcurve::eval(pc, t), curve.point(t))
            && let Some(s) = on.point(q)
        {
            worst = worst.max(s.dist(p));
        }
    }
    worst
}

/// The nearest point of the surface to `p` found from the parameters `from` by Gauss-Newton steps
/// (each at most [`LONGEST_STEP`] long, kept in the surface's box), and how far it lies from `p`:
/// the one nearest `from`, not the nearest of the whole surface.
fn near(on: &SurfaceEval, p: Vec3, from: [f64; 2]) -> Option<([f64; 2], f64)> {
    let domain = on.domain();
    let mut q = from;
    for _ in 0..NEWTON_STEPS {
        let (s, su, sv) = on.derivs(q[0], q[1])?;
        let r = p - s;
        let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
        let det = a * c - b * b;
        if !(det > 1e-12 * a * c) {
            return None;
        }
        let (g0, g1) = (su.dot(r), sv.dot(r));
        let mut step = [(c * g0 - b * g1) / det, (a * g1 - b * g0) / det];
        let length = step[0].hypot(step[1]);
        if !length.is_finite() {
            return None;
        }
        if length > LONGEST_STEP {
            step = [
                step[0] * LONGEST_STEP / length,
                step[1] * LONGEST_STEP / length,
            ];
        }
        let mut next = [q[0] + step[0], q[1] + step[1]];
        for (x, range) in next.iter_mut().zip(domain.range) {
            if let Some((lo, hi)) = range {
                *x = x.clamp(lo, hi);
            }
        }
        let moved = (next[0] - q[0]).hypot(next[1] - q[1]);
        q = next;
        if moved < 1e-12 {
            break;
        }
    }
    let s = on.point(q)?;
    Some((q, s.dist(p)))
}

/// The slope in the edge's parameter, at `t`, of the parameters `q` of its points.
fn slope(on: &SurfaceEval, curve: &CurveEval, t: f64, q: [f64; 2]) -> Option<[f64; 2]> {
    let (_, d) = curve.derivs(t)?;
    let (_, su, sv) = on.derivs(q[0], q[1])?;
    let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
    let det = a * c - b * b;
    if !(det > 1e-12 * a * c) {
        return None;
    }
    let (g0, g1) = (su.dot(d), sv.dot(d));
    let m = [(c * g0 - b * g1) / det, (a * g1 - b * g0) / det];
    (m[0].is_finite() && m[1].is_finite()).then_some(m)
}

/// The cubic Hermite value at `s` in `[0, 1]` from values `p` and slopes `m` (per unit of `s`).
fn hermite(p: [f64; 2], m: [f64; 2], s: f64) -> f64 {
    let (s2, s3) = (s * s, s * s * s);
    (2.0 * s3 - 3.0 * s2 + 1.0) * p[0]
        + (s3 - 2.0 * s2 + s) * m[0]
        + (-2.0 * s3 + 3.0 * s2) * p[1]
        + (s3 - s2) * m[1]
}

/// The cubic Hermite parameters at `s` between nodes `a` and `b`.
fn between(a: &Node, b: &Node, s: f64) -> [f64; 2] {
    let h = b.t - a.t;
    [
        hermite([a.q[0], b.q[0]], [h * a.m[0], h * b.m[0]], s),
        hermite([a.q[1], b.q[1]], [h * a.m[1], h * b.m[1]], s),
    ]
}

/// The surface and the edge the pcurve is followed along.
struct Walk<'s> {
    on: &'s SurfaceEval,
    curve: &'s CurveEval,
}

impl Walk<'_> {
    /// The node at `t`, found from the parameters `hint`, which its point must lie within `reach`
    /// mm of on the surface.
    fn node(&self, t: f64, hint: [f64; 2], reach: f64) -> Option<Node> {
        let p = self.curve.point(t)?;
        let (q, off) = near(self.on, p, hint)?;
        if self.on.point(q)?.dist(self.on.point(hint)?) > reach {
            eprintln!("DBG node reach t={t} hint={hint:?} q={q:?} off={off} reach={reach}");
            return None;
        }
        let m = slope(self.on, self.curve, t, q)?;
        Some(Node { t, q, m, off })
    }

    /// The nodes after `a` up to and with `b`, halving until each piece fits.
    fn pieces(&self, a: Node, b: Node, depth: u32, out: &mut Vec<Node>) -> Option<()> {
        let h = b.t - a.t;
        let mut fits = true;
        for s in [0.25, 0.5, 0.75] {
            let t = a.t + s * h;
            let hq = between(&a, &b, s);
            let p = self.curve.point(t)?;
            let on = self.on.point(hq)?.dist(p);
            let nearest = near(self.on, p, hq).map_or(on, |(_, d)| d);
            if on > nearest.max(a.off).max(b.off) + FIT {
                fits = false;
                break;
            }
        }
        if fits {
            out.push(b);
            return Some(());
        }
        if depth >= MAX_DEPTH || out.len() >= MAX_NODES {
            eprintln!("DBG pieces depth t={}..{}", a.t, b.t);
            return None;
        }
        let mid = a.t + 0.5 * h;
        let reach = REACH_SHARE * self.curve.point(a.t)?.dist(self.curve.point(b.t)?) + REACH_MORE;
        let node = self.node(mid, between(&a, &b, 0.5), reach)?;
        self.pieces(a, node, depth + 1, out)?;
        self.pieces(node, b, depth + 1, out)
    }
}

/// The pcurve of the edge on `curve` over `interval`, followed along the surface from the start
/// of `seed` (the kernel's), or `None` where the edge cannot be followed or ends elsewhere than
/// `seed` does.
fn followed(
    on: &SurfaceEval,
    curve: &CurveEval,
    interval: [f64; 2],
    seed: &Curve2,
    backward: bool,
) -> Option<Curve2> {
    let [t0, t1] = interval;
    if !(t1 > t0) {
        return None;
    }
    let walk = Walk { on, curve };
    // The edge is followed from the end of the kernel's pcurve that is on the right part of the
    // surface: tried from its start, then from its end (the start may lie on the wrong one of two
    // sheets a few hundredths of a millimetre apart).
    let (anchor_t, far_t, sign) = if backward { (t1, t0, -1.0) } else { (t0, t1, 1.0) };
    let q0 = pcurve::eval(seed, anchor_t)?;
    eprintln!("DBG followed backward={backward} interval={interval:?} anchor q0={q0:?}");
    let first = Node {
        t: anchor_t,
        q: q0,
        m: slope(on, curve, anchor_t, q0)?,
        off: on.point(q0)?.dist(curve.point(anchor_t)?),
    };
    let mut coarse = vec![first];
    let mut prev = first;
    for i in 1..=FIRST_PIECES {
        let t = if i == FIRST_PIECES {
            far_t
        } else {
            anchor_t + sign * (t1 - t0) * (i as f64 / FIRST_PIECES as f64)
        };
        let h = t - prev.t;
        let hint = [prev.q[0] + h * prev.m[0], prev.q[1] + h * prev.m[1]];
        let reach = REACH_SHARE * curve.point(prev.t)?.dist(curve.point(t)?) + REACH_MORE;
        let Some(node) = walk.node(t, hint, reach) else {
            eprintln!("DBG coarse fail i={i} t={t}");
            return None;
        };
        prev = node;
        coarse.push(node);
    }
    if backward {
        coarse.reverse();
    }
    // The far end (not the anchor) is kept where it was followed to when walking backward.
    let first = *coarse.first()?;
    if !backward {
        // It ends where the kernel's pcurve does.
        let mut last = *coarse.last()?;
        let end = pcurve::eval(seed, t1)?;
        if on.point(end)?.dist(on.point(last.q)?) > END_AGREE {
            return None;
        }
        last.q = end;
        last.m = slope(on, curve, t1, end)?;
        last.off = on.point(end)?.dist(curve.point(t1)?);
        if let Some(slot) = coarse.last_mut() {
            *slot = last;
        }
    }
    let mut nodes = vec![first];
    for pair in coarse.windows(2) {
        let (a, b) = (*pair.first()?, *pair.get(1)?);
        walk.pieces(a, b, 0, &mut nodes)?;
    }
    // A cubic Hermite piece from each pair of nodes, as a spline through them.
    let mut knots = vec![t0; 4];
    let mut points = vec![first.q];
    for (k, pair) in nodes.windows(2).enumerate() {
        let (p, q) = (*pair.first()?, *pair.get(1)?);
        let h = q.t - p.t;
        points.push([p.q[0] + h * p.m[0] / 3.0, p.q[1] + h * p.m[1] / 3.0]);
        points.push([q.q[0] - h * q.m[0] / 3.0, q.q[1] - h * q.m[1] / 3.0]);
        if k + 2 < nodes.len() {
            knots.extend([q.t; 2]);
        } else {
            points.push(q.q);
        }
    }
    knots.extend([t1; 4]);
    if points.len() > MAX_SPLINE_POINTS || points.iter().flatten().any(|x| !x.is_finite()) {
        return None;
    }
    Some(Curve2::Spline { knots, points })
}

/// The pcurves of a spline face's uses (`pcs`, in the order of `edge_loops`) that lie [`FAR`] off
/// their edges, made again where following the edge brings them nearer it: how many were.
///
/// # Errors
/// An edge or surface that cannot be evaluated.
pub(super) fn retrack(
    surface: &BSurface,
    edges: &[EdgeB],
    edge_loops: &[Vec<(u32, bool)>],
    pcs: &mut [Vec<Curve2>],
) -> R<usize> {
    let on = SurfaceEval::new(surface).map_err(|m| m.to_string())?;
    let mut made = 0;
    for (l, loop_pcs) in edge_loops.iter().zip(pcs.iter_mut()) {
        for (&(e, _), pc) in l.iter().zip(loop_pcs.iter_mut()) {
            let edge = edges.get(e as usize).ok_or("edge reference")?;
            let (curve, interval) = match &edge.form {
                Some((c, i)) => (c, *i),
                None => (&edge.curve, edge.interval),
            };
            let eval = CurveEval::new(curve).map_err(|m| m.to_string())?;
            let off = gap(&on, &eval, pc, interval);
            eprintln!("DBG retrack edge={e} off={off}");
            if !(off > FAR) {
                continue;
            }
            let mut best: Option<(f64, Curve2)> = None;
            for backward in [false, true] {
                let r = followed(&on, &eval, interval, pc, backward);
                eprintln!("DBG result backward={backward} some={}", r.is_some());
                if let Some(again) = r {
                    let g = gap(&on, &eval, &again, interval);
                    eprintln!("DBG gap {g}");
                    if g < off && best.as_ref().is_none_or(|(b, _)| g < *b) {
                        best = Some((g, again));
                    }
                }
            }
            if let Some((_, again)) = best {
                *pc = again;
                made += 1;
            }
        }
    }
    Ok(made)
}
