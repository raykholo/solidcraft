//! Bodies another kernel holds in its own form ([`solidcraft_kernel_api::HeldBody`]), and the
//! count of every read of their record.
//!
//! A held body has no record until something reads it ([`crate::Body::record`], or the kernel's
//! own code reading the body's faces and edges): the first read exports the body
//! ([`solidcraft_kernel_api::HeldBody::export`]) and the record is kept. Every read is counted
//! by the line that asked ([`record_reads`]), the first one of each body as a conversion, so a
//! measurement shows which callers still need the record of a body another kernel holds. A
//! solidcore body is never held: it is its record, and reading it counts nothing.

use std::collections::BTreeMap;
use std::panic::Location;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use solidcore_record::brep::Brep;
use solidcraft_kernel_api::HeldBody;

/// A held body and its record once made ([`crate::body::RecordCell::Held`]).
pub(crate) struct HeldCell {
    pub(crate) body: Arc<dyn HeldBody>,
    /// The record once made, and why the body did not give one when it did not.
    made: OnceLock<(Arc<Brep>, Option<String>)>,
    reads: AtomicU64,
}

impl std::fmt::Debug for HeldCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldCell").field("body", &self.body).field("made", &self.made.get().is_some()).finish_non_exhaustive()
    }
}

impl HeldCell {
    pub(crate) fn new(body: Arc<dyn HeldBody>) -> HeldCell {
        HeldCell { body, made: OnceLock::new(), reads: AtomicU64::new(0) }
    }

    /// The record, exported the first time it is read; every read counted at `at`. A body that
    /// does not export (or panics exporting), or exports a record that fails the record check,
    /// reads as an empty record, and every operation on it is refused with why
    /// ([`HeldCell::failure`], [`crate::Body::require_brep`]): never taken as a body with nothing
    /// in it.
    pub(crate) fn record(&self, at: &'static Location<'static>) -> &Arc<Brep> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        note(at, false);
        &self
            .made
            .get_or_init(|| {
                note(at, true);
                let backend = self.body.backend();
                log::trace!("the record of a body {backend} holds is made, read at {}:{}", at.file(), at.line());
                let made = crate::guard("held body export", || {
                    let out = self.body.export().map_err(crate::KernelError::from)?;
                    crate::body::solid_record(out.record)
                });
                match made {
                    Ok(record) => (Arc::new(record), None),
                    Err(e) => {
                        let why = format!("the body {backend} holds gives no record ({e})");
                        log::error!("{why}");
                        (Arc::new(crate::body::empty_record()), Some(why))
                    }
                }
            })
            .0
    }

    /// Why the body gave no record, once its record was read and it did not.
    pub(crate) fn failure(&self) -> Option<&str> {
        self.made.get().and_then(|(_, why)| why.as_deref())
    }

    /// Whether the record was made.
    pub(crate) fn made(&self) -> bool {
        self.made.get().is_some()
    }

    /// How many times the record was read.
    pub(crate) fn reads(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }
}

/// A held body's triangles as the mesh a viewport draws: positions and edges as points, the
/// seams it names (every other edge drawn), no edge a tangent join or a protected crease.
/// Checked as a mesh from outside is: every index in range, one face per triangle, one normal
/// per position, the faces of every edge, finite points, and every face number below `faces`
/// when the kernel gives its face count; an error naming what does not hold.
pub(crate) fn mesh_of(m: solidcraft_kernel_api::HeldMesh, faces: Option<usize>) -> crate::Result<solidcraft_geom::Mesh> {
    let bad = |what: &str| Err(crate::KernelError::Invalid(format!("a held body's mesh: {what}")));
    // Checked and copied as the kernel gave it: no geometry is worked out here.
    let solidcraft_kernel_api::HeldMesh { positions, normals, triangles: corners, tri_face, edges, edge_faces, seams } = m;
    let n = positions.len();
    if normals.len() != n {
        return bad("not one normal per position");
    }
    if tri_face.len() != corners.len() {
        return bad("not one face per triangle");
    }
    if edge_faces.len() != edges.len() || (!seams.is_empty() && seams.len() != edges.len()) {
        return bad("not one entry per edge");
    }
    if corners.iter().flatten().any(|&i| i as usize >= n) {
        return bad("a triangle's corner past its positions");
    }
    if let Some(faces) = faces
        && tri_face.iter().chain(edge_faces.iter().flatten()).any(|&f| f as usize >= faces)
    {
        return bad("a face number past its faces");
    }
    let finite = |p: &[f64; 3]| p.iter().all(|v| v.is_finite());
    if !positions.iter().chain(&normals).chain(edges.iter().flatten()).all(finite) {
        return bad("a point that is not finite");
    }
    let point = |p: [f64; 3]| solidcore_record::Vec3::new(p[0], p[1], p[2]);
    let count = edges.len();
    let mut seams = seams;
    seams.resize(count, false);
    Ok(solidcraft_geom::Mesh {
        positions: positions.into_iter().map(point).collect(),
        normals: normals.into_iter().map(point).collect(),
        triangles: corners,
        tri_face,
        edges: edges.into_iter().map(|e| e.into_iter().map(point).collect()).collect(),
        display_only_edges: false,
        seams,
        smooth_edges: vec![false; count],
        sharp_edges: vec![false; count],
        edge_faces,
    })
}

/// Where the record of held bodies was read, and how often ([`record_reads`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordReads {
    /// Records made from held bodies: one for each body whose record was read.
    pub conversions: u64,
    /// Reads of a held body's record, a body's first included.
    pub reads: u64,
    /// By the line that read: `(file, line, reads, conversions)`, by file and line.
    pub sites: Vec<RecordReadSite>,
}

/// One line that read the record of held bodies ([`RecordReads::sites`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordReadSite {
    pub file: &'static str,
    pub line: u32,
    pub reads: u64,
    pub conversions: u64,
}

static SITES: Mutex<BTreeMap<(&'static str, u32), (u64, u64)>> = Mutex::new(BTreeMap::new());

/// Counts a read at `at`, or the conversion it made (`converts`).
fn note(at: &'static Location<'static>, converts: bool) {
    let mut sites = SITES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let site = sites.entry((at.file(), at.line())).or_default();
    if converts {
        site.1 += 1;
    } else {
        site.0 += 1;
    }
}

/// Every read of a held body's record so far in this process, by the line that read it: what
/// the record still stands in for when another kernel holds the body. Empty when every body is
/// solidcore's.
pub fn record_reads() -> RecordReads {
    let sites = SITES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let sites: Vec<RecordReadSite> =
        sites.iter().map(|(&(file, line), &(reads, conversions))| RecordReadSite { file, line, reads, conversions }).collect();
    RecordReads { conversions: sites.iter().map(|s| s.conversions).sum(), reads: sites.iter().map(|s| s.reads).sum(), sites }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Body;
    use solidcore_record::Vec3;
    use solidcraft_geom::prelude::*;
    use solidcraft_kernel_api::{BackendError, BackendId, Exported};

    /// A kernel that holds a solidcore record as its own form and gives it back when asked: the
    /// least a held body can be, so every read of its record is one the router counted.
    #[derive(Debug)]
    struct Loopback(Brep);

    impl HeldBody for Loopback {
        fn backend(&self) -> BackendId {
            BackendId("loopback")
        }
        fn export(&self) -> Result<Exported, BackendError> {
            Ok(Exported { record: self.0.clone(), history: None })
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    /// A kernel's box held in its own form, answering what a viewport asks (its box, its
    /// triangles, rays and points) from its six planes with no record, and giving solidcore's box
    /// record only when its record is read. Its faces and edges are numbered as that record's, so
    /// a pick names the face the record has.
    #[derive(Debug)]
    struct StubBox {
        lo: [f64; 3],
        hi: [f64; 3],
        record: Brep,
        /// Per record face: the axis it is square to, and whether it is the high one.
        faces: Vec<(usize, bool)>,
        /// Per record edge: its two faces.
        edges: Vec<[u32; 2]>,
    }

    /// A body [`StubBox`] holds, from `lo` to `hi`.
    fn held_stub_box(lo: solidcore_record::Vec3, hi: solidcore_record::Vec3) -> crate::Result<crate::Body> {
        let solid = crate::box_solid(lo, hi)?;
        let invalid = |what: &str| crate::KernelError::Invalid(format!("stub box: {what}"));
        let mut faces = Vec::new();
        for i in 0..solid.record().faces.len() {
            let plane = solid.planar_face(i).ok_or_else(|| invalid("a face is not flat"))?;
            let n = plane.x.cross(plane.y);
            let n = [n.x, n.y, n.z];
            let axis = (0..3).max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs())).unwrap_or(0);
            faces.push((axis, n[axis] > 0.0));
        }
        let mut edges = Vec::new();
        for e in &solid.record().edges {
            let f = crate::history::faces_of(e);
            let (Some(&a), Some(&b)) = (f.first(), f.get(1)) else { return Err(invalid("an edge has one face")) };
            edges.push([u32::try_from(a).map_err(|_| invalid("face"))?, u32::try_from(b).map_err(|_| invalid("face"))?]);
        }
        let held = StubBox { lo: [lo.x, lo.y, lo.z], hi: [hi.x, hi.y, hi.z], record: solid.record().clone(), faces, edges };
        Ok(crate::Body::held(Arc::new(held)))
    }

    impl StubBox {
        /// Face `f`'s plane: its axis, where it crosses it, and its outward normal.
        fn plane(&self, f: usize) -> Option<(usize, f64, [f64; 3])> {
            let &(axis, high) = self.faces.get(f)?;
            let mut n = [0.0; 3];
            n[axis] = if high { 1.0 } else { -1.0 };
            Some((axis, if high { self.hi[axis] } else { self.lo[axis] }, n))
        }
    }

    impl HeldBody for StubBox {
        fn backend(&self) -> solidcraft_kernel_api::BackendId {
            solidcraft_kernel_api::BackendId("stub")
        }
        fn export(&self) -> Result<solidcraft_kernel_api::Exported, solidcraft_kernel_api::BackendError> {
            Ok(solidcraft_kernel_api::Exported { record: self.record.clone(), history: None })
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn bounds(&self) -> Option<[[f64; 3]; 2]> {
            Some([self.lo, self.hi])
        }
        fn is_sheet(&self) -> Option<bool> {
            Some(false)
        }
        fn counts(&self) -> Option<(usize, usize)> {
            Some((self.faces.len(), self.edges.len()))
        }
        fn tessellate(
            &self,
            _chord: f64,
            _angle: Option<f64>,
        ) -> Option<Result<solidcraft_kernel_api::HeldMesh, solidcraft_kernel_api::BackendError>> {
            let mut m = solidcraft_kernel_api::HeldMesh::default();
            for f in 0..self.faces.len() {
                let Some((axis, at, n)) = self.plane(f) else { continue };
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let corner = |a: bool, b: bool| {
                    let mut p = [0.0; 3];
                    p[axis] = at;
                    p[u] = if a { self.hi[u] } else { self.lo[u] };
                    p[v] = if b { self.hi[v] } else { self.lo[v] };
                    p
                };
                let base = u32::try_from(m.positions.len()).unwrap_or(u32::MAX);
                m.positions.extend([corner(false, false), corner(true, false), corner(true, true), corner(false, true)]);
                m.normals.extend([n; 4]);
                // u x v is +axis: counter-clockwise seen from +axis, so turned for the low face.
                let (t0, t1) = if n[axis] > 0.0 { ([0, 1, 2], [0, 2, 3]) } else { ([0, 2, 1], [0, 3, 2]) };
                m.triangles.push(t0.map(|i| base + i));
                m.triangles.push(t1.map(|i| base + i));
                let face = u32::try_from(f).unwrap_or(u32::MAX);
                m.tri_face.extend([face, face]);
            }
            for pair in &self.edges {
                let (Some((a, at_a, _)), Some((b, at_b, _))) = (self.plane(pair[0] as usize), self.plane(pair[1] as usize)) else { continue };
                let c = 3 - a - b;
                let mut p = [0.0; 3];
                p[a] = at_a;
                p[b] = at_b;
                let (mut q, mut r) = (p, p);
                q[c] = self.lo[c];
                r[c] = self.hi[c];
                m.edges.push(vec![q, r]);
                m.edge_faces.push(pair.to_vec());
            }
            Some(Ok(m))
        }
        fn ray(
            &self,
            origin: [f64; 3],
            direction: [f64; 3],
        ) -> Option<Result<Vec<solidcraft_kernel_api::FaceHit>, solidcraft_kernel_api::BackendError>> {
            let len = direction.iter().map(|d| d * d).sum::<f64>().sqrt();
            let mut hits = Vec::new();
            for f in 0..self.faces.len() {
                let Some((axis, at, _)) = self.plane(f) else { continue };
                if direction[axis] == 0.0 {
                    continue;
                }
                let t = (at - origin[axis]) / direction[axis];
                let p: [f64; 3] = std::array::from_fn(|i| origin[i] + t * direction[i]);
                let inside = (0..3).filter(|&i| i != axis).all(|i| p[i] >= self.lo[i] - 1e-9 && p[i] <= self.hi[i] + 1e-9);
                if t >= 0.0 && inside {
                    hits.push(solidcraft_kernel_api::FaceHit { face: f, point: p, distance: t * len });
                }
            }
            hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
            Some(Ok(hits))
        }
        fn classify(&self, point: [f64; 3]) -> Option<Result<solidcraft_kernel_api::ClassifiedPoint, solidcraft_kernel_api::BackendError>> {
            const TOL: f64 = 1e-6;
            let within = (0..3).all(|i| point[i] >= self.lo[i] - TOL && point[i] <= self.hi[i] + TOL);
            if !within {
                return Some(Ok(solidcraft_kernel_api::ClassifiedPoint::Outside));
            }
            for f in 0..self.faces.len() {
                if let Some((axis, at, n)) = self.plane(f)
                    && (point[axis] - at).abs() <= TOL
                {
                    let normal = solidcraft_kernel_api::UnitNormal { value: n, bound: 0.0 };
                    return Some(Ok(solidcraft_kernel_api::ClassifiedPoint::On { face: f, normal }));
                }
            }
            Some(Ok(solidcraft_kernel_api::ClassifiedPoint::Inside))
        }
    }

    fn held_box() -> (Body, Body) {
        let solid = crate::box_solid(Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 20.0, 30.0)).unwrap();
        let held = Body::held(Arc::new(Loopback(solid.record().clone())));
        (solid, held)
    }

    #[test]
    fn a_held_body_has_no_record_until_one_is_read() {
        let (_, held) = held_box();
        assert_eq!(held.held_record_reads(), Some((false, 0)));
        assert!(held.held_body().is_some_and(|b| b.backend() == BackendId("loopback")));
        let clone = held.clone();
        assert!(clone.same(&held), "a clone is the same body");
        assert_eq!(held.held_record_reads(), Some((false, 0)), "cloning and comparing read nothing");
    }

    #[test]
    fn a_held_body_reads_as_the_record_it_exports_made_once() {
        let (solid, held) = held_box();
        let before = record_reads();
        assert_eq!(held.record(), solid.record());
        let line = line!() - 1;
        assert_eq!(held.record().faces.len(), 6);
        assert_eq!(held.held_record_reads(), Some((true, 2)), "made at the first read and kept");
        let after = record_reads();
        assert!(after.conversions > before.conversions);
        let here = after.sites.iter().find(|s| s.file == file!() && s.line == line);
        assert!(here.is_some_and(|s| s.conversions == 1 && s.reads == 1), "the read is counted where it was asked: {:?}", after.sites);
    }

    #[test]
    fn a_held_body_measures_as_its_record() {
        let (solid, held) = held_box();
        let (a, b) = (crate::measure_mass(&solid).unwrap(), crate::measure_mass(&held).unwrap());
        assert_eq!(a.volume.to_bits(), b.volume.to_bits());
        assert!(held.held_record_reads().is_some_and(|(made, _)| made));
    }

    #[test]
    fn a_solidcore_body_is_not_held_and_counts_nothing() {
        let (solid, _) = held_box();
        assert!(solid.held_body().is_none());
        assert_eq!(solid.held_record_reads(), None);
    }

    #[test]
    fn a_held_body_that_does_not_export_reads_as_empty() {
        #[derive(Debug)]
        struct Refuses;
        impl HeldBody for Refuses {
            fn backend(&self) -> BackendId {
                BackendId("refuses")
            }
            fn export(&self) -> Result<Exported, BackendError> {
                Err(BackendError::Exchange("no record".into()))
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
        let held = Body::held(Arc::new(Refuses));
        assert!(held.record().faces.is_empty());
        assert!(crate::measure_mass(&held).is_err());
        let refused = crate::boolean(&held, &held_box().0, crate::BoolOp::Union);
        assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("gives no record")), "{refused:?}");
    }

    /// A body another kernel holds is shown and picked from that kernel's own answers: its
    /// triangles, its box and size, a ray's faces and a point's side. Its record is
    /// never made, so nothing of solidcore runs on it.
    #[test]
    fn a_held_body_is_drawn_and_picked_with_no_record() {
        let held = held_stub_box(Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 20.0, 30.0)).unwrap();
        let tol = crate::display_tolerance(held.size());
        let mesh = held.mesh(&tol).unwrap();
        assert_eq!((mesh.triangles.len(), mesh.edges.len(), mesh.tri_face.len()), (12, 12, 12));
        assert!(held.mesh_plan(&tol).is_err(), "no solidcore plan: drawn from its triangles");
        assert!((held.size() - (10f64 * 10.0 + 400.0 + 900.0).sqrt()).abs() < 1e-12);
        assert_eq!(held.outline_bounds().max, Vec3::new(10.0, 20.0, 30.0));
        assert_eq!(held.exact_bounds().unwrap().min, Vec3::new(0.0, 0.0, 0.0));
        let top = held.raycast_face(Vec3::new(5.0, 5.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
        assert_eq!(top.0, 70.0);
        let hits = held.face_hits_by_ray([5.0, 5.0, 100.0], [0.0, 0.0, -1.0]).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[0].distance < hits[1].distance, "nearest first");
        assert_eq!(held.classify_point(Vec3::new(5.0, 5.0, 5.0)).unwrap(), crate::PointClass::Inside);
        assert_eq!(held.classify_point(Vec3::new(5.0, 5.0, 30.0)).unwrap(), crate::PointClass::OnBoundary);
        assert!(!held.contains(Vec3::new(50.0, 5.0, 5.0)));
        assert!(!held.is_surface());
        let _ = held.keeping_plans().keeping_meshes().mesh(&tol).unwrap();
        assert_eq!((held.face_count(), held.edge_count()), (6, 12));
        assert_eq!((held.unmeshed_faces(), held.unmeshed_display_faces()), (0, 0));
        assert_eq!(held.held_record_reads(), Some((false, 0)), "{:?}", record_reads().sites);
        // The face a pick names is the record's, once the record is made (to store the pick).
        let record_face = held.raycast_face(Vec3::new(5.0, 5.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap().1;
        let plane = held.planar_face(record_face).unwrap();
        assert_eq!(plane.x.cross(plane.y).z.signum(), 1.0, "the top face");
        assert!(held.held_record_reads().is_some_and(|(made, _)| made));
    }

    /// A held body's mesh that does not hold together is refused with why, never drawn.
    #[test]
    fn a_held_mesh_that_does_not_hold_together_is_refused() {
        let tri = solidcraft_kernel_api::HeldMesh {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            triangles: vec![[0, 1, 2]],
            tri_face: vec![0],
            ..Default::default()
        };
        assert!(mesh_of(tri.clone(), Some(1)).is_ok());
        let past = solidcraft_kernel_api::HeldMesh { triangles: vec![[0, 1, 3]], ..tri.clone() };
        assert!(mesh_of(past, None).is_err_and(|e| e.to_string().contains("past its positions")));
        let faces = solidcraft_kernel_api::HeldMesh { tri_face: vec![], ..tri.clone() };
        assert!(mesh_of(faces, None).is_err());
        let edge = solidcraft_kernel_api::HeldMesh { edges: vec![vec![[0.0; 3], [1.0, 0.0, 0.0]]], edge_faces: vec![vec![0, 4]], ..tri.clone() };
        assert!(mesh_of(edge.clone(), None).is_ok());
        assert!(mesh_of(edge, Some(1)).is_err_and(|e| e.to_string().contains("past its faces")));
        let nan = solidcraft_kernel_api::HeldMesh { positions: vec![[f64::NAN; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], ..tri };
        assert!(mesh_of(nan, None).is_err());
    }
}
