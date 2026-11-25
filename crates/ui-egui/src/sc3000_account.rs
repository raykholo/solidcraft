//! SolidCraft 3000: the "Cloud Account". Sign in with any Cloud ID and any password, get signed out
//! "for your security" now and then, find that your password is suddenly incorrect, and reset it
//! under a short list of escalating rules. While the design has unsaved changes, a note asks every
//! fifteen seconds whether to back it up to the cloud, and after a long unsaved stretch a recovery
//! report proudly restores none of it.
//!
//! None of it is real. There is no account and no server: nothing here makes a request. The
//! password itself is never kept anywhere, not even in a text field's undo history; the account
//! keeps a Cloud ID, the password's length and when it was set, in the host's [`Store`] (the web
//! build's `localStorage`, guarded so a browser that blocks storage still works) or, without one,
//! for this run only. Every window and note closes with one click, none is modal, and none touches
//! the design: the recovery report only reports. Saving and exporting work as usual throughout.

use crate::SolveApp;
use crate::ui_kit;
use solidcraft_ui_text::{format, tr};

/// The least use (seconds with the window focused) between two sign-outs.
pub const SESSION_SECONDS: f64 = 600.0;
/// How often the back-up note asks while the design has unsaved changes, in seconds of use.
pub const NAG_SECONDS: f64 = 15.0;
/// Unsaved use (seconds) before the recovery report.
pub const RECOVERY_SECONDS: f64 = 1800.0;
/// The reset attempt that is accepted whatever it holds: the flow always ends.
pub const MERCY_ATTEMPT: usize = 8;
/// The longest Cloud ID and password the fields take.
const NAME_LIMIT: usize = 64;
const PASSWORD_LIMIT: usize = 64;
/// The longest old password the "longer than" rule asks to beat.
const LONGER_CAP: usize = 24;
/// Letters "fillet" needs: the forbidden letter is never one of them, so the rules always agree.
const FILLET: &str = "fillet";

/// Where the host keeps the account between runs: one text value. Reads that fail give `None`
/// and writes that fail give `false` (a browser that blocks or throws on storage); neither panics.
pub trait Store: Send + Sync {
    fn read(&self) -> Option<String>;
    fn write(&self, value: &str) -> bool;
}

/// Everything the account keeps: never the password, only its length and when it was set.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Saved {
    pub name: String,
    pub password_length: usize,
    /// When the password was set, in seconds since 1970.
    pub password_set_at: u64,
    pub signed_in: bool,
    /// Signed out "for your security": the old password is now "incorrect" until a reset.
    pub expired: bool,
}

/// The account window's page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    SignIn,
    Expired,
    Reset,
    Account,
}

/// One of the reset's rules, revealed one at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Fillet,
    Thousand,
    /// Not this letter. `true`: one the person just typed; `false`: the fallback.
    NoLetter(char, bool),
    /// Longer than this many characters (the old password's length, capped).
    Longer(usize),
}

impl Rule {
    pub fn holds(self, password: &str) -> bool {
        match self {
            Rule::Fillet => password.to_lowercase().contains(FILLET),
            Rule::Thousand => password.contains("3000"),
            Rule::NoLetter(letter, _) => !password.to_lowercase().contains(letter),
            Rule::Longer(length) => password.chars().count() > length,
        }
    }

    pub fn text(self) -> String {
        match self {
            Rule::Fillet => tr("sc3000.account.rule_fillet").to_string(),
            Rule::Thousand => tr("sc3000.account.rule_3000").to_string(),
            Rule::NoLetter(letter, typed) => {
                let id = if typed { "sc3000.account.rule_letter" } else { "sc3000.account.rule_letter_reserved" };
                format(id, &[("letter", &letter.to_string())])
            }
            Rule::Longer(length) => format("sc3000.account.rule_longer", &[("length", &length.to_string())]),
        }
    }
}

/// How many rules the reset has in all.
pub const RULES: usize = 4;

/// The reset in progress. Holds no password: each attempt is checked and dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reset {
    pub attempts: usize,
    pub rules: Vec<Rule>,
    /// What the last attempt was told (a message id), and the rule it broke, if any.
    pub message: Option<&'static str>,
    pub broken: Option<usize>,
}

pub struct Account {
    pub saved: Saved,
    store: Option<Box<dyn Store>>,
    /// Whether the account's last write reached the store (false: kept for this run only).
    pub kept: bool,
    /// A number in 0..1 for the sign-out's timing (the browser's `Math.random`).
    pub random: fn() -> f64,
    /// Seconds since 1970, for when a password was set.
    pub clock: fn() -> u64,
    pub screen: Option<Screen>,
    /// The fields. The password is taken out and dropped on every submit and close.
    pub name: String,
    pub password: String,
    /// The sign-in's answer (a message id).
    pub error: Option<&'static str>,
    /// A note on the account page (a message id): the reset's outcome.
    pub notice: Option<&'static str>,
    pub reset: Reset,
    /// Seconds of use (window focused), and the last frame's time.
    active: f64,
    previous: Option<f64>,
    /// When (in seconds of use) the next sign-out is due, and how many there have been.
    next_expiry: f64,
    pub expiries: u32,
    /// The back-up note: whether it shows, and when (seconds of use) it asks next.
    pub nag: bool,
    nag_at: f64,
    /// A save was asked for (Ctrl+S, the menu, the note's Save now) since the last frame.
    save_seen: bool,
    /// Unsaved use so far, and whether this stretch has had its recovery report.
    unsaved: f64,
    recovered: bool,
    pub recovery: bool,
}

impl Default for Account {
    fn default() -> Self {
        Account::new(None, || 0.5, unix_now)
    }
}

/// Seconds since 1970 by the system clock.
pub fn unix_now() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `seconds` since 1970 as a calendar date, `YYYY-MM-DD` (UTC).
pub fn date(seconds: u64) -> String {
    // Howard Hinnant's days-to-civil.
    let z = (seconds / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    std::format!("{year:04}-{month:02}-{day:02}")
}

impl Account {
    /// The account as `store` last kept it (none, or unreadable: signed out).
    pub fn new(store: Option<Box<dyn Store>>, random: fn() -> f64, clock: fn() -> u64) -> Self {
        let saved = store.as_ref().and_then(|s| s.read()).and_then(|text| serde_json::from_str::<Saved>(&text).ok()).unwrap_or_default();
        let mut account = Account {
            saved,
            kept: store.is_some(),
            store,
            random,
            clock,
            screen: None,
            name: String::new(),
            password: String::new(),
            error: None,
            notice: None,
            reset: Reset::default(),
            active: 0.0,
            previous: None,
            next_expiry: 0.0,
            expiries: 0,
            nag: false,
            nag_at: NAG_SECONDS,
            save_seen: false,
            unsaved: 0.0,
            recovered: false,
            recovery: false,
        };
        account.next_expiry = account.session_length();
        account
    }

    /// Ten to twenty minutes of use: never less than [`SESSION_SECONDS`].
    fn session_length(&self) -> f64 {
        let roll = (self.random)();
        SESSION_SECONDS * (1.0 + if roll.is_finite() { roll.clamp(0.0, 1.0) } else { 0.5 })
    }

    fn persist(&mut self) {
        let text = serde_json::to_string(&self.saved).unwrap_or_default();
        self.kept = self.store.as_ref().is_some_and(|s| s.write(&text));
    }

    /// Open the account window on the page that fits.
    pub fn open(&mut self) {
        self.error = None;
        self.screen = Some(if self.saved.signed_in { Screen::Account } else { Screen::SignIn });
        if self.name.is_empty() {
            self.name = self.saved.name.clone();
        }
    }

    /// Close the window, dropping whatever was typed in the password field.
    pub fn close(&mut self) {
        self.screen = None;
        self.password.clear();
        self.error = None;
        self.notice = None;
    }

    /// Sign in with the fields. Any Cloud ID and password work, except after a sign-out "for your
    /// security": then every password is "incorrect" until it is reset.
    pub fn sign_in(&mut self) {
        let password = std::mem::take(&mut self.password);
        let name = self.name.trim().to_string();
        self.error = if name.is_empty() {
            Some("sc3000.account.need_id")
        } else if password.is_empty() {
            Some("sc3000.account.need_password")
        } else if self.saved.expired {
            Some("sc3000.account.incorrect")
        } else {
            None
        };
        if self.error.is_some() {
            return;
        }
        self.saved = Saved { name, password_length: password.chars().count(), password_set_at: (self.clock)(), signed_in: true, expired: false };
        self.persist();
        self.close();
    }

    pub fn sign_out(&mut self) {
        self.saved.signed_in = false;
        self.persist();
        self.close();
    }

    /// "Your session has expired for your security."
    pub fn expire(&mut self) {
        self.saved.signed_in = false;
        self.saved.expired = true;
        self.persist();
        self.password.clear();
        self.error = None;
        self.notice = None;
        self.screen = Some(Screen::Expired);
        self.expiries += 1;
    }

    pub fn start_reset(&mut self) {
        self.password.clear();
        self.error = None;
        self.reset = Reset::default();
        self.screen = Some(Screen::Reset);
    }

    /// One attempt at a new password. The first is always "your existing password"; each one that
    /// keeps every rule so far brings one more; once all [`RULES`] are out, one that keeps them all
    /// is accepted, and attempt [`MERCY_ATTEMPT`] is accepted whatever it is. Returns whether the
    /// password was changed.
    pub fn submit_reset(&mut self) -> bool {
        let password = std::mem::take(&mut self.password);
        let reset = &mut self.reset;
        reset.broken = None;
        if password.is_empty() {
            reset.message = Some("sc3000.account.empty_new");
            return false;
        }
        reset.attempts += 1;
        let mercy = reset.attempts >= MERCY_ATTEMPT;
        if reset.attempts == 1 && !mercy {
            reset.message = Some("sc3000.account.existing");
            reset.rules.push(Rule::Fillet);
            return false;
        }
        if !mercy {
            if let Some(broken) = reset.rules.iter().position(|rule| !rule.holds(&password)) {
                reset.broken = Some(broken);
                reset.message = Some("sc3000.account.breaks");
                return false;
            }
            if reset.rules.len() < RULES {
                let next = match reset.rules.len() {
                    1 => Rule::Thousand,
                    2 => {
                        // The last letter just typed that "fillet" does not need, or a fallback.
                        let typed = password.to_lowercase().chars().rev().find(|c| c.is_ascii_alphabetic() && !FILLET.contains(*c));
                        typed.map_or(Rule::NoLetter('q', false), |c| Rule::NoLetter(c, true))
                    }
                    _ => Rule::Longer(self.saved.password_length.min(LONGER_CAP)),
                };
                reset.rules.push(next);
                reset.message = Some("sc3000.account.new_rule");
                return false;
            }
        }
        if self.name.trim().is_empty() {
            self.name = if self.saved.name.is_empty() { tr("sc3000.account.default_id").to_string() } else { self.saved.name.clone() };
        }
        self.saved = Saved {
            name: self.name.trim().to_string(),
            password_length: password.chars().count(),
            password_set_at: (self.clock)(),
            signed_in: true,
            expired: false,
        };
        self.persist();
        self.reset = Reset::default();
        self.screen = Some(Screen::Account);
        self.notice = Some(if mercy { "sc3000.account.mercy" } else { "sc3000.account.changed" });
        true
    }

    /// A save was asked for: the back-up note goes until the next [`NAG_SECONDS`].
    pub fn saving(&mut self) {
        self.save_seen = true;
    }

    /// Seconds of use so far.
    pub fn active(&self) -> f64 {
        self.active
    }

    /// Bring the next sign-out forward to now (for tests and the browser check).
    pub fn expire_now(&mut self) {
        self.next_expiry = self.active;
    }

    /// Move the timers on to `now` (seconds): `focused` counts as use, `dirty` is whether the
    /// design has unsaved changes. Gaps over a second (a sleeping tab) count as one.
    pub fn tick(&mut self, now: f64, focused: bool, dirty: bool) {
        let delta = self.previous.map_or(0.0, |last| (now - last).clamp(0.0, 1.0));
        self.previous = Some(now);
        if focused {
            self.active += delta;
        }
        // Sign-outs wait for the account window and the recovery report to close.
        if self.active >= self.next_expiry && self.screen.is_none() && !self.recovery {
            self.expire();
            self.next_expiry = self.active + self.session_length();
        }
        if std::mem::take(&mut self.save_seen) {
            self.nag = false;
            self.nag_at = self.active + NAG_SECONDS;
        }
        if dirty {
            if !self.nag && self.active >= self.nag_at {
                self.nag = true;
            }
            if focused {
                self.unsaved += delta;
            }
            if self.unsaved >= RECOVERY_SECONDS && !self.recovered {
                self.recovered = true;
                self.recovery = true;
            }
        } else {
            self.nag = false;
            self.nag_at = self.nag_at.max(self.active + NAG_SECONDS);
            self.unsaved = 0.0;
            self.recovered = false;
        }
    }

    /// "Not now" on the back-up note.
    pub fn dismiss_nag(&mut self) {
        self.nag = false;
        self.nag_at = self.active + NAG_SECONDS;
    }
}

/// The account's link in the maintenance strip: "Sign in", or the Cloud ID once signed in.
pub fn strip(app: &mut SolveApp, ui: &mut egui::Ui) {
    let account = &mut app.help.sc3000_account;
    let t = solidcraft_theme::Tokens::get();
    let text = if account.saved.signed_in { account.saved.name.clone() } else { tr("sc3000.account.sign_in").to_string() };
    let link = ui.link(egui::RichText::new(text).size(t.font_small));
    crate::scenario::publish_target("sc3000_account:open", link.rect);
    if link.clicked() {
        account.open();
    }
}

/// Forget what a field's undo history holds (a password must not linger there).
fn forget_history(ui: &egui::Ui, id: egui::Id) {
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
        state.clear_undoer();
        egui::TextEdit::store_state(ui.ctx(), id, state);
    }
}

/// Cut `text` to `limit` characters.
fn limit(text: &mut String, limit: usize) {
    if let Some((at, _)) = text.char_indices().nth(limit) {
        text.truncate(at);
    }
}

/// The Cloud ID field; returns whether Enter was pressed in it.
fn name_field(ui: &mut egui::Ui, text: &mut String) -> bool {
    let (enter, response) = ui_kit::text_field_reserving(ui, text, tr("sc3000.account.cloud_id_hint"), 0.0);
    limit(text, NAME_LIMIT);
    crate::scenario::publish_target("sc3000_account:id", response.rect);
    enter
}

/// A password field that keeps nothing in its undo history; returns it and whether Enter was pressed.
fn password_field(ui: &mut egui::Ui, text: &mut String, handle: &str) -> (egui::Response, bool) {
    let response = ui_kit::password_field(ui, egui::Id::unique(handle), text, "");
    limit(text, PASSWORD_LIMIT);
    forget_history(ui, response.id);
    crate::scenario::publish_target(handle, response.rect);
    let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    (response, enter)
}

fn button(ui: &mut egui::Ui, id: &str, kind: ui_kit::Button, handle: &str) -> bool {
    let response = ui_kit::button(ui, tr(id), kind);
    crate::scenario::publish_target(handle, response.rect);
    response.clicked()
}

/// The parody line and what the account keeps: on every page.
fn footer(ui: &mut egui::Ui, kept: bool) {
    let t = solidcraft_theme::Tokens::get();
    ui.separator();
    let parody = ui.label(egui::RichText::new(tr("sc3000.account.parody")).strong().color(t.warning));
    crate::scenario::publish_rect("sc3000_account:parody", parody.rect);
    ui.label(egui::RichText::new(tr(if kept { "sc3000.account.kept" } else { "sc3000.account.kept_tab" })).small().color(t.text_dim));
}

pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let dirty = crate::documents::any_dirty(app);
    let now = ctx.input(|i| i.time);
    let focused = ctx.input(|i| i.focused);
    let account = &mut app.help.sc3000_account;
    account.tick(now, focused, dirty);
    // The timers count while the window is focused, even with nothing else moving.
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
    if account.screen.is_some() {
        window(account, ctx);
    }
    if account.recovery {
        recovery(account, ctx);
    }
    if app.help.sc3000_account.nag {
        match nag(ctx) {
            // Save now: the real save, as Ctrl+S runs it.
            Some(true) => crate::toolbar::save(app),
            Some(false) => app.help.sc3000_account.dismiss_nag(),
            None => {}
        }
    }
}

fn window(account: &mut Account, ctx: &egui::Context) {
    let Some(screen) = account.screen else { return };
    let t = solidcraft_theme::Tokens::get();
    let title = match screen {
        Screen::Expired => tr("sc3000.account.expired_title"),
        Screen::Reset => tr("sc3000.account.reset"),
        Screen::SignIn | Screen::Account => tr("sc3000.account.title"),
    };
    let mut close = false;
    let closed = ui_kit::Window::new(egui::Id::unique("sc3000_account"), "help", title).show_body(ctx, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        match screen {
            Screen::Expired => {
                ui.label(tr("sc3000.account.expired_body"));
                ui.horizontal_wrapped(|ui| {
                    if button(ui, "sc3000.account.sign_in", ui_kit::Button::Primary, "sc3000_account:expired:sign_in") {
                        account.name = account.saved.name.clone();
                        account.screen = Some(Screen::SignIn);
                    }
                    close |= button(ui, "sc3000.account.not_now", ui_kit::Button::Normal, "sc3000_account:expired:not_now");
                });
            }
            Screen::SignIn => {
                ui.label(tr("sc3000.account.cloud_id"));
                let name_entered = name_field(ui, &mut account.name);
                ui.label(tr("sc3000.account.password"));
                let (_, password_entered) = password_field(ui, &mut account.password, "sc3000_account:password");
                if let Some(error) = account.error {
                    ui.label(egui::RichText::new(tr(error)).strong().color(t.error));
                }
                let mut submit = name_entered || password_entered;
                ui.horizontal_wrapped(|ui| {
                    submit |= button(ui, "sc3000.account.sign_in", ui_kit::Button::Primary, "sc3000_account:submit");
                    close |= button(ui, "sc3000.account.not_now", ui_kit::Button::Normal, "sc3000_account:cancel");
                });
                let reset = ui.link(tr("sc3000.account.reset"));
                crate::scenario::publish_target("sc3000_account:reset", reset.rect);
                if submit {
                    account.sign_in();
                } else if reset.clicked() {
                    account.start_reset();
                }
            }
            Screen::Reset => {
                let reset = &account.reset;
                if let Some(message) = reset.message {
                    ui.label(egui::RichText::new(tr(message)).strong().color(if message == "sc3000.account.new_rule" { t.accent } else { t.error }));
                    if message == "sc3000.account.existing" {
                        ui.label(egui::RichText::new(tr("sc3000.account.existing_note")).small().color(t.text_dim));
                    }
                }
                if !reset.rules.is_empty() {
                    ui.label(tr("sc3000.account.rules"));
                    for (index, rule) in reset.rules.iter().enumerate() {
                        let broken = reset.broken == Some(index);
                        let text = std::format!("{} {}", if broken { "✗" } else { "•" }, rule.text());
                        let shown = ui.label(if broken { egui::RichText::new(text).color(t.error) } else { egui::RichText::new(text) });
                        crate::scenario::publish_rect(&std::format!("sc3000_account:rule:{index}"), shown.rect);
                    }
                }
                ui.label(tr("sc3000.account.new_password"));
                let (password, mut submit) = password_field(ui, &mut account.password, "sc3000_account:new_password");
                ui.horizontal_wrapped(|ui| {
                    submit |= button(ui, "sc3000.account.change", ui_kit::Button::Primary, "sc3000_account:change");
                    close |= button(ui, "sc3000.account.not_now", ui_kit::Button::Normal, "sc3000_account:cancel");
                });
                if submit {
                    account.submit_reset();
                    password.request_focus();
                }
            }
            Screen::Account => {
                if let Some(notice) = account.notice {
                    ui.label(egui::RichText::new(tr(notice)).strong().color(t.accent));
                }
                ui.label(egui::RichText::new(format("sc3000.account.signed_in_as", &[("name", &account.saved.name)])).strong());
                ui.label(format(
                    "sc3000.account.password_facts",
                    &[("length", &account.saved.password_length.to_string()), ("date", &date(account.saved.password_set_at))],
                ));
                ui.horizontal_wrapped(|ui| {
                    if button(ui, "sc3000.account.sign_out", ui_kit::Button::Normal, "sc3000_account:sign_out") {
                        account.sign_out();
                    }
                    close |= button(ui, "sc3000.account.close", ui_kit::Button::Normal, "sc3000_account:close");
                });
            }
        }
        footer(ui, account.kept);
    });
    if close || closed {
        account.close();
    }
}

/// "SolidCraft 3000 has recovered 0 of 47 hours of work." Only a report: both buttons close it.
fn recovery(account: &mut Account, ctx: &egui::Context) {
    let t = solidcraft_theme::Tokens::get();
    let mut close = false;
    let closed = ui_kit::Window::new(egui::Id::unique("sc3000_account_recovery"), "help", tr("sc3000.account.recovery_title")).show_body(ctx, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.label(egui::RichText::new(tr("sc3000.account.recovery")).strong());
        ui.label(tr("sc3000.account.recovery_note"));
        ui.horizontal_wrapped(|ui| {
            close |= button(ui, "sc3000.account.restore", ui_kit::Button::Primary, "sc3000_account:recovery:restore");
            close |= button(ui, "sc3000.account.close", ui_kit::Button::Normal, "sc3000_account:recovery:close");
        });
        ui.separator();
        ui.label(egui::RichText::new(tr("sc3000.account.parody")).strong().color(t.warning));
    });
    if close || closed {
        account.recovery = false;
    }
}

/// The back-up note, a toast at the bottom centre of the window. Returns `Some(true)` for Save now
/// and `Some(false)` for Not now or its close.
fn nag(ctx: &egui::Context) -> Option<bool> {
    use ui_kit::toast::{Action, Extras, Lead, Place};
    let t = solidcraft_theme::Tokens::get();
    let screen = ctx.content_rect();
    // Above the status bar and the view's own toasts' usual place.
    let free = egui::Rect::from_min_max(screen.min + egui::vec2(t.space_16, 0.0), screen.max - egui::vec2(t.space_16, t.space_24 * 3.0));
    let actions = [(tr("sc3000.account.save_now"), Action::Primary), (tr("sc3000.account.not_now"), Action::Dismiss)];
    let extras = Extras { close: Some("sc3000_account_nag"), close_label: Some(tr("sc3000.account.not_now")), ..Extras::default() };
    let shown = egui::Area::new(egui::Id::unique("sc3000_account_nag"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| ui_kit::toast::show(ui, free, Place::Centre, tr("sc3000.account.nag").to_string(), Lead::Dot(false), &actions, extras))
        .inner?;
    crate::scenario::publish_rect("sc3000_account:nag", shown.rect);
    for (handle, rect) in ["sc3000_account:nag:save", "sc3000_account:nag:dismiss"].into_iter().zip(&shown.buttons) {
        crate::scenario::publish_target(handle, *rect);
    }
    match shown.clicked {
        Some(0) => Some(true),
        Some(_) => Some(false),
        None => shown.closed.then_some(false),
    }
}

#[cfg(test)]
#[path = "sc3000_account_tests.rs"]
mod tests;
