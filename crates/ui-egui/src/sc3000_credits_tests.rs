//! SolidCraft 3000: SolidCredits put a price on some features, and never stop one.
use serde_json::json;

use super::*;
use crate::scenario::{Harness, rect_of};

fn harness() -> Harness {
    let mut h = Harness::new();
    h.app.preferences.language = "en".into();
    h.call("ui.resize", json!({"width":1600,"height":900}));
    h.app.home.open = false;
    h.app.help.sc3000_account = crate::sc3000_account::Account::new(None, || 0.5, || 1_795_478_400);
    h.settle();
    h
}

fn click(h: &mut Harness, handle: &str) {
    assert!(rect_of(handle).is_some(), "{handle} is on screen");
    h.step(&json!({"click":{"handle": handle}})).unwrap();
    h.frames(3);
}

/// A 20 x 10 plate, 5 mm thick, extruded from a sketch.
fn plate(h: &mut Harness) {
    h.app.run("sketch.create", json!({"plane": "XY", "name": "Base"})).unwrap();
    h.app.run("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [20, 10]})).unwrap();
    h.app.run("sketch.finish", json!({})).unwrap();
    h.app.run("solid.extrude", json!({"profile": {"sketch": "Base", "index": 0}, "distance": 5, "body_name": "Plate"})).unwrap();
    h.frames(3);
}

#[test]
fn an_extrude_costs_by_the_millimetre_and_is_built_anyway() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    plate(&mut h);
    assert!(h.app.session.doc.to_json().to_string().contains("Plate"), "the extrude made its body");
    let note = h.app.help.sc3000_credits.note.clone().expect("the extrude's note");
    assert_eq!(note.text, "This extrude cost 0.50 SolidCredits (5.00 mm at 0.10 per mm).");
    assert!((note.amount - 0.5).abs() < 1e-9);
    assert!(rect_of("sc3000_credits:note").is_some(), "the note shows");
    // A longer one costs more.
    h.app.run("sketch.create", json!({"plane": "XY", "name": "Tall"})).unwrap();
    h.app.run("sketch.rectangle.two_point", json!({"p0": [40, 0], "p1": [50, 10]})).unwrap();
    h.app.run("sketch.finish", json!({})).unwrap();
    h.app.run("solid.extrude", json!({"profile": {"sketch": "Tall", "index": 0}, "distance": 124})).unwrap();
    assert_eq!(h.app.help.sc3000_credits.note.as_ref().unwrap().text, "This extrude cost 12.40 SolidCredits (124.00 mm at 0.10 per mm).");
    assert!((h.app.help.sc3000_credits.charged - 12.9).abs() < 1e-9, "both went on the free trial");
}

#[test]
fn a_pattern_costs_per_instance() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    plate(&mut h);
    h.app
        .run(
            "solid.pattern.rectangular",
            json!({"bodies": ["Plate"], "dir1": [1, 0, 0], "count1": 3, "spacing1": 30, "dir2": [0, 1, 0], "count2": 2, "spacing2": 20}),
        )
        .unwrap();
    let note = h.app.help.sc3000_credits.note.clone().expect("the pattern's note");
    assert_eq!(note.text, "This pattern cost 12.00 SolidCredits (6 instances at 2.00 each).");
    let charge = price(&h.app, "sketch.pattern.circular", &json!({"count": "4"})).unwrap();
    assert_eq!(charge.amount, 8.0, "a count written as an expression is evaluated");
}

#[test]
fn fillets_shells_and_pack_appearances_are_pro_and_still_work() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    plate(&mut h);
    let revision = h.app.session.revision;
    h.app.run("solid.fillet", json!({"edges": [[20, 5, 5]], "radius": 1})).unwrap();
    assert!(h.app.session.revision > revision, "the fillet was made");
    assert_eq!(h.app.help.sc3000_credits.note.as_ref().unwrap().text, "Fillets are a SolidCraft 3000 Pro feature: 5.00 SolidCredits.");
    h.app.run("appearance.assign", json!({"bodies": ["Plate"], "appearance": "Brass - Polished"})).unwrap();
    assert_eq!(h.app.help.sc3000_credits.note.as_ref().unwrap().text, "Brass - Polished is in the Appearance Pack: 3.00 SolidCredits.");
    assert!(h.app.session.doc.to_json().to_string().contains("Brass - Polished"), "the appearance applied");
    // A free appearance, a box and a save cost nothing.
    h.app.help.sc3000_credits.note = None;
    h.app.run("appearance.assign", json!({"bodies": ["Plate"], "appearance": "Aluminum - Satin"})).unwrap();
    assert_eq!(h.app.help.sc3000_credits.note, None);
    for id in ["file.save", "file.save_as", "file.export_step", "file.export_stl", "solid.box"] {
        assert_eq!(price(&h.app, id, &json!({"distance": 10, "count": 4})), None, "{id} is free");
    }
    assert!(price(&h.app, "solid.shell", &json!({})).is_some());
    assert!(in_pack("Titanium") && in_pack("Copper - Clean PBR") && !in_pack("Steel - Satin"));
}

#[test]
fn top_up_has_nothing_to_buy_and_ok_closes_the_note() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    plate(&mut h);
    click(&mut h, "sc3000_credits:note:ok");
    assert_eq!(h.app.help.sc3000_credits.note, None, "OK closes the note");
    h.app.run("solid.fillet", json!({"edges": [[20, 5, 5]], "radius": 1})).unwrap();
    h.frames(3);
    click(&mut h, "sc3000_credits:note:top_up");
    assert!(h.app.help.sc3000_credits.top_up && h.app.help.sc3000_credits.note.is_none());
    assert!(h.widget(tr("sc3000.credits.top_up_body")).is_some());
    assert!(rect_of("sc3000_credits:top_up:parody").is_some(), "the parody line is on the page");
    click(&mut h, "sc3000_credits:top_up:close");
    assert!(!h.app.help.sc3000_credits.top_up);
}

#[test]
fn the_account_page_shows_the_balance() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    plate(&mut h);
    let account = &mut h.app.help.sc3000_account;
    account.name = "drafter".into();
    account.password = "x".into();
    account.sign_in();
    account.open();
    h.frames(4);
    assert!(h.widget("SolidCredits: 0.00").is_some());
    assert!(h.widget("Charged to your free trial so far: 0.50 SolidCredits").is_some());
    click(&mut h, "sc3000_credits:account:top_up");
    assert!(h.app.help.sc3000_credits.top_up);
}

#[test]
fn german_prices_use_a_decimal_comma() {
    crate::i18n::install();
    let _language = solidcraft_ui_text::scope("de");
    let mut h = harness();
    h.app.preferences.language = "de".into();
    plate(&mut h);
    assert_eq!(h.app.help.sc3000_credits.note.as_ref().unwrap().text, "Diese Extrusion hat 0,50 SolidCredits gekostet (5,00 mm zu 0,10 pro mm).");
}

#[test]
fn save_and_export_work_with_the_note_and_top_up_open() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    let dir = h.dir.clone();
    h.app.services.pick_save = Some(Box::new(move |name, _| Box::pin(std::future::ready(Some(dir.join(name).to_string_lossy().into_owned())))));
    h.app.run("file.new", json!({"name": "Priced"})).unwrap();
    h.frames(3);
    plate(&mut h);
    h.app.help.sc3000_credits.top_up = true;
    h.frames(4);
    assert!(rect_of("sc3000_credits:note").is_some() && h.app.help.sc3000_credits.top_up, "both are open");
    h.step(&json!({"key":"S","cmd":true})).unwrap();
    h.settle();
    h.frames(3);
    assert!(!crate::documents::any_dirty(&h.app), "Ctrl+S saved the design");
    assert!(h.dir.join("Priced.sc").is_file());
    assert!(crate::toolbar::start_export(&mut h.app, "file.export_step"));
    h.settle();
    h.frames(3);
    assert!(h.dir.join("Priced.step").is_file(), "STEP export written");
}

#[test]
fn every_string_is_in_english_and_german() {
    crate::i18n::install();
    let file: serde_json::Value = serde_json::from_str(include_str!("../../../assets/locales/pending/sc3000-credits.json")).unwrap();
    let (english, german) = (file["en"].as_object().unwrap(), file["de"].as_object().unwrap());
    assert_eq!(english.keys().collect::<Vec<_>>(), german.keys().collect::<Vec<_>>(), "the same messages in both");
    // The same in both languages on purpose.
    let alike = ["sc3000.credits.balance", "sc3000.credits.ok", "sc3000.credits.pro"];
    for (id, text) in english {
        let shown = {
            let _language = solidcraft_ui_text::scope("en");
            tr(id).to_string()
        };
        assert_eq!(&shown, text.as_str().unwrap(), "{id}");
        let _language = solidcraft_ui_text::scope("de");
        assert_eq!(tr(id), german[id].as_str().unwrap(), "{id}");
        if !alike.contains(&id.as_str()) {
            assert_ne!(tr(id), shown, "{id} has German text");
        }
    }
    let source = include_str!("sc3000_credits.rs");
    for piece in source.split("\"sc3000.credits.").skip(1) {
        let id = format!("sc3000.credits.{}", &piece[..piece.find('"').unwrap()]);
        assert!(english.contains_key(&id), "{id} has no text");
    }
}

#[test]
fn nothing_here_can_reach_the_network_or_take_payment() {
    let source = include_str!("sc3000_credits.rs");
    for word in
        ["open_url", "http", "fetch", "XMLHttpRequest", "sendBeacon", "WebSocket", "window.open", "tickets", "OpenUrl", "text_field", "password"]
    {
        assert!(!source.contains(word), "the credits code mentions {word}");
    }
    let strings = include_str!("../../../assets/locales/pending/sc3000-credits.json").to_lowercase();
    for word in ["$", "€", "£", "usd", "eur", "card", "karte", "iban", "paypal", "checkout", "subscribe", "abonn"] {
        assert!(!strings.contains(word), "the credits text mentions {word}");
    }
}
