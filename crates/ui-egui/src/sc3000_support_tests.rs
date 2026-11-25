//! SolidCraft 3000: the Discord and Documentation links open the in-app Community Support chat,
//! and nothing in it leaves the app.
use serde_json::json;

use super::*;
use crate::scenario::{Harness, rect_of};

type Calls = std::sync::Arc<std::sync::atomic::AtomicUsize>;

/// A harness whose HTTP services count every call (the chat must make none).
fn harness_counting() -> (Harness, Calls) {
    let mut h = harness();
    let calls: Calls = Default::default();
    let seen = calls.clone();
    h.app.services.http_get = Some(std::sync::Arc::new(move |_| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Vec::new())
    }));
    let seen = calls.clone();
    h.app.services.tickets_http = Some(std::sync::Arc::new(move |_, done: std::sync::mpsc::Sender<crate::tickets::Reply>| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = done.send(Ok((200, Vec::new())));
    }));
    (h, calls)
}

fn harness() -> Harness {
    let mut h = Harness::new();
    h.app.preferences.language = "en".into();
    h.call("ui.resize", json!({"width":1600,"height":900}));
    h.app.home.open = false;
    h.settle();
    h
}

/// Frames for `seconds` of the harness clock (1/60 s a frame).
fn wait(h: &mut Harness, seconds: f64) {
    h.frames((seconds * 60.0).ceil() as usize + 2);
}

fn said(h: &Harness, who: Who) -> Vec<String> {
    h.app.help.sc3000_support.lines.iter().filter(|l| l.who == who).map(|l| l.text.clone()).collect()
}

/// Type `text` into the chat and press Enter.
fn say(h: &mut Harness, text: &str) {
    h.step(&json!({"click":{"handle":"sc3000_support:input"}})).unwrap();
    h.step(&json!({"text": text})).unwrap();
    h.step(&json!({"key":"Enter"})).unwrap();
    h.frames(2);
}

#[test]
fn every_discord_link_and_documentation_open_community_support_and_no_page() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    let open = |h: &mut Harness, how: &str, go: &dyn Fn(&mut Harness)| {
        h.app.help.sc3000_support.close();
        h.frames(2);
        assert!(!h.app.help.sc3000_support.open, "{how}: closed to begin with");
        go(h);
        h.frames(3);
        assert!(h.app.help.sc3000_support.open, "{how} opens Community Support");
        assert!(rect_of("sc3000_support:input").is_some(), "{how}: the chat is on screen");
        assert_eq!(h.take_opened(), Vec::<String>::new(), "{how} opens no page");
    };
    open(&mut h, "the title bar's Discord pill", &|h| h.step(&json!({"click":{"handle":"titlebar:discord"}})).unwrap());
    for label in ["Join the Discord…", "Documentation"] {
        open(&mut h, label, &|h| {
            h.app.help.menu = true;
            h.frames(2);
            let [x, y] = h.widget(label).unwrap_or_else(|| panic!("the Help menu lists {label}"));
            h.call("ui.click", json!({"x": x, "y": y}));
        });
    }
    open(&mut h, "About's Discord button", &|h| {
        h.app.help.about = true;
        h.frames(3);
        h.step(&json!({"click":{"handle":"about:discord"}})).unwrap();
    });
    h.app.help.about = false;
    open(&mut h, "the start page's Discord link", &|h| {
        h.app.home.open = true;
        h.frames(3);
        h.step(&json!({"click":{"handle":"home:discord"}})).unwrap();
    });
    // Ko-fi is not a support link: it still opens its real page.
    h.app.help.sc3000_support.close();
    h.frames(2);
    h.step(&json!({"click":{"handle":"home:kofi"}})).unwrap();
    assert_eq!(h.take_opened(), ["https://ko-fi.com/bherbruck"]);
    assert!(!h.app.help.sc3000_support.open);
}

#[test]
fn the_bots_answer_with_escalating_non_answers_and_nothing_leaves_the_app() {
    let _language = solidcraft_ui_text::scope("en");
    let (mut h, calls) = harness_counting();
    h.step(&json!({"click":{"handle":"titlebar:discord"}})).unwrap();
    h.frames(3);
    assert_eq!(said(&h, Who::Support), [tr("sc3000.support.greeting")]);
    assert!(said(&h, Who::Support)[0].contains("is your computer switched on?"));

    say(&mut h, "My fillet fails");
    assert_eq!(said(&h, Who::You), ["My fillet fails"]);
    assert!(h.app.help.sc3000_support.input.is_empty(), "the input is cleared on send");
    assert!(h.widget("Sunny is typing…").is_some(), "the bot types first");
    wait(&mut h, REPLY_SECONDS);
    assert_eq!(said(&h, Who::Support).last().unwrap(), "Have you tried signing out of the cloud and back in?");

    say(&mut h, "There is no cloud");
    wait(&mut h, REPLY_SECONDS + COMMUNITY_SECONDS);
    assert_eq!(said(&h, Who::Support).last().unwrap(), "Please attach your design, your designs’ designs, and a photo of your monitor.");
    assert_eq!(said(&h, Who::Fan), ["same problem here, any update?"]);

    for text in ["ok", "still broken", "hello?", "anyone?"] {
        say(&mut h, text);
        wait(&mut h, REPLY_SECONDS + COMMUNITY_SECONDS);
    }
    let support = said(&h, Who::Support);
    assert_eq!(&support[support.len() - 3..], [
        "Is your SolidCraft 3000 up to date? (It is now. It wasn’t 3 seconds ago.)",
        "Have you tried turning the cloud off and on again? Actually, please don’t. It’s the only one we have.",
        "Great question! This has been forwarded to the Enterprise Plus team.",
    ]);
    assert_eq!(said(&h, Who::Bumper), ["bump"]);
    assert_eq!(said(&h, Who::Notice), ["This thread has been marked Solved."]);
    assert_eq!(said(&h, Who::Fan).last().unwrap(), "how did you fix it??");

    // Solved, so it starts again from the top.
    say(&mut h, "it is not solved");
    wait(&mut h, REPLY_SECONDS);
    assert!(said(&h, Who::Support).last().unwrap().starts_with("This thread is Solved! To reopen it"));
    say(&mut h, "please");
    wait(&mut h, REPLY_SECONDS);
    assert_eq!(said(&h, Who::Support).last().unwrap(), "Have you tried signing out of the cloud and back in?");

    // A blank message sends nothing.
    let lines = h.app.help.sc3000_support.lines.len();
    say(&mut h, "   ");
    wait(&mut h, REPLY_SECONDS);
    assert_eq!(h.app.help.sc3000_support.lines.len(), lines);

    assert_eq!(h.take_opened(), Vec::<String>::new(), "no page opened");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "no HTTP request");
}

#[test]
fn closing_forgets_everything_typed() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    h.step(&json!({"click":{"handle":"titlebar:discord"}})).unwrap();
    h.frames(3);
    say(&mut h, "my secret part number 12345");
    h.step(&json!({"click":{"handle":"sc3000_support:input"}})).unwrap();
    h.step(&json!({"text": "half-typed"})).unwrap();
    h.frames(2);
    assert_eq!(h.app.help.sc3000_support.input, "half-typed");
    h.app.help.sc3000_support.close();
    h.frames(3);
    let chat = &h.app.help.sc3000_support;
    assert!(!chat.open && chat.lines.is_empty() && chat.input.is_empty() && chat.typing().is_none());
    // Nothing typed reached the preferences either.
    assert!(!serde_json::to_string(&h.app.preferences).unwrap().contains("12345"));
    // Opened again, it starts fresh with the greeting.
    h.step(&json!({"click":{"handle":"titlebar:discord"}})).unwrap();
    h.frames(3);
    assert_eq!(h.app.help.sc3000_support.lines.len(), 1);
    assert_eq!(h.take_opened(), Vec::<String>::new());
}

#[test]
fn the_parody_line_sends_real_support_to_solidcrafts_discord_only_when_clicked() {
    // German wraps the link onto its own row: the click must still land on it.
    for language in ["en", "de"] {
        let _language = solidcraft_ui_text::scope(language);
        let mut h = harness();
        h.app.preferences.language = language.into();
        h.settle();
        h.step(&json!({"click":{"handle":"titlebar:discord"}})).unwrap();
        h.frames(3);
        assert!(h.widget(&tr("sc3000.support.parody")).is_some(), "{language}");
        assert!(h.widget(&tr("sc3000.support.real_discord")).is_some(), "{language}");
        // The sentence's full stop stays on the link's row, right after it.
        let link = rect_of("sc3000_support:real_discord").expect("the link is drawn");
        assert!(
            h.widgets(".").iter().any(|[x, y]| (link.top()..=link.bottom()).contains(&(*y as f32)) && (*x as f32) > link.right() && (*x as f32) < link.right() + 20.0),
            "{language}: the full stop follows the link on its row"
        );
        wait(&mut h, 3.0);
        assert_eq!(h.take_opened(), Vec::<String>::new(), "{language}: nothing opens by itself");
        h.step(&json!({"click":{"handle":"sc3000_support:real_discord"}})).unwrap();
        h.frames(2);
        assert_eq!(h.take_opened(), [REAL_DISCORD_URL], "{language}");
    }
    // SolidCraft's own server (its README's link), the same one Join the Discord opens.
    assert_eq!(REAL_DISCORD_URL, "https://discord.gg/FtU4NQNVDH");
    assert_eq!(REAL_DISCORD_URL, crate::links::DISCORD_URL);
}

#[test]
fn every_line_is_in_english_and_german() {
    crate::i18n::install();
    let ids = [
        "attach", "bot_name", "bot_role", "bump", "cloud", "forwarded", "greeting", "hint", "how", "member_role", "moderating",
        "parody", "private", "real_discord", "reopen", "same", "send", "signout", "solved", "title", "typing", "update", "you",
    ];
    for (_, id, community) in SCRIPT {
        let short = id.trim_start_matches("sc3000.support.");
        assert!(ids.contains(&short), "{id} is listed");
        if let Some((_, id)) = community {
            assert!(ids.contains(&id.trim_start_matches("sc3000.support.")), "{id} is listed");
        }
    }
    for id in ids {
        let id = format!("sc3000.support.{id}");
        let english = {
            let _language = solidcraft_ui_text::scope("en");
            tr(&id).to_string()
        };
        let german = {
            let _language = solidcraft_ui_text::scope("de");
            tr(&id).to_string()
        };
        assert_ne!(english, id, "{id} has English text");
        if id != "sc3000.support.bot_name" {
            assert_ne!(german, english, "{id} has German text");
        }
    }
    let _language = solidcraft_ui_text::scope("de");
    assert_eq!(tr("sc3000.support.title"), "SolidCraft 3000 Community-Support");
}

#[test]
fn no_other_company_is_named() {
    crate::i18n::install();
    for language in ["en", "de"] {
        let _language = solidcraft_ui_text::scope(language);
        for (_, id, community) in SCRIPT {
            let mut texts = vec![tr(id).to_lowercase()];
            if let Some((_, id)) = community {
                texts.push(tr(id).to_lowercase());
            }
            for text in texts {
                for name in ["autodesk", "fusion", "solidworks", "onshape"] {
                    assert!(!text.contains(name), "{language}: {text}");
                }
            }
        }
    }
}

#[test]
fn the_whole_first_exchange_shows_without_a_message_cut_off_under_the_header() {
    let _language = solidcraft_ui_text::scope("en");
    for (width, height) in [(1600.0, 900.0), (1280.0, 720.0)] {
        let mut h = harness();
        h.call("ui.resize", json!({"width": width, "height": height}));
        h.settle();
        h.step(&json!({"click":{"handle":"titlebar:discord"}})).unwrap();
        h.frames(3);
        say(&mut h, "My fillet fails");
        wait(&mut h, REPLY_SECONDS);
        say(&mut h, "There is no cloud");
        wait(&mut h, REPLY_SECONDS + COMMUNITY_SECONDS);
        h.frames(3);
        let lines = h.app.help.sc3000_support.lines.len();
        assert_eq!(lines, 6, "greeting, two questions, two answers and a community reply");
        let shown = rect_of("sc3000_support:lines").expect("the chat's lines");
        for index in 0..lines {
            let line = rect_of(&format!("sc3000_support:line:{index}")).unwrap();
            assert!(shown.contains_rect(line), "{width}x{height}: line {index} {line:?} is cut off by {shown:?}");
        }
        // The whole window sits below the top chrome (the maintenance strip's text is still in
        // view) and ends on screen.
        let window = rect_of("dialog:window").expect("the chat window");
        let chrome = chrome_bottom(&h.ctx);
        assert!(chrome > 0.0, "the app bar reports where the top chrome ends");
        assert!(window.top() >= chrome, "{width}x{height}: the chat {window:?} covers the top chrome ending at {chrome}");
        assert!(window.bottom() <= height as f32, "{width}x{height}: the chat {window:?} runs off the screen");
        let strip = h.widget(&tr("sc3000.maintenance")).expect("the maintenance strip");
        assert!((strip[1] as f32) < window.top(), "{width}x{height}: the maintenance strip at {strip:?} is under the chat {window:?}");
    }
}

#[test]
fn the_start_pages_back_link_names_an_untitled_design_like_its_tab() {
    let _language = solidcraft_ui_text::scope("de");
    let mut h = Harness::new();
    h.app.preferences.language = "de".into();
    h.call("ui.resize", json!({"width":1600,"height":900}));
    h.app.home.open = true;
    h.frames(4);
    assert_eq!(tr("ui.untitled"), "Ohne Titel");
    assert!(h.widget("Zurück zu Ohne Titel").is_some(), "the back link says Ohne Titel, like the tab");
    assert!(h.widget("Zurück zu Untitled").is_none());
}

#[test]
fn the_pages_parody_note_speaks_every_shipped_language() {
    crate::i18n::install();
    let script = include_str!("../../../packaging/sc3000-note.sh");
    for id in ["id=\"sc3000-note-before\"", "id=\"sc3000-note-after\"", "dir=\"auto\"", "https://github.com/tangency-labs/solidcraft"] {
        assert!(script.contains(id), "the note has {id}");
    }
    let english = {
        let _language = solidcraft_ui_text::scope("en");
        (tr("sc3000.web.note_before").to_string(), tr("sc3000.web.note_after").to_string())
    };
    assert!(english.0.starts_with("SolidCraft 3000 is a parody") && script.contains(&english.0) && script.contains(&english.1), "English matches the page's own text");
    let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/locales");
    let mut checked = 0;
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let locale = path.file_stem().unwrap().to_string_lossy().into_owned();
        let _language = solidcraft_ui_text::scope(&locale);
        let (before, after) = (tr("sc3000.web.note_before"), tr("sc3000.web.note_after"));
        assert!(before.contains("SolidCraft 3000"), "{locale}: {before}");
        assert!(!before.contains('<') && !after.contains('<'), "{locale}: plain text only");
        if !locale.starts_with("en") {
            assert_ne!((before.to_string(), after.to_string()), english, "{locale} is translated");
        }
        checked += 1;
    }
    assert!(checked >= 38, "{checked} locales");
}
