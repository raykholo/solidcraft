//! Contract for Surface Fillet (`surface.fillet`) in `cmd/surface.rs` (#769). It replies with the
//! plain feature reply, [`super::features::FeatureResult`].
//!
//! Whether the bodies exist and are surfaces, the points lie on their faces, the surfaces meet and
//! the radius evaluates are the handler's to check.

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::check::whole;
use super::types::{AngleExpr, FeatureCommon, LengthExpr, Point3, feature_common};

/// One of the two faces to round between: a surface body and a point on its (one) face.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct FilletFace {
    /// The surface body, by name.
    #[serde(deserialize_with = "body")]
    pub body: String,
    /// A point [x, y, z] in mm on the face. It also picks the side of the other surface that is
    /// kept: the side the rolling ball (or the bevel) stays on. Pick it farther from the seam than
    /// the round or bevel reaches: a point the round or bevel would cut away is refused.
    pub point: Point3,
}

/// The handler's message for `body` when it isn't text.
fn body<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    match Value::deserialize(d)? {
        Value::String(s) => Ok(s),
        _ => Err(D::Error::custom(whole("`body` is required"))),
    }
}

/// Round the seam between two surface bodies: each is cut where the other meets it, keeping the
/// side its point is on; the kept pieces are sewn into `a`'s body (`b`'s is then gone) and the
/// seam is rounded with a fillet of `radius`. Each body is one face. The two surfaces must meet
/// along one seam (one edge once cut): surfaces that meet along more than one seam are refused.
/// One undo step.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SurfaceFilletParams {
    /// One of the two faces. Its body keeps the result with `trim` both or first.
    pub a: FilletFace,
    /// The other of the two faces.
    pub b: FilletFace,
    /// The radius of the rolling ball, a length (> 0).
    pub radius: LengthExpr,
    /// What is left of the two surfaces. `both` (the default): each is cut back to the round and
    /// sewn to it, one body under `a`'s name (`b`'s body is gone). `first`: only `a` is cut back
    /// and sewn to the round; `b` is left as it was. `none`: neither changes, and the round alone
    /// is a new surface body.
    #[serde(default)]
    pub trim: Option<FilletTrimParam>,
    /// Default false. True extends a surface that stops short of the other so that they meet;
    /// with false such surfaces are refused.
    #[serde(default)]
    pub extend: Option<bool>,
    #[serde(flatten, deserialize_with = "feature_common")]
    pub common: FeatureCommon,
}

/// What Surface Fillet leaves of the two surfaces, as the handler spells it (any case).
#[derive(Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FilletTrimParam {
    Both,
    First,
    None,
}

impl<'de> Deserialize<'de> for FilletTrimParam {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::String(s) => match s.to_ascii_lowercase().as_str() {
                "both" => Ok(FilletTrimParam::Both),
                "first" => Ok(FilletTrimParam::First),
                "none" => Ok(FilletTrimParam::None),
                _ => Err(D::Error::custom(whole("`trim` must be both, first or none"))),
            },
            _ => Err(D::Error::custom(whole("`trim` must be both, first or none"))),
        }
    }
}

/// Bevel the seam between two surface bodies (`surface.chamfer`, #769): each is cut where the
/// other meets it, keeping the side its point is on; the kept pieces are sewn into `a`'s body
/// (`b`'s is then gone, with `trim` both) and the seam is bevelled `distance` back along each
/// surface, or `distance` along the reference surface and `distance2` along the other, or
/// `distance` along the reference surface at `angle` to it. The reference surface is not `a` by
/// default: it is whichever of the two faces has its normal, at the middle of the seam on the
/// sewn body, pointing most upward (+Z); `flip` takes the one pointing most downward (-Z)
/// instead. Each body is one face. The two surfaces must meet along one seam (one edge once
/// cut): surfaces that meet along more than one seam are refused. One undo step.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SurfaceChamferParams {
    /// One of the two faces (not necessarily the reference surface: see `flip`). Its body keeps
    /// the result with `trim` both or first.
    pub a: FilletFace,
    /// The other of the two faces.
    pub b: FilletFace,
    /// A length (> 0). Alone: how far back along each of the two surfaces the bevel reaches.
    /// With `distance2` or `angle`: how far back along the reference surface (the one facing most
    /// upward, +Z; most downward with `flip`).
    pub distance: LengthExpr,
    /// A length (> 0): how far back along the other surface (not the reference surface) the
    /// bevel reaches. Not with `angle`.
    #[serde(default)]
    pub distance2: Option<LengthExpr>,
    /// The angle the bevel makes with the reference surface; `distance` is along the reference
    /// surface and the reach along the other follows from the angle. Strictly between 0 and 180 degrees (checked when the bevel is built).
    /// Not with `distance2`.
    #[serde(default)]
    pub angle: Option<AngleExpr>,
    /// Default false: the reference surface (for `distance` with `distance2` or `angle`) is the
    /// face whose normal at the middle of the seam points most upward (+Z). True: the face
    /// pointing most downward (-Z). Where both lean alike, either way takes the same face. No
    /// effect on an equal bevel (`distance` alone). It does not change what `a`, `b` or `trim`
    /// first mean.
    #[serde(default)]
    pub flip: Option<bool>,
    /// What is left of the two surfaces, as for `surface.fillet`. Default: both. `first` always
    /// means `a`, whichever surface is the reference.
    #[serde(default)]
    pub trim: Option<FilletTrimParam>,
    /// Default false. True extends a surface that stops short of the other so that they meet;
    /// with false such surfaces are refused.
    #[serde(default)]
    pub extend: Option<bool>,
    #[serde(flatten, deserialize_with = "feature_common")]
    pub common: FeatureCommon,
}
