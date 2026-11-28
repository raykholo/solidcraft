//! A through-all join stops at the model's far corner along the extrude direction, not past it.
//! The extent saves `through_all_to_corner: true` for such an extrude. A design saved
//! without it keeps the older reach, the far corner times 1.02 plus 1 mm, so it rebuilds with
//! the same volume. No step is needed: older designs omit the field. The last release opens a
//! design that has the field with a newer-format warning, ignores it, and reads the older reach.
