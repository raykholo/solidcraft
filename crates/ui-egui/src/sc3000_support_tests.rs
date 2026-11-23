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
fn the_parody_line_names_the_real_discord_and_opens_it_only_when_clicked() {
    let _language = solidcraft_ui_text::scope("en");
    let mut h = harness();
    h.step(&json!({"click":{"handle":"titlebar:discord"}})).unwrap();
    h.frames(3);
    assert!(h.widget("Parody. Not a real support channel. The real Discord is").is_some());
    assert!(h.widget("discord.gg/yTcseBEdeu").is_some());
    wait(&mut h, 3.0);
    assert_eq!(h.take_opened(), Vec::<String>::new(), "nothing opens by itself");
    h.step(&json!({"click":{"handle":"sc3000_support:real_discord"}})).unwrap();
    h.frames(2);
    assert_eq!(h.take_opened(), [REAL_DISCORD_URL]);
    assert_eq!(REAL_DISCORD_URL, "https://discord.gg/yTcseBEdeu");
}

#[test]
fn every_line_is_in_english_and_german() {
    crate::i18n::install();
    let ids = [
        "attach", "bot_name", "bot_role", "bump", "cloud", "forwarded", "greeting", "hint", "how", "member_role", "moderating",
        "parody", "private", "reopen", "same", "send", "signout", "solved", "title", "typing", "update", "you",
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
