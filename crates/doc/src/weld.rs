//! Saved weld graphics: manufacturing settings and optional symbols, never features or bodies.
use serde::{Deserialize, Serialize};
use solidcraft_geom::weld::{MAX_DIMENSION_MM, MIN_DIMENSION_MM, Weld};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_WELDS: usize = 1000;
pub const MAX_PICKS: usize = 256;
pub const TYPES: &[&str] = &["fillet", "butt", "edge", "plug", "spot", "seam", "stud", "surfacing", "tack"];
pub const PICK_KINDS: &[&str] = &["line", "groove", "point", "hole", "ring", "area"];
pub const SIZE_KEYS: &[&str] = &["a", "z", "z1", "z2", "h", "s", "d", "w", "t", "l", "e", "n"];
pub const SYMBOL_STANDARDS: &[&str] = &["iso2553_2019_a", "iso2553_2019_b", "aws_a2_4_2020"];
pub const GROOVES: &[&str] = &["square", "v", "bevel", "u", "j", "y", "hy", "x", "k"];
/// ISO 6947:2019 main welding positions.
pub const POSITIONS: &[&str] = &["PA", "PB", "PC", "PD", "PE", "PF", "PG", "PH", "PJ", "PK"];
/// Quality-level standards, each with levels B, C and D; saved as e.g. `ISO 10042-C`.
pub const QUALITY_STANDARDS: &[&str] = &["ISO 5817", "ISO 10042", "ISO 13919-1", "ISO 13919-2"];
pub const NDT_METHODS: &[&str] = &["VT", "PT", "MT", "UT", "RT"];
/// A label shown in place of W<n>.
pub const MAX_LABEL_BYTES: usize = 16;
const MAX_NUMBER: u32 = 1_000_000;
/// Size names belonging to the chosen type.
pub fn size_keys(weld_type: &str) -> &'static [&'static str] {
    match weld_type {
        "fillet" => &["a", "z", "z1", "z2", "s"],
        "butt" => &["h", "s"],
        "edge" => &["w"],
        "seam" => &["w", "l"],
        "plug" => &["h", "e"],
        "spot" | "stud" => &["d", "e"],
        "surfacing" => &["t", "w"],
        "tack" => &["l", "e", "n"],
        _ => &[],
    }
}
/// Intermittent elements; pitch is measured centre to centre in millimetres.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeldSpans {
    pub count: u32,
    pub length_mm: f64,
    pub pitch_mm: f64,
}
impl WeldSpans {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=10000).contains(&self.count) || ![self.length_mm, self.pitch_mm].into_iter().all(|v| v.is_finite() && v > 0.) {
            return Err("spans need count 1..=10000 and positive finite length and pitch".into());
        }
        Ok(())
    }
}
/// Fixed points or exact topology references in the body/index form used by other commands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeldPick {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}
impl WeldPick {
    pub fn validate(&self) -> Result<(), String> {
        let points = [self.point, self.edge, self.face];
        if points.iter().flatten().count() > 1 || points.iter().flatten().flatten().any(|v| !v.is_finite() || v.abs() >= 1e9) {
            return Err("a pick accepts at most one finite point, edge or face coordinate".into());
        }
        if self.point.is_some() {
            if self.body.is_some() || self.index.is_some() {
                return Err("a fixed point has no body or index".into());
            }
        } else if self.body.as_ref().is_none_or(|b| b.trim().is_empty() || b.len() > 1024) || self.index.is_none_or(|i| i > 1_000_000) {
            return Err("an edge or face pick needs body and index".into());
        }
        Ok(())
    }
}
/// Non-destructive testing: each method's extent in per cent of the weld, and a free note.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WeldNdt {
    pub methods: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}
/// Values evaluated at creation: millimetres except count and density (kg/m³).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WeldSettings {
    pub weld_type: String,
    pub symbol_standard: String,
    pub groove: String,
    pub penetration: String,
    pub root: String,
    pub tack_mode: String,
    pub root_gap: Option<f64>,
    pub add_fillet: bool,
    pub picks: BTreeMap<String, Vec<WeldPick>>,
    pub size: BTreeMap<String, f64>,
    pub face: String,
    pub size_by_leg: bool,
    pub blend_toes: bool,
    pub sheet_picked: bool,
    pub tangent_chain: bool,
    pub spans: Option<WeldSpans>,
    pub both_sides: bool,
    pub staggered: bool,
    pub sharp_mitre: bool,
    pub merge_crossing: bool,
    pub different_halves: bool,
    pub field_weld: bool,
    pub all_around: bool,
    pub passes_shown: bool,
    pub penetration_shown: bool,
    pub side: String,
    pub ends: String,
    pub process: Option<String>,
    pub spot_look: String,
    pub reference_plane: String,
    pub density: Option<f64>,
    pub overlap: Option<f64>,
    /// ISO 6947 position, one of [`POSITIONS`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    /// Quality level, `<one of QUALITY_STANDARDS>-<B|C|D>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    /// Welding procedure specification reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wps: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ndt: Option<WeldNdt>,
    /// Free text for the symbol's tail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_text: Option<String>,
    /// Filler metal designation, e.g. `ISO 14341-A G 3Si1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filler: Option<String>,
}
impl Default for WeldSettings {
    fn default() -> Self {
        Self {
            weld_type: "fillet".into(),
            symbol_standard: "iso2553_2019_a".into(),
            groove: "v".into(),
            penetration: "full".into(),
            root: "as_welded".into(),
            tack_mode: "pitch".into(),
            root_gap: None,
            add_fillet: false,
            picks: BTreeMap::new(),
            size: [("a".into(), 3.)].into_iter().collect(),
            face: "convex".into(),
            size_by_leg: false,
            blend_toes: false,
            sheet_picked: false,
            tangent_chain: false,
            spans: None,
            both_sides: false,
            staggered: false,
            sharp_mitre: false,
            merge_crossing: true,
            different_halves: false,
            field_weld: false,
            all_around: false,
            passes_shown: false,
            penetration_shown: false,
            side: "arrow".into(),
            ends: "square".into(),
            process: None,
            spot_look: "marks".into(),
            reference_plane: "sketch".into(),
            density: None,
            overlap: None,
            position: None,
            quality: None,
            wps: None,
            ndt: None,
            tail_text: None,
            filler: None,
        }
    }
}
impl WeldSettings {
    /// Build the supported annotation subset from evaluated settings, also used by the form.
    /// The caller retains unsupported choices in this settings record; no symbol is invented.
    pub fn symbol(&self) -> solidcraft_geom::weld::Result<Weld> {
        use solidcraft_geom::weld::{Contour, Intermittent, Invalid, Kind, Size, WeldSide};
        let kind = match self.weld_type.as_str() {
            "fillet" => Kind::Fillet,
            "butt" if self.groove == "square" => Kind::SquareButt,
            "butt" if self.groove == "v" => Kind::SingleVButt,
            _ => return Err(Invalid("unsupported weld type or groove shape")),
        };
        let size = if kind == Kind::Fillet {
            if self.size.contains_key("z1") || self.size.contains_key("z2") {
                return Err(Invalid("unequal-leg callouts are not supported"));
            }
            let value = *self.size.get(if self.size_by_leg { "z" } else { "a" }).ok_or(Invalid("missing fillet size"))?;
            Some(if let Some(total_mm) = self.size.get("s") {
                Size::DeepPenetrationThroat {
                    total_mm: *total_mm,
                    nominal_mm: if self.size_by_leg { value / std::f64::consts::SQRT_2 } else { value },
                }
            } else if self.size_by_leg {
                Size::Leg(value)
            } else {
                Size::Throat(value)
            })
        } else if self.penetration == "partial" {
            Some(Size::ButtPenetration(*self.size.get("s").ok_or(Invalid("partial penetration requires s"))?))
        } else {
            None
        };
        let spans = self.spans.as_ref().filter(|s| {
            kind == Kind::Fillet
                && !self.staggered
                && !self.all_around
                && (2..=10000).contains(&s.count)
                && dimension(s.length_mm)
                && dimension(s.pitch_mm)
                && s.pitch_mm >= s.length_mm + MIN_DIMENSION_MM
        });
        let side = WeldSide {
            kind,
            size,
            length_mm: spans.map(|s| s.length_mm),
            intermittent: spans.and_then(|s| u16::try_from(s.count).ok().map(|count| Intermittent { count: Some(count), pitch_mm: s.pitch_mm })),
            contour: Some(match self.face.as_str() {
                "flat" => Contour::Flush,
                "concave" => Contour::Concave,
                _ => Contour::Convex,
            }),
        };
        let (arrow_side, other_side) = if self.both_sides {
            (Some(side.clone()), Some(side))
        } else if self.side == "other" {
            (None, Some(side))
        } else {
            (Some(side), None)
        };
        let symbol = Weld { arrow_side, other_side, all_round: self.all_around, field: self.field_weld, tail: self.process.clone() };
        symbol.validate()?;
        Ok(symbol)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !TYPES.contains(&self.weld_type.as_str()) {
            return Err("unknown weld type".into());
        }
        if !["flat", "convex", "concave"].contains(&self.face.as_str()) {
            return Err("unknown weld face".into());
        }
        if self.size.is_empty()
            || self.size.len() > SIZE_KEYS.len()
            || self.size.iter().any(|(k, v)| !size_keys(&self.weld_type).contains(&k.as_str()) || !dimension(*v))
        {
            return Err("size values must be between 1e-6 and 100000 mm with known size keys".into());
        }
        if self.picks.len() > PICK_KINDS.len()
            || self.picks.keys().any(|k| !PICK_KINDS.contains(&k.as_str()))
            || self.picks.values().map(Vec::len).sum::<usize>() > MAX_PICKS
        {
            return Err("invalid weld pick groups or too many picks".into());
        }
        for pick in self.picks.values().flatten() {
            pick.validate()?;
        }
        for (name, value, allowed) in [
            ("symbol_standard", self.symbol_standard.as_str(), SYMBOL_STANDARDS),
            ("groove", self.groove.as_str(), GROOVES),
            ("penetration", self.penetration.as_str(), &["full", "partial"][..]),
            ("root", self.root.as_str(), &["as_welded", "melt_through", "backing_run"][..]),
            ("tack_mode", self.tack_mode.as_str(), &["pitch", "count", "points"][..]),
            ("side", self.side.as_str(), &["arrow", "other"][..]),
            ("ends", self.ends.as_str(), &["square", "tapered", "rounded"][..]),
            ("spot_look", self.spot_look.as_str(), &["marks", "ball", "marks_ball", "marks_pin", "pin", "dent"][..]),
            ("reference_plane", self.reference_plane.as_str(), &["sketch"][..]),
        ] {
            if !allowed.contains(&value) {
                return Err(format!("unknown {name}"));
            }
        }
        if self.weld_type == "butt" && (self.penetration == "partial") != self.size.contains_key("s") {
            return Err("partial penetration requires s; full penetration omits s".into());
        }
        if self.weld_type == "tack" {
            let valid = self.size.contains_key("l")
                && match self.tack_mode.as_str() {
                    "pitch" => self.size.contains_key("e") && !self.size.contains_key("n"),
                    "count" => self.size.get("n").is_some_and(|v| v.fract() == 0.) && !self.size.contains_key("e"),
                    "points" => self.size.len() == 1,
                    _ => false,
                };
            if !valid {
                return Err("tack sizes must match its mode; n must be a positive integer".into());
            }
        }
        if self.root_gap.is_some_and(|v| !v.is_finite() || !(0.0..=MAX_DIMENSION_MM).contains(&v)) {
            return Err("root gap must be finite and between 0 and 100000 mm".into());
        }
        if self.process.as_deref().is_some_and(|p| !process_valid(p)) {
            return Err("process must be ISO 4063 numbers of 1-3 digits, each with an optional variant (131-D), joined by + for a hybrid".into());
        }
        if self.position.as_deref().is_some_and(|p| !POSITIONS.contains(&p)) {
            return Err("unknown welding position".into());
        }
        if self
            .quality
            .as_deref()
            .is_some_and(|q| !q.rsplit_once('-').is_some_and(|(std, level)| QUALITY_STANDARDS.contains(&std) && ["B", "C", "D"].contains(&level)))
        {
            return Err("quality must be a known standard and level, e.g. ISO 5817-C".into());
        }
        for (name, value, max) in [("wps", &self.wps, 64), ("tail_text", &self.tail_text, 256), ("filler", &self.filler, 128)] {
            plain(name, value.as_deref(), max)?;
        }
        if let Some(ndt) = &self.ndt {
            plain("ndt note", ndt.note.as_deref(), 256)?;
            if (ndt.methods.is_empty() && ndt.note.is_none())
                || ndt.methods.iter().any(|(m, e)| !NDT_METHODS.contains(&m.as_str()) || !e.is_finite() || *e <= 0. || *e > 100.)
            {
                return Err("ndt needs a method or a note; methods VT, PT, MT, UT, RT with an extent above 0 and at most 100 %".into());
            }
        }
        if let Some(spans) = &self.spans {
            spans.validate()?;
        }
        if [self.density, self.overlap].into_iter().flatten().any(|v| !dimension(v)) {
            return Err("density and overlap must be finite and between 1e-6 and 100000".into());
        }
        Ok(())
    }
}
/// ISO 4063:2023 forms: `135`, a group `13`, a variant `131-D`, a hybrid `522+15`.
// shortcut: a variant is 1-3 capital letters, not checked against ISO 4063:2023's own list (paywalled); narrow once it is read.
pub fn process_valid(p: &str) -> bool {
    let digits = |n: &str| (1..=3).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit());
    p.len() <= 32
        && p.split('+').all(|part| match part.split_once('-') {
            Some((n, v)) => digits(n) && (1..=3).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_uppercase()),
            None => digits(part),
        })
}
fn plain(name: &str, text: Option<&str>, max: usize) -> Result<(), String> {
    if text.is_some_and(|t| t.trim().is_empty() || t.len() > max || !solidcraft_geom::plain_text::is_plain_line(t)) {
        return Err(format!("{name} must be one plain line of at most {max} bytes"));
    }
    Ok(())
}
fn label_valid(label: &str) -> bool {
    // A label like "W12" would read as another weld's number; spaces round it would hide that ("W3 ").
    let number_like = label.strip_prefix('W').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    plain("label", Some(label), MAX_LABEL_BYTES).is_ok() && label.trim() == label && !number_like
}
fn dimension(v: f64) -> bool {
    v.is_finite() && (MIN_DIMENSION_MM..=MAX_DIMENSION_MM).contains(&v)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "WrittenRecord")]
pub struct WeldRecord {
    pub id: u64,
    pub number: u32,
    /// Shown in place of W<number> when set (Q210); the number stays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub settings: WeldSettings,
    pub symbol: Option<Weld>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WrittenRecord {
    id: u64,
    number: u32,
    #[serde(default)]
    label: Option<String>,
    settings: WeldSettings,
    #[serde(default)]
    symbol: Option<Weld>,
}
impl TryFrom<WrittenRecord> for WeldRecord {
    type Error = String;
    fn try_from(r: WrittenRecord) -> Result<Self, String> {
        if r.id == 0 || r.id > crate::MAX_FEATURE_ID || r.number == 0 || r.number > MAX_NUMBER || r.label.as_deref().is_some_and(|l| !label_valid(l))
        {
            return Err("invalid weld identity".into());
        }
        r.settings.validate()?;
        if let Some(symbol) = &r.symbol {
            symbol.validate().map_err(|e| e.to_string())?;
        }
        Ok(Self { id: r.id, number: r.number, label: r.label, settings: r.settings, symbol: r.symbol })
    }
}
impl WeldRecord {
    /// What the table, the map and drawings show: the label, else W<number>.
    pub fn name(&self) -> String {
        self.label.clone().unwrap_or_else(|| format!("W{}", self.number))
    }
}
/// The number a new weld gets: one past the highest; None past the limit.
pub fn next_number(welds: &[WeldRecord]) -> Option<u32> {
    welds.iter().map(|w| w.number).max().unwrap_or(0).checked_add(1).filter(|n| *n <= MAX_NUMBER)
}
/// W1..Wn in creation order (ids grow with creation); labels stay.
pub fn renumber(welds: &mut [WeldRecord]) {
    let mut order: Vec<usize> = (0..welds.len()).collect();
    order.sort_by_key(|&i| welds[i].id);
    for (n, i) in (1..).zip(order) {
        welds[i].number = n;
    }
}
/// The number typed for weld `id` (W7 typed as W12); refused when another weld has it.
pub fn set_number(welds: &mut [WeldRecord], id: u64, number: u32) -> Result<(), String> {
    if !(1..=MAX_NUMBER).contains(&number) || welds.iter().any(|w| w.id != id && w.number == number) {
        return Err(format!("W{number} is taken or out of range"));
    }
    welds.iter_mut().find(|w| w.id == id).ok_or("weld does not exist")?.number = number;
    Ok(())
}
/// A label shown in place of W<n> for weld `id`, or None for W<n> again; unique among the welds.
pub fn set_label(welds: &mut [WeldRecord], id: u64, label: Option<String>) -> Result<(), String> {
    if let Some(l) = &label
        && (!label_valid(l) || welds.iter().any(|w| w.id != id && w.label.as_ref() == Some(l)))
    {
        return Err(format!("label must be one plain line of at most {MAX_LABEL_BYTES} bytes, not W<number>, and unique"));
    }
    welds.iter_mut().find(|w| w.id == id).ok_or("weld does not exist")?.label = label;
    Ok(())
}
/// Stream the list with a cap rather than allocating an unbounded saved list first.
pub(crate) fn deserialize_records<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<WeldRecord>, D::Error> {
    struct Records;
    impl<'de> serde::de::Visitor<'de> for Records {
        type Value = Vec<WeldRecord>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a bounded list of weld graphics")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut records = Vec::new();
            let mut ids = BTreeSet::new();
            let mut numbers = BTreeSet::new();
            let mut labels = BTreeSet::new();
            while let Some(record) = seq.next_element::<WeldRecord>()? {
                if records.len() >= MAX_WELDS
                    || !ids.insert(record.id)
                    || !numbers.insert(record.number)
                    || record.label.as_ref().is_some_and(|l| !labels.insert(l.clone()))
                {
                    return Err(serde::de::Error::custom("too many welds or repeated weld identity"));
                }
                records.push(record);
            }
            Ok(records)
        }
    }
    d.deserialize_seq(Records)
}
