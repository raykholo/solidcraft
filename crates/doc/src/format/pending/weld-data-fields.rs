//! A weld graphic's settings can save manufacturing data: `position` (ISO 6947 main position,
//! PA to PK), `quality` (`ISO 5817`, `ISO 10042`, `ISO 13919-1` or `ISO 13919-2` with level B,
//! C or D, e.g. `ISO 5817-C`), `wps`, `ndt` (`methods`: VT, PT, MT, UT, RT to an extent in per
//! cent, and a `note`), `tail_text` and `filler`; its `process` can be an ISO 4063 variant
//! (`131-D`) or hybrid (`522+15`). A weld can save a `label` shown in place of W<number>. Each is
//! saved only when set, so a design without them saves as before and needs no step. The last
//! release warns about the newer format number, opens what it can read and keeps the rest
//! (`crates/doc/src/newer_file.rs`).
