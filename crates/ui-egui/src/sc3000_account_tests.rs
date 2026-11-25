//! SolidCraft 3000: the pretend Cloud Account keeps no password, sends nothing, and never gets in
//! the way of the real CAD.
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::*;
use crate::scenario::{Harness, rect_of};

/// Seconds since 1970 the tests' clock says: 2026-11-24.
const NOW: u64 = 1_795_478_400;

/// A store that keeps every value written (the last one is what it reads back).
#[derive(Clone, Default)]
struct Recording(Arc<Mutex<Vec<String>>>);

impl Store for Recording {
    fn read(&self) -> Option<String> {
        self.0.lock().unwrap().last().cloned()
    }
    fn write(&self, value: &str) -> bool {
        self.0.lock().unwrap().push(value.to_string());
        true
    }
}

impl Recording {
    fn all(&self) -> String {
        self.0.lock().unwrap().join("\n")
    }
}

/// A browser that blocks storage: every read and write fails (the web store turns a throw into
/// exactly this).
struct Blocked;

impl Store for Blocked {
    fn read(&self) -> Option<String> {
        None
    }
    fn write(&self, _value: &str) -> bool {
        false
    }
}

/// A store holding something that is not an account.
struct Garbage;

impl Store for Garbage {
    fn read(&self) -> Option<String> {
        Some("{not json".into())
    }
    fn write(&self, _value: &str) -> bool {
        true
    }
}

type Calls = Arc<std::sync::atomic::AtomicUsize>;

/// A harness whose account keeps to `store` and whose HTTP services count every call.
fn harness(store: Option<Box<dyn Store>>) -> (Harness, Calls) {
    let mut h = Harness::new();
    h.app.preferences.language = "en".into();
    h.call("ui.resize", json!({"width":1600,"height":900}));
    h.app.home.open = false;
    h.app.help.sc3000_account = Account::new(store, || 0.5, || NOW);
    let calls: Calls = Default::default();
    let seen = calls.clone();
    h.app.services.http_get = Some(Arc::new(move |_| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Vec::new())
    }));
    let seen = calls.clone();
    h.app.services.tickets_http = Some(Arc::new(move |_, done: std::sync::mpsc::Sender<crate::tickets::Reply>| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = done.send(Ok((200, Vec::new())));
    }));
    h.settle();
    (h, calls)
}

fn click(h: &mut Harness, handle: &str) {
    assert!(rect_of(handle).is_some(), "{handle} is on screen");
    h.step(&json!({"click":{"handle": handle}})).unwrap();
    h.frames(3);
}

fn type_into(h: &mut Harness, handle: &str, text: &str) {
    click(h, handle);
    h.step(&json!({"text": text})).unwrap();
    h.frames(2);
}

fn shows(h: &Harness, id: &str) -> bool {
    h.widget(tr(id)).is_some()
}

fn sign_in(h: &mut Harness, name: &str, password: &str) {
    click(h, "sc3000_account:open");
    if !name.is_empty() {
        type_into(h, "sc3000_account:id", name);
    }
    type_into(h, "sc3000_account:password", password);
    click(h, "sc3000_account:submit");
}

/// Whether the last frame painted `text` (the back-up note is painted, not a widget).
fn painted(h: &Harness, text: &str) -> bool {
    let mut found = false;
    for clipped in &h.shapes {
        crate::test_support::each_shape(&clipped.shape, &mut |shape| {
            if let egui::Shape::Text(shape) = shape
                && shape.galley.text() == text
            {
                found = true;
            }
        });
    }
    found
}

/// Make the design shown unsaved.
fn edit(h: &mut Harness) {
    h.app.session.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10})).unwrap();
    h.frames(3);
    assert!(crate::documents::any_dirty(&h.app));
}

#[test]
fn signing_in_keeps_only_the_passwords_length_and_when_it_was_set() {
    let _language = solidcraft_ui_text::scope("en");
    let store = Recording::default();
    let (mut h, calls) = harness(Some(Box::new(store.clone())));
    assert!(h.widget("Sign in").is_some(), "the maintenance strip offers Sign in");
    click(&mut h, "sc3000_account:open");
    assert_eq!(h.app.help.sc3000_account.screen, Some(Screen::SignIn));
    assert!(shows(&h, "sc3000.account.parody"), "the parody line is on the sign-in page");
    assert!(shows(&h, "sc3000.account.kept"));
    type_into(&mut h, "sc3000_account:id", "drafter");
    type_into(&mut h, "sc3000_account:password", "hunter2-secret!");
    assert_eq!(h.app.help.sc3000_account.password, "hunter2-secret!");
    click(&mut h, "sc3000_account:submit");

    let account = &h.app.help.sc3000_account;
    assert!(account.saved.signed_in && account.screen.is_none(), "any password signs in the first time");
    assert_eq!(account.saved, Saved { name: "drafter".into(), password_length: 15, password_set_at: NOW, signed_in: true, expired: false });
    assert!(account.password.is_empty(), "the field is emptied on submit");
    assert!(account.kept);
    let written = store.all();
    assert!(!written.is_empty(), "the account was kept");
    assert!(!written.contains("hunter2"), "the password is never stored: {written}");
    let last: serde_json::Value = serde_json::from_str(&store.read().unwrap()).unwrap();
    assert_eq!(last, json!({"name": "drafter", "password_length": 15, "password_set_at": NOW, "signed_in": true, "expired": false}));
    assert!(!serde_json::to_string(&h.app.preferences).unwrap().contains("hunter2"), "nor in the preferences");
    // The strip now names the account, and its page says what is known: the length and the day.
    assert!(h.widget("drafter").is_some());
    click(&mut h, "sc3000_account:open");
    assert_eq!(h.app.help.sc3000_account.screen, Some(Screen::Account));
    assert!(h.widget("Password: 15 characters, set 2026-11-24. That is all we know.").is_some());
    assert!(shows(&h, "sc3000.account.parody"), "the parody line is on the account page");
    // Next time, the account is read back signed in.
    let again = Account::new(Some(Box::new(store.clone())), || 0.5, || NOW);
    assert!(again.saved.signed_in && again.saved.name == "drafter");
    // Signing out keeps it signed out.
    click(&mut h, "sc3000_account:sign_out");
    assert!(!h.app.help.sc3000_account.saved.signed_in && h.app.help.sc3000_account.screen.is_none());
    assert!(!Account::new(Some(Box::new(store.clone())), || 0.5, || NOW).saved.signed_in);
    assert!(!store.all().contains("hunter2"));
    assert_eq!(h.take_opened(), Vec::<String>::new(), "no page opened");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "no HTTP request");
}

#[test]
fn a_browser_that_blocks_storage_still_signs_in_for_the_tab() {
    let _language = solidcraft_ui_text::scope("en");
    let (mut h, _) = harness(Some(Box::new(Blocked)));
    sign_in(&mut h, "drafter", "anything");
    let account = &h.app.help.sc3000_account;
    assert!(account.saved.signed_in, "signed in although nothing could be kept");
    assert!(!account.kept);
    click(&mut h, "sc3000_account:open");
    assert!(shows(&h, "sc3000.account.kept_tab"), "the page says it lasts until the tab closes");
    assert!(!shows(&h, "sc3000.account.kept"));
    // Unreadable storage, or none at all, is a signed-out account.
    assert_eq!(Account::new(Some(Box::new(Blocked)), || 0.5, || NOW).saved, Saved::default());
    assert_eq!(Account::new(Some(Box::new(Garbage)), || 0.5, || NOW).saved, Saved::default());
    let mut none = Account::new(None, || 0.5, || NOW);
    none.name = "x".into();
    none.password = "y".into();
    none.sign_in();
    assert!(none.saved.signed_in && !none.kept);
}

#[test]
fn sign_ins_ask_for_some_id_and_some_password() {
    let _language = solidcraft_ui_text::scope("en");
    let mut account = Account::new(None, || 0.5, || NOW);
    account.open();
    account.password = "pw".into();
    account.sign_in();
    assert_eq!(account.error, Some("sc3000.account.need_id"));
    assert!(account.password.is_empty(), "a refused password is dropped too");
    account.name = "  drafter ".into();
    account.sign_in();
    assert_eq!(account.error, Some("sc3000.account.need_password"));
    account.password = "pw".into();
    account.sign_in();
    assert!(account.saved.signed_in && account.saved.name == "drafter" && account.saved.password_length == 2);
}

#[test]
fn sign_outs_come_at_most_once_per_ten_minutes_of_use() {
    let rolls: [fn() -> f64; 4] = [|| 0.0, || 0.3, || 1.0, || f64::NAN];
    for random in rolls {
        let roll = random();
        let mut account = Account::new(None, random, || NOW);
        let mut at = Vec::new();
        let mut time = 0.0;
        while time <= 4.0 * 3600.0 {
            account.tick(time, true, false);
            if account.screen == Some(Screen::Expired) {
                at.push(account.active());
                account.close();
            }
            time += 0.5;
        }
        assert!(at.len() >= 10, "{roll}: sign-outs do happen: {at:?}");
        assert!(at.len() <= 24, "{roll}: {at:?}");
        assert!(at[0] >= SESSION_SECONDS, "{roll}: not in the first ten minutes: {at:?}");
        for pair in at.windows(2) {
            assert!(pair[1] - pair[0] >= SESSION_SECONDS, "{roll}: two sign-outs {pair:?} less than ten minutes apart");
            assert!(pair[1] - pair[0] <= 2.0 * SESSION_SECONDS + 1.0, "{roll}: {pair:?}");
        }
        assert_eq!(account.expiries as usize, at.len());
    }
    // Time in the background (window not focused) is not use.
    let mut idle = Account::new(None, || 0.0, || NOW);
    for second in 0..=3600 {
        idle.tick(f64::from(second), false, false);
    }
    assert_eq!(idle.screen, None);
    // A sign-out waits while the account window or the recovery report is open.
    let mut busy = Account::new(None, || 0.0, || NOW);
    busy.open();
    for second in 0..=1300 {
        busy.tick(f64::from(second), true, false);
    }
    assert_eq!((busy.screen, busy.expiries), (Some(Screen::SignIn), 0));
    busy.close();
    busy.tick(1301.0, true, false);
    assert_eq!((busy.screen, busy.expiries), (Some(Screen::Expired), 1));
}

#[test]
fn the_expired_notice_closes_in_one_click() {
    let _language = solidcraft_ui_text::scope("en");
    let (mut h, _) = harness(None);
    sign_in(&mut h, "drafter", "abcdef");
    h.app.help.sc3000_account.expire_now();
    h.frames(3);
    assert_eq!(h.app.help.sc3000_account.screen, Some(Screen::Expired));
    assert!(h.widget("Please sign in again. Your design is fine: it never left this tab.").is_some());
    assert!(shows(&h, "sc3000.account.parody"));
    assert!(!h.app.help.sc3000_account.saved.signed_in && h.app.help.sc3000_account.saved.expired);
    click(&mut h, "sc3000_account:expired:not_now");
    assert_eq!(h.app.help.sc3000_account.screen, None, "Not now closes it");
    // The CAD carries on: a command runs and the design changes.
    edit(&mut h);
}

#[test]
fn after_a_sign_out_the_password_is_incorrect_until_it_is_reset_under_the_rules() {
    let _language = solidcraft_ui_text::scope("en");
    let store = Recording::default();
    let (mut h, calls) = harness(Some(Box::new(store.clone())));
    sign_in(&mut h, "drafter", "abcdef");
    h.app.help.sc3000_account.expire_now();
    h.frames(3);
    click(&mut h, "sc3000_account:expired:sign_in");
    assert_eq!(h.app.help.sc3000_account.screen, Some(Screen::SignIn));
    assert_eq!(h.app.help.sc3000_account.name, "drafter", "the Cloud ID is filled in");
    type_into(&mut h, "sc3000_account:password", "abcdef");
    click(&mut h, "sc3000_account:submit");
    assert!(h.widget("Incorrect password.").is_some(), "the same password is now incorrect");
    assert!(!h.app.help.sc3000_account.saved.signed_in);
    click(&mut h, "sc3000_account:reset");
    assert_eq!(h.app.help.sc3000_account.screen, Some(Screen::Reset));
    assert!(shows(&h, "sc3000.account.parody"), "the parody line is on the reset page");

    let attempt = |h: &mut Harness, password: &str| {
        type_into(h, "sc3000_account:new_password", password);
        click(h, "sc3000_account:change");
        assert!(h.app.help.sc3000_account.password.is_empty());
    };
    attempt(&mut h, "abcdef");
    assert!(h.widget("You cannot use your existing password.").is_some());
    assert_eq!(h.app.help.sc3000_account.reset.rules, [Rule::Fillet]);
    attempt(&mut h, "nope");
    assert!(h.widget("That password breaks a rule:").is_some());
    assert!(h.widget("✗ must contain a fillet (the word “fillet” will do)").is_some(), "the broken rule is marked");
    attempt(&mut h, "Fillet");
    assert!(h.widget("Almost! One more rule just came in.").is_some());
    assert_eq!(h.app.help.sc3000_account.reset.rules, [Rule::Fillet, Rule::Thousand]);
    attempt(&mut h, "fillet3000abc");
    assert_eq!(h.app.help.sc3000_account.reset.rules[2], Rule::NoLetter('c', true), "the letter just typed");
    assert!(h.widget("• cannot contain the letter “c”: you just typed it").is_some());
    attempt(&mut h, "fillet3000xyz");
    assert_eq!(h.app.help.sc3000_account.reset.rules[3], Rule::Longer(6), "longer than the old six characters");
    attempt(&mut h, "fillet3000xyz");

    let account = &h.app.help.sc3000_account;
    assert_eq!(account.screen, Some(Screen::Account));
    assert_eq!(account.notice, Some("sc3000.account.changed"));
    assert_eq!(account.saved, Saved { name: "drafter".into(), password_length: 13, password_set_at: NOW, signed_in: true, expired: false });
    assert!(h.widget("Password changed. It is rated Cloud Strong. You are signed in.").is_some());
    let written = store.all();
    for secret in ["abcdef", "nope", "fillet", "3000abc", "3000xyz"] {
        assert!(!written.contains(secret), "{secret} was stored: {written}");
    }
    // Signed in again, the next sign-in takes any password.
    click(&mut h, "sc3000_account:close");
    assert_eq!(h.app.help.sc3000_account.screen, None);
    assert_eq!(h.take_opened(), Vec::<String>::new());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "no HTTP request");
}

#[test]
fn the_reset_always_ends_within_a_few_attempts() {
    // Whatever is typed, attempt MERCY_ATTEMPT is accepted.
    for typed in ["x", "fillet", "fillet3000", "qqqqqqqq"] {
        let mut account = Account::new(None, || 0.5, || NOW);
        account.saved = Saved { name: "a".into(), password_length: 64, password_set_at: 0, signed_in: false, expired: true };
        account.start_reset();
        let mut tries = 0;
        let changed = loop {
            account.password = typed.into();
            tries += 1;
            if account.submit_reset() {
                break true;
            }
            assert!(tries < MERCY_ATTEMPT, "{typed}: still refused after {tries}");
        };
        assert!(changed && account.saved.signed_in && !account.saved.expired && account.saved.password_length == typed.len());
        assert!(tries <= MERCY_ATTEMPT, "{typed}: {tries}");
    }
    // A blank attempt does not count.
    let mut account = Account::new(None, || 0.5, || NOW);
    account.start_reset();
    assert!(!account.submit_reset());
    assert_eq!((account.reset.attempts, account.reset.message), (0, Some("sc3000.account.empty_new")));
    // The rules never contradict each other: this password keeps every rule there can be.
    let keeper = format!("fillet3000{}", "9".repeat(LONGER_CAP));
    for letter in 'a'..='z' {
        if !FILLET.contains(letter) {
            assert!(Rule::NoLetter(letter, true).holds(&keeper));
        }
    }
    for rule in [Rule::Fillet, Rule::Thousand, Rule::NoLetter('q', false), Rule::Longer(LONGER_CAP)] {
        assert!(rule.holds(&keeper), "{rule:?}");
    }
    // Knowing the rules, five attempts do it.
    let mut account = Account::new(None, || 0.5, || NOW);
    account.saved.password_length = 64;
    account.start_reset();
    let mut tries = 0;
    loop {
        account.password = keeper.clone();
        tries += 1;
        if account.submit_reset() {
            break;
        }
        assert!(tries < MERCY_ATTEMPT);
    }
    assert!(account.reset.rules.is_empty() && account.saved.password_length == keeper.len());
    assert_eq!((tries, account.notice), (5, Some("sc3000.account.changed")));
    // With no Cloud ID ever given, the reset signs in a default one.
    let mut nameless = Account::new(None, || 0.5, || NOW);
    nameless.start_reset();
    for _ in 0..MERCY_ATTEMPT {
        nameless.password = "x".into();
        if nameless.submit_reset() {
            break;
        }
    }
    assert_eq!(nameless.notice, Some("sc3000.account.mercy"));
    assert!(!nameless.saved.name.is_empty());
}

#[test]
fn the_back_up_note_asks_every_fifteen_seconds_while_the_design_is_unsaved() {
    let mut account = Account::new(None, || 0.5, || NOW);
    let mut shown = Vec::new();
    let mut time = 0.0;
    while time <= 100.0 {
        account.tick(time, true, true);
        if account.nag {
            shown.push(account.active());
            account.dismiss_nag();
        }
        time += 0.25;
    }
    assert_eq!(shown.len(), 6, "{shown:?}");
    assert!((shown[0] - NAG_SECONDS).abs() < 0.3, "{shown:?}");
    for pair in shown.windows(2) {
        assert!((pair[1] - pair[0] - NAG_SECONDS).abs() < 0.3, "{pair:?}");
    }
    // Left alone, it stays until it is dismissed.
    while !account.nag {
        time += 0.25;
        account.tick(time, true, true);
    }
    for _ in 0..40 {
        time += 0.25;
        account.tick(time, true, true);
        assert!(account.nag);
    }
    // A save request (Ctrl+S) dismisses it for the next fifteen seconds of use.
    account.saving();
    time += 0.25;
    account.tick(time, true, true);
    assert!(!account.nag, "a save dismisses it");
    let saved = account.active();
    while !account.nag {
        time += 0.25;
        account.tick(time, true, true);
    }
    assert!((account.active() - saved - NAG_SECONDS).abs() < 0.3, "back after fifteen seconds, not before");
    // A saved design is never nagged.
    let mut clean = Account::new(None, || 0.5, || NOW);
    for second in 0..=120 {
        clean.tick(f64::from(second), true, false);
        assert!(!clean.nag);
    }
}

#[test]
fn ctrl_s_and_not_now_dismiss_the_back_up_note_and_save_now_really_saves() {
    let _language = solidcraft_ui_text::scope("en");
    let (mut h, _) = harness(None);
    edit(&mut h);
    assert!(!h.app.help.sc3000_account.nag, "not straight away");
    let due = |h: &mut Harness| {
        let account = &mut h.app.help.sc3000_account;
        account.nag_at = account.active() + 0.05;
        h.frames(6);
        assert!(h.app.help.sc3000_account.nag);
        assert!(rect_of("sc3000_account:nag").is_some(), "the note is on screen");
        assert!(painted(h, "Your design is not backed up to the cloud. Save now?"));
    };
    due(&mut h);
    // Ctrl+S with its file picker cancelled: the note goes and the design stays unsaved.
    h.app.services.pick_save = Some(Box::new(|_, _| Box::pin(std::future::ready(None))));
    let active = h.app.help.sc3000_account.active();
    h.step(&json!({"key":"S","cmd":true})).unwrap();
    h.frames(6);
    assert!(!h.app.help.sc3000_account.nag, "Ctrl+S dismisses it");
    assert!(rect_of("sc3000_account:nag").is_none());
    assert!(!painted(&h, "Your design is not backed up to the cloud. Save now?"));
    assert!(crate::documents::any_dirty(&h.app));
    assert!(h.app.help.sc3000_account.nag_at >= active + NAG_SECONDS - 0.01, "until the next fifteen seconds");
    // Not now dismisses it too, in one click.
    due(&mut h);
    click(&mut h, "sc3000_account:nag:dismiss");
    assert!(!h.app.help.sc3000_account.nag);
    // Save now runs the real save (here into the scenario's folder), and a saved design is not nagged.
    let dir = h.dir.clone();
    h.app.services.pick_save = Some(Box::new(move |name, _| Box::pin(std::future::ready(Some(dir.join(name).to_string_lossy().into_owned())))));
    due(&mut h);
    click(&mut h, "sc3000_account:nag:save");
    h.settle();
    h.frames(3);
    assert!(!crate::documents::any_dirty(&h.app), "the design was saved");
    assert!(h.app.session.path.as_ref().is_some_and(|p| std::path::Path::new(p).exists()), "to a real file");
    assert!(!h.app.help.sc3000_account.nag);
}

#[test]
fn the_recovery_report_recovers_nothing_and_leaves_the_design_alone() {
    let _language = solidcraft_ui_text::scope("en");
    let (mut h, _) = harness(None);
    edit(&mut h);
    let before = (h.app.session.revision, h.app.session.doc.features.len(), h.app.session.doc.name.clone(), h.app.docs.count());
    h.app.help.sc3000_account.unsaved = RECOVERY_SECONDS;
    h.frames(3);
    assert!(h.app.help.sc3000_account.recovery);
    assert!(h.widget("SolidCraft 3000 has recovered 0 of 47 hours of work.").is_some());
    assert!(shows(&h, "sc3000.account.parody"));
    click(&mut h, "sc3000_account:recovery:restore");
    assert!(!h.app.help.sc3000_account.recovery, "Restore 0 hours only closes it");
    let after = (h.app.session.revision, h.app.session.doc.features.len(), h.app.session.doc.name.clone(), h.app.docs.count());
    assert_eq!(before, after, "the design is untouched");
    assert!(crate::documents::any_dirty(&h.app), "and still unsaved: nothing was cleared");
    // Once per unsaved stretch.
    h.frames(10);
    assert!(!h.app.help.sc3000_account.recovery);
    // The other button closes it too.
    let account = &mut h.app.help.sc3000_account;
    account.recovered = false;
    account.unsaved = RECOVERY_SECONDS;
    h.frames(3);
    click(&mut h, "sc3000_account:recovery:close");
    assert!(!h.app.help.sc3000_account.recovery);
    assert_eq!(before, (h.app.session.revision, h.app.session.doc.features.len(), h.app.session.doc.name.clone(), h.app.docs.count()));
    edit(&mut h);
}

#[test]
fn every_string_is_in_english_and_german() {
    crate::i18n::install();
    let file: serde_json::Value = serde_json::from_str(include_str!("../../../assets/locales/pending/sc3000-account.json")).unwrap();
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
        assert_ne!(tr(id), shown, "{id} has German text");
    }
    // Every message the code names is in the file.
    let source = include_str!("sc3000_account.rs");
    let mut named = 0;
    for piece in source.split("\"sc3000.account.").skip(1) {
        let id = format!("sc3000.account.{}", &piece[..piece.find('"').unwrap()]);
        assert!(english.contains_key(&id), "{id} has no text");
        named += 1;
    }
    assert!(named >= english.len(), "{named} names for {} messages", english.len());
    let _language = solidcraft_ui_text::scope("de");
    assert_eq!(tr("sc3000.account.expired_title"), "Ihre Sitzung ist zu Ihrer Sicherheit abgelaufen");
}

#[test]
fn nothing_here_can_reach_the_network() {
    let source = include_str!("sc3000_account.rs");
    for word in ["open_url", "http", "fetch", "XMLHttpRequest", "sendBeacon", "WebSocket", "window.open", "tickets", "OpenUrl"] {
        assert!(!source.contains(word), "the account code mentions {word}");
    }
}

#[test]
fn dates_are_calendar_days() {
    assert_eq!(date(0), "1970-01-01");
    assert_eq!(date(NOW), "2026-11-24");
    assert_eq!(date(951_782_400), "2000-02-29");
}
