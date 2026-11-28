//! A body held in its own kernel's form ([`HeldBody`]), the record made from it only when
//! something asks for the record.
//!
//! A kernel's result can stay in that kernel: the router keeps it behind the body handle, and
//! the record (`solidcore_record::brep::Brep`) is made from it ([`HeldBody::export`]) the first
//! time a caller reads the record. Every such read is counted by the router, by where it was
//! asked from, so a measurement shows which callers still need the record of a body that
//! another kernel holds.
//!
//! What a viewport needs of a body, the kernel answers itself on its own form: its box, its
//! triangles ([`HeldBody::tessellate`]), the faces a ray meets and which side of the body a
//! point is ([`HeldBody::ray`], [`HeldBody::classify`]). Each answer is optional: a kernel
//! that gives `None` has the router read the record and answer as for a solidcore body. Faces
//! and edges are numbered in every answer as in the record the body exports, so a pick names
//! the face the record has, should the record be made later (to store the pick, say).

use crate::{BackendError, BackendId, ClassifiedPoint, Exported, FaceHit};

/// A held body's triangles ([`HeldBody::tessellate`]), faces and edges numbered as in the
/// record it exports: what a viewport draws and picks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeldMesh {
    /// Vertex positions.
    pub positions: Vec<[f64; 3]>,
    /// Vertex normals, one per position (unit, pointing out of the material).
    pub normals: Vec<[f64; 3]>,
    /// Triangles as vertex index triples, counter-clockwise seen from outside.
    pub triangles: Vec<[u32; 3]>,
    /// The face each triangle lies on.
    pub tri_face: Vec<u32>,
    /// Each edge as a polyline.
    pub edges: Vec<Vec<[f64; 3]>>,
    /// Per edge, the faces it bounds (one or two).
    pub edge_faces: Vec<Vec<u32>>,
    /// Per edge: a seam between two pieces of one smooth surface, not drawn. Empty: none is.
    pub seams: Vec<bool>,
}

/// A body a kernel holds in its own form, object-safe so the router can keep any kernel's body
/// behind one handle.
pub trait HeldBody: Send + Sync + std::fmt::Debug + 'static {
    /// The kernel holding the body.
    fn backend(&self) -> BackendId;

    /// The body as a record in canonical order, with the history that made it. Called once per
    /// body at most: the router keeps what it gives.
    fn export(&self) -> Result<Exported, BackendError>;

    /// The body's own form, for the kernel's adapter to find its own type again.
    fn as_any(&self) -> &dyn std::any::Any;

    /// The body's tight box, `[min, max]`: the exact box of its faces, edges and vertices, as
    /// an exact measure of the body gives it (a box worked out from control points is not
    /// tight). `None`: the router measures the record's.
    fn bounds(&self) -> Option<[[f64; 3]; 2]> {
        None
    }

    /// How many faces and edges the body has, numbered as in its record. `None`: the router
    /// counts the record's.
    fn counts(&self) -> Option<(usize, usize)> {
        None
    }

    /// Whether the body is a sheet: an edge of it bounds one face only. `None`: the router reads
    /// the record's edges.
    fn is_sheet(&self) -> Option<bool> {
        None
    }

    /// The body's triangles, no facet standing off its surface by more than `chord` (mm), nor
    /// turning more than `angle` (radians) where one is given. `None`: the router meshes the
    /// record.
    fn tessellate(&self, _chord: f64, _angle: Option<f64>) -> Option<Result<HeldMesh, BackendError>> {
        None
    }

    /// The faces the ray from `origin` along `direction` meets, nearest first. `None`: the
    /// router asks the record.
    fn ray(&self, _origin: [f64; 3], _direction: [f64; 3]) -> Option<Result<Vec<FaceHit>, BackendError>> {
        None
    }

    /// Where `point` is: in the material, outside it, or on a face within the body's
    /// tolerance. `None`: the router asks the record.
    fn classify(&self, _point: [f64; 3]) -> Option<Result<ClassifiedPoint, BackendError>> {
        None
    }
}
