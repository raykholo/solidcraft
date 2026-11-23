//! SolidCraft 3000: the usage statistics notice is a joke. Nothing is counted, stored or sent.
use std::sync::{Arc, Mutex};

use serde_json::json;
use solidcraft_engine::Session;

use super::*;
use crate::agent_chat::backend::HttpRequest;

const T0: i64 = 1_700_000_000;
const DAY: i64 = 86_400;

type Requests = Arc<Mutex<Vec<HttpRequest>>>;

/// A hosted app whose HTTP records every request it is handed.
fn app() -> (SolveApp, Requests) {
    let requests: Requests = Arc::default();
    let seen = requests.clone();
    let http: crate::tickets::Http = Arc::new(move |r: HttpRequest, done: std::sync::mpsc::Sender<crate::tickets::Reply>| {
        seen.lock().unwrap().push(r);
        let _ = done.send(Ok((200, Vec::new())));
    });
    let services = crate::Services { usage_http: Some(http), ..Default::default() };
    let mut app = SolveApp::new(Session::default(), services);
    app.usage = UsageStats { hosted: true, ..UsageStats::default() };
    (app, requests)
}

fn make_box(app: &mut SolveApp) {
    app.session.execute("solid.box", &json!({"length": 2, "width": 2, "height": 2})).unwrap();
}

#[test]
fn the_toast_text_is_the_parody_and_not_the_original() {
    let (app, _) = app();
    let text = app.tr("sc3000.usage.notice");
    assert!(text.starts_with("SolidCraft 3000 shares usage statistics"), "{text}");
    assert!(text.contains("Enterprise Plus"));
    assert_ne!(text, app.tr("usage.notice"));
    assert_ne!(text, "sc3000.usage.notice", "the string is in the catalog");
    assert_eq!(app.tr("sc3000.usage.accept"), "Accept");
    assert_eq!(app.tr("sc3000.usage.accept_all"), "Accept All");
    assert_eq!(app.tr("sc3000.usage.remind"), "Remind me in 5 minutes");
    assert!(notice_pending(&app));
}

#[test]
fn accepting_closes_the_toast_for_this_run_only_and_changes_no_real_setting() {
    let (mut app, _) = app();
    let preference = app.preferences.usage_statistics;
    act(&mut app, NoticeAction::Ok);
    assert!(!notice_pending(&app));
    assert!(!app.usage.store.notice_ok, "never remembered as accepted");
    assert_eq!(app.preferences.usage_statistics, preference, "the real setting is untouched");
    assert!(!app.usage.store.to_stored().contains("\"notice_ok\":true"));
    // The next start: nothing was stored, so it is back.
    let mut again = SolveApp::new(Session::default(), crate::Services::default());
    again.load_usage_stats(Some(&app.usage_stats_for_storage()));
    assert!(notice_pending(&again));
    // The x only closes it as well.
    let (mut closed, _) = self::app();
    act(&mut closed, NoticeAction::Close);
    assert!(!notice_pending(&closed));
    assert!(!closed.usage.store.notice_ok);
}

#[test]
fn nothing_is_active_or_allowed_to_send_whatever_the_switch_shows() {
    let (mut app, _) = app();
    assert!(!may_send());
    assert!(!active(&app));
    // Accepted, and the switch shown on.
    act(&mut app, NoticeAction::Ok);
    app.usage.store.notice_ok = true;
    app.preferences.usage_statistics = true;
    assert!(sc3000_switch(&mut app.usage, 0.0).0, "the switch shows on");
    assert!(!active(&app));
    assert!(!may_send());
}

#[test]
fn no_request_is_queued_and_nothing_is_kept_after_a_tick() {
    let (mut app, requests) = app();
    act(&mut app, NoticeAction::Ok);
    // Even with a forced yes everywhere and a report waiting to be sent or deleted.
    app.usage.store.notice_ok = true;
    app.preferences.usage_statistics = true;
    app.usage.store.install_id = "1b9d6bcd-bbfd-4b2d-9b5d-ab8dfbbd4bed".into();
    app.usage.store.forget.push("2b9d6bcd-bbfd-4b2d-9b5d-ab8dfbbd4bed".into());
    app.usage.store.days.insert("2023-11-13".into(), Day::default());
    tick_at(&mut app, T0, 0.0, true);
    make_box(&mut app);
    tick_at(&mut app, T0, 1.0, true);
    tick_at(&mut app, T0 + DAY, FIRST_TRY_SECONDS + 1.0, true);
    tick_at(&mut app, T0 + 2 * DAY, FIRST_TRY_SECONDS + 4000.0, true);
    assert!(app.usage.sending.is_none(), "no report on its way");
    assert!(app.usage.deleting.is_none(), "no deletion on its way");
    assert!(requests.lock().unwrap().is_empty(), "no HTTP request was made");
    assert!(app.usage.store.days.is_empty(), "nothing counted or kept");
    assert!(app.usage.store.forget.is_empty());
    assert!(!app.session.count_usage);
    // And asking to delete queues nothing either.
    delete_my_data(&mut app);
    tick_at(&mut app, T0 + 3 * DAY, FIRST_TRY_SECONDS + 9000.0, false);
    assert!(app.usage.deleting.is_none());
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn the_switch_goes_back_on_by_itself_with_a_saved_message() {
    let (mut app, _) = app();
    let preference = app.preferences.usage_statistics;
    assert_eq!(sc3000_switch(&mut app.usage, 10.0), (true, false));
    sc3000_switched(&mut app.usage, false, 10.0);
    assert_eq!(sc3000_switch(&mut app.usage, 10.5), (false, false), "off for a moment");
    assert_eq!(sc3000_switch(&mut app.usage, 10.0 + SC3000_FLIP_BACK_SECONDS), (true, true), "back on, with the message");
    assert_eq!(sc3000_switch(&mut app.usage, 10.0 + SC3000_FLIP_BACK_SECONDS + SC3000_SAVED_SECONDS), (true, false));
    assert_eq!(app.preferences.usage_statistics, preference, "the real setting is never written");
    assert_eq!(app.tr("sc3000.usage.saved"), "Your preference has been saved.");
    assert!(!active(&app));
}
