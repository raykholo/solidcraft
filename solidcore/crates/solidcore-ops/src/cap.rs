//! Whether a face of a body is a prism's planar cap: the kernel's
//! [`solidcore::geom::plane_height::is_square_plane_at`] on records' numbers. Ops converts.

use solidcore::geom::plane_height::is_square_plane_at;
use solidcore::geom::vector3d::Vec3;

fn vec(p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

/// Whether the plane through `face_origin` with unit normal `face_normal` is square to the
/// unit direction `normal` and lies at `height` along it from `origin`, within `tol` (the
/// kernel's [`is_square_plane_at`]). The application sequences it over a body's planar faces.
#[must_use]
pub fn is_square_plane_at_height(
    face_origin: [f64; 3],
    face_normal: [f64; 3],
    origin: [f64; 3],
    normal: [f64; 3],
    height: f64,
    tol: f64,
) -> bool {
    is_square_plane_at(
        vec(face_origin),
        vec(face_normal),
        vec(origin),
        vec(normal),
        height,
        tol,
    )
}
