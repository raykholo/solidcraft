//! An extrude that ends To Object or starts From Object may save `picked_placements` in its
//! `extent`: where each other component stood, relative to the extrude's own, when the object
//! was picked (`[{component, placement}]`, a 4x4 matrix each). The extrude measures against the
//! model as it was then, so a joint that slides a component does not stretch a body built to
//! another component's face (#16732). Older builds measured against the placements as shown, so
//! a drawer that ended at its frame grew with every slide. A design saved that way gets the
//! placements its file holds as it opens, which are where the extrude was last built, so it
//! opens with the bodies it had and no longer stretches. The last release opens a design that has
//! the field with a newer-format warning, ignores it, and measures against the placements shown.

use serde_json::Value;

use crate::{Mat, Occurrence, Result};

pub fn step(v: &mut Value) -> Result<()> {
    let occurrences: Vec<Occurrence> = match v.get("occurrences").cloned().map(serde_json::from_value) {
        Some(Ok(occurrences)) => occurrences,
        _ => return Ok(()),
    };
    if occurrences.is_empty() {
        return Ok(());
    }
    // Each component's world placement: its first occurrence, through its parents.
    let world = |component: u64| -> Mat { crate::assembly::transform_through(component, |c| occurrences.iter().find(|o| o.component == c)) };
    let mut components: Vec<u64> = occurrences.iter().map(|o| o.component).collect();
    components.push(0);
    components.sort_unstable();
    components.dedup();
    for feature in v.get_mut("features").and_then(Value::as_array_mut).into_iter().flatten() {
        if feature.get("type").and_then(Value::as_str) != Some("extrude") {
            continue;
        }
        let own = feature.get("component").and_then(Value::as_u64).unwrap_or(0);
        let Some(inverse) = crate::mat_inverse(&world(own)) else { continue };
        let Some(extent) = feature.get_mut("extent").and_then(Value::as_object_mut) else { continue };
        let picks = ["to", "from"].iter().any(|key| extent.get(*key).is_some_and(|p| !p.is_null()));
        if !picks || extent.contains_key("picked_placements") {
            continue;
        }
        let pins: Vec<Value> = components
            .iter()
            .filter(|c| **c != own)
            .map(|c| serde_json::json!({"component": c, "placement": crate::mat_mul(&inverse, &world(*c))}))
            .collect();
        extent.insert("picked_placements".into(), Value::Array(pins));
    }
    Ok(())
}
