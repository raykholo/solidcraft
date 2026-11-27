//! SolidCraft 3000: Cumulo helps with nothing and the updates insist, and neither ever gets in the way.
use serde_json::json;

use super::*;
use crate::scenario::{Harness, rect_of};

fn harness() -> Harness {
    let mut h = Harness::new();
    h.app.preferences.language = "en".into();
    h.call("ui.resize", json!({"width":1600,"height":900}));
    h.app.home.open = false;
    h.settle();
    h
}

fn click(h: &mut Harness, handle: &str) {
    assert!(rect_of(handle).is_some(), "{handle} is on screen");
    h.step(&json!({"click":{"handle": handle}})).unwrap();
    h.frames(3);
}

/// Seconds of use, one second a tick.
fn use_for(helper: &mut Helper, updates: &mut Updates, seconds: usize) {
    let start = helper.active();
    for s in 0..=seconds {
        helper.tick(start + s as f64, true);
        updates.tick(start + s as f64, true);
    }
}

#[test]
fn cumulo_says_hello_then_tips_on_what_was_done() {
    let (mut helper, mut updates) = (Helper::default(), Updates::default());
    use_for(&mut helper, &mut updates, 40);
    assert_eq!(helper.tip, None, "not before 45 s of use");
    use_for(&mut helper, &mut updates, 10);
    assert_eq!(helper.tip, Some("sc3000.helper.hello"));
    helper.away(false);
    helper.command("solid.extrude");
    use_for(&mut helper, &mut updates, 60);
    assert_eq!(helper.tip, None, "Not now keeps it away for two minutes");
    use_for(&mut helper, &mut updates, 70);
    assert_eq!(helper.tip, Some("sc3000.helper.tip_extrude"));
    helper.more();
    assert_eq!(helper.tip, Some("sc3000.helper.tip_general_1"));
    helper.away(true);
    helper.command("solid.fillet");
    use_for(&mut helper, &mut updates, 590);
    assert_eq!(helper.tip, None, "Take a break keeps it away for ten minutes");
    use_for(&mut helper, &mut updates, 20);
    assert_eq!(helper.tip, Some("sc3000.helper.tip_fillet"));
    helper.away(false);
    use_for(&mut helper, &mut updates, 300);
    assert_eq!(helper.tip, None, "with nothing new done, Cumulo stays away");
    assert_eq!(tip_for("sketch.line"), Some("sc3000.helper.tip_sketch"));
    assert_eq!(tip_for("solid.pattern.circular"), Some("sc3000.helper.tip_pattern"));
    assert_eq!(tip_for("view.fit"), None);
}

#[test]
fn later_always_closes_the_update_and_it_asks_more_firmly_each_time() {
    let (mut helper, mut updates) = (Helper::default(), Updates::default());
    use_for(&mut helper, &mut updates, 290);
    assert!(!updates.open, "not before five minutes of use");
    use_for(&mut helper, &mut updates, 15);
    for level in 0..LEVELS.len() + 2 {
        assert!(updates.open, "the notice is back (round {level})");
        assert_eq!(updates.level, level.min(LEVELS.len() - 1));
        updates.later();
        assert!(!updates.open, "Later closed it at level {}", updates.level);
        use_for(&mut helper, &mut updates, 170);
        assert!(!updates.open, "three minutes between notices");
        use_for(&mut helper, &mut updates, 15);
    }
    // Update now "installs" in four seconds, and then the next one is polite again.
    updates.update_now();
    use_for(&mut helper, &mut updates, 2);
    assert!(!updates.installed());
    use_for(&mut helper, &mut updates, 3);
    assert!(updates.installed());
    updates.done();
    assert!(!updates.open);
    assert_eq!((updates.level, updates.version_text().as_str()), (0, "3000.0.2"));
    // Later works mid-install too.
    updates.due_now();
    use_for(&mut helper, &mut updates, 1);
    updates.update_now();
    use_for(&mut helper, &mut updates, 1);
    updates.later();
    assert!(!updates.open && updates.progress.is_none());
}

#[test]
fn the_windows_show_and_close_in_one_click() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    h.app.help.sc3000_helper.visit_now();
    h.app.help.sc3000_updates.due_now();
    h.frames(4);
    assert!(rect_of("sc3000_helper:face").is_some(), "Cumulo shows, with its face");
    assert!(h.widget(tr("sc3000.helper.hello")).is_some());
    click(&mut h, "sc3000_helper:more");
    assert!(h.widget(tr("sc3000.helper.tip_general_1")).is_some());
    click(&mut h, "sc3000_helper:not_now");
    assert_eq!(h.app.help.sc3000_helper.tip, None);
    assert!(h.widget("SolidCraft 3000 3000.0.1 is available. Update now?").is_some());
    click(&mut h, "sc3000_updates:later");
    assert!(!h.app.help.sc3000_updates.open);
    h.app.help.sc3000_updates.due_now();
    h.frames(3);
    click(&mut h, "sc3000_updates:now");
    for _ in 0..12 {
        h.app.help.sc3000_updates.progress = h.app.help.sc3000_updates.progress.map(|p| (p + 0.25).min(1.0));
        h.frames(1);
    }
    h.frames(3);
    assert!(rect_of("sc3000_updates:installed").is_some(), "the update reports it is installed");
    click(&mut h, "sc3000_updates:close");
    assert!(!h.app.help.sc3000_updates.open);
}

#[test]
fn cumulo_stays_clear_of_the_middle_of_the_window() {
    let _language = solidcraft_ui_text::scope("de");
    let mut h = harness();
    h.app.preferences.language = "de".into();
    h.app.help.sc3000_helper.visit_now();
    h.frames(4);
    let card = rect_of("sc3000_helper").expect("Cumulo shows");
    assert!(card.left() > 800.0 && card.bottom() < 900.0, "bottom right and on screen: {card:?}");
}

#[test]
fn save_and_export_work_with_cumulo_and_the_update_open() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    let dir = h.dir.clone();
    h.app.services.pick_save = Some(Box::new(move |name, _| Box::pin(std::future::ready(Some(dir.join(name).to_string_lossy().into_owned())))));
    h.app.run("file.new", json!({"name": "Helped"})).unwrap();
    h.app.session.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10})).unwrap();
    h.app.help.sc3000_helper.visit_now();
    h.app.help.sc3000_updates.due_now();
    h.frames(4);
    assert!(h.app.help.sc3000_helper.tip.is_some() && h.app.help.sc3000_updates.open, "both are open");
    h.step(&json!({"key":"S","cmd":true})).unwrap();
    h.settle();
    h.frames(3);
    assert!(!crate::documents::any_dirty(&h.app), "Ctrl+S saved the design");
    assert!(h.dir.join("Helped.sc").is_file());
    assert!(crate::toolbar::start_export(&mut h.app, "file.export_step"));
    h.settle();
    h.frames(3);
    assert!(h.dir.join("Helped.step").is_file(), "STEP export written");
}

#[test]
fn every_string_is_in_english_and_german() {
    crate::i18n::install();
    let file: serde_json::Value = serde_json::from_str(include_str!("../../../assets/locales/pending/sc3000-helper.json")).unwrap();
    let (english, german) = (file["en"].as_object().unwrap(), file["de"].as_object().unwrap());
    assert_eq!(english.keys().collect::<Vec<_>>(), german.keys().collect::<Vec<_>>(), "the same messages in both");
    for (id, text) in english {
        let shown = {
            let _language = solidcraft_ui_text::scope("en");
            tr(id).to_string()
        };
        assert_eq!(&shown, text.as_str().unwrap(), "{id}");
        let _language = solidcraft_ui_text::scope("de");
        assert_eq!(tr(id), german[id].as_str().unwrap(), "{id}");
        if id != "sc3000.helper.name" {
            assert_ne!(tr(id), shown, "{id} has German text");
        }
    }
    let source = include_str!("sc3000_helper.rs");
    for piece in source.split("\"sc3000.helper.").skip(1) {
        let id = format!("sc3000.helper.{}", &piece[..piece.find('"').unwrap()]);
        assert!(english.contains_key(&id), "{id} has no text");
    }
}

#[test]
fn nothing_here_can_reach_the_network_and_nobody_is_threatened() {
    let source = include_str!("sc3000_helper.rs");
    for word in ["open_url", "http", "fetch", "XMLHttpRequest", "sendBeacon", "WebSocket", "window.open", "tickets", "OpenUrl"] {
        assert!(!source.contains(word), "the helper code mentions {word}");
    }
    let strings = include_str!("../../../assets/locales/pending/sc3000-helper.json").to_lowercase();
    for word in [
        "or else",
        "kill",
        "destroy",
        "delete",
        "you will lose",
        "punish",
        "sonst",
        "löschen",
        "zerstör",
        "clippy",
        "paperclip",
        "büroklammer",
        "microsoft",
        "it looks like",
    ] {
        assert!(!strings.contains(word), "the helper text says {word}");
    }
}
