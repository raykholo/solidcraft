//! SolidCraft 3000: "SolidCredits". Some features now carry a price: an extrude costs so much per
//! millimetre, a pattern so much per instance, fillets and shells are "Pro", and a few appearances
//! are in the "Appearance Pack". After such a command a note says what it cost and that, as the
//! balance is 0.00, it went on the free trial (0 days left). The Cloud Account shows the balance
//! and a Top up button, whose page only says top-ups are paused.
//!
//! None of it is real and none of it gets in the way. The note comes after the command has run:
//! nothing is ever refused, delayed or undone, and there is nothing to buy: no prices in money, no
//! payment fields, no request leaves the page. Saving and exporting are never charged.

use serde_json::Value;
use solidcraft_ui_text::{format, tr};

use crate::SolveApp;
use crate::ui_kit;

/// SolidCredits per millimetre of extrude.
pub const PER_MM: f64 = 0.1;
/// SolidCredits per pattern instance.
pub const PER_INSTANCE: f64 = 2.0;
/// The flat prices of the "Pro" features.
pub const FILLET: f64 = 5.0;
pub const SHELL: f64 = 8.0;
/// The price of an Appearance Pack appearance.
pub const APPEARANCE: f64 = 3.0;

/// The library appearances in the Appearance Pack: those whose name starts with one of these.
const PACK: [&str; 3] = ["Brass", "Copper", "Titanium"];

/// Whether the library appearance `name` is in the Appearance Pack.
pub fn in_pack(name: &str) -> bool {
    PACK.iter().any(|metal| name.starts_with(metal))
}

/// What one command "cost": the note's first line and the amount.
#[derive(Clone, Debug, PartialEq)]
pub struct Charge {
    pub text: String,
    pub amount: f64,
}

#[derive(Debug, Default)]
pub struct Credits {
    /// Everything "charged to the free trial" this run.
    pub charged: f64,
    /// The note after the last priced command, until OK, Top up or its close.
    pub note: Option<Charge>,
    /// The Top up page is open.
    pub top_up: bool,
}

/// `value` with two decimals, in the reader's language's style (a decimal comma in German).
pub fn amount(value: f64) -> String {
    let text = std::format!("{value:.2}");
    if solidcraft_ui_text::language().starts_with("de") { text.replace('.', ",") } else { text }
}

/// A number from a command's parameter: a number, or an expression the design evaluates.
fn number(app: &SolveApp, value: Option<&Value>) -> Option<f64> {
    use solidcraft_engine::doc::expr::Kind;
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => app.session.doc.eval(text, Kind::Unitless).ok(),
        _ => None,
    }
}

/// The price of command `id` with `params`, which has just run, or `None` for one that is free.
pub fn price(app: &SolveApp, id: &str, params: &Value) -> Option<Charge> {
    let charge = |text: String, amount: f64| Some(Charge { text, amount });
    match id {
        "solid.extrude" => {
            let length = solidcraft_engine::extrude_reach(&app.session, params)?.abs();
            let cost = (length * PER_MM * 100.0).round() / 100.0;
            charge(format("sc3000.credits.extrude", &[("cost", &amount(cost)), ("length", &amount(length)), ("rate", &amount(PER_MM))]), cost)
        }
        "solid.pattern.circular" | "solid.pattern.rectangular" | "solid.pattern.path" | "sketch.pattern.circular" | "sketch.pattern.rectangular" => {
            let first = number(app, params.get("count").or_else(|| params.get("count1"))).unwrap_or(2.0).round().max(1.0);
            let second = number(app, params.get("count2")).unwrap_or(1.0).round().max(1.0);
            let instances = first * second;
            let cost = instances * PER_INSTANCE;
            charge(
                format(
                    "sc3000.credits.pattern",
                    &[("cost", &amount(cost)), ("count", &std::format!("{instances}")), ("rate", &amount(PER_INSTANCE))],
                ),
                cost,
            )
        }
        "solid.fillet" | "sketch.fillet" => charge(format("sc3000.credits.fillet", &[("cost", &amount(FILLET))]), FILLET),
        "solid.shell" => charge(format("sc3000.credits.shell", &[("cost", &amount(SHELL))]), SHELL),
        "appearance.assign" => {
            let name = params.get("appearance").or_else(|| params.get("name")).and_then(Value::as_str).filter(|name| in_pack(name))?;
            charge(format("sc3000.credits.appearance", &[("cost", &amount(APPEARANCE)), ("name", name)]), APPEARANCE)
        }
        _ => None,
    }
}

/// After a command ran: put its price, if it has one, on the trial and show the note.
pub fn after_command(app: &mut SolveApp, id: &str, params: &Value) {
    if let Some(charge) = price(app, id, params) {
        let credits = &mut app.help.sc3000_credits;
        credits.charged += charge.amount;
        credits.note = Some(charge);
    }
}

/// The balance lines and Top up button on the Cloud Account's page.
pub fn account_rows(credits: &mut Credits, ui: &mut egui::Ui) {
    let t = solidcraft_theme::Tokens::get();
    let balance = ui.label(egui::RichText::new(format("sc3000.credits.balance", &[("balance", &amount(0.0))])).strong());
    crate::scenario::publish_rect("sc3000_credits:balance", balance.rect);
    ui.label(egui::RichText::new(format("sc3000.credits.owed", &[("total", &amount(credits.charged))])).small().color(t.text_dim));
    let response = ui_kit::button(ui, tr("sc3000.credits.top_up"), ui_kit::Button::Normal);
    crate::scenario::publish_target("sc3000_credits:account:top_up", response.rect);
    if response.clicked() {
        credits.top_up = true;
    }
}

/// A small "PRO" tag in the top-right corner of `rect` (an Appearance Pack swatch).
pub fn badge(ui: &egui::Ui, rect: egui::Rect) {
    let t = solidcraft_theme::Tokens::get();
    let galley = ui.painter().layout_no_wrap(tr("sc3000.credits.pro").to_string(), egui::FontId::proportional(t.font_small * 0.8), t.on_accent);
    let size = galley.size() + egui::vec2(t.space_4, t.space_2);
    let tag = egui::Rect::from_min_size(egui::pos2(rect.right() - size.x, rect.top()), size);
    ui.painter().rect_filled(tag, t.space_2, t.accent);
    ui.painter().galley(tag.center() - galley.size() / 2.0, galley, t.on_accent);
}

pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let credits = &mut app.help.sc3000_credits;
    if credits.top_up {
        top_up(credits, ctx);
    }
    if let Some(charge) = credits.note.clone() {
        match note(ctx, &charge) {
            Some(true) => {
                credits.note = None;
                credits.top_up = true;
            }
            Some(false) => credits.note = None,
            None => {}
        }
    }
}

/// "Top-ups are paused." Nothing to fill in and nothing to buy: Close is all it has.
fn top_up(credits: &mut Credits, ctx: &egui::Context) {
    let t = solidcraft_theme::Tokens::get();
    let mut close = false;
    let closed = ui_kit::Window::new(egui::Id::unique("sc3000_credits_top_up"), "help", tr("sc3000.credits.top_up_title")).show_body(ctx, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.label(egui::RichText::new(format("sc3000.credits.balance", &[("balance", &amount(0.0))])).strong());
        ui.label(tr("sc3000.credits.top_up_body"));
        ui.label(egui::RichText::new(tr("sc3000.credits.top_up_note")).small().color(t.text_dim));
        let response = ui_kit::button(ui, tr("sc3000.account.close"), ui_kit::Button::Primary);
        crate::scenario::publish_target("sc3000_credits:top_up:close", response.rect);
        close |= response.clicked();
        ui.separator();
        let parody = ui.label(egui::RichText::new(tr("sc3000.account.parody")).strong().color(t.warning));
        crate::scenario::publish_rect("sc3000_credits:top_up:parody", parody.rect);
    });
    if close || closed {
        credits.top_up = false;
    }
}

/// The note after a priced command, at the bottom centre above the back-up note's place. Returns
/// `Some(true)` for Top up and `Some(false)` for OK or its close.
fn note(ctx: &egui::Context, charge: &Charge) -> Option<bool> {
    use ui_kit::toast::{Action, Extras, Lead, Place};
    let t = solidcraft_theme::Tokens::get();
    let screen = ctx.content_rect();
    let free = egui::Rect::from_min_max(screen.min + egui::vec2(t.space_16, 0.0), screen.max - egui::vec2(t.space_16, t.space_24 * 6.0));
    let text = std::format!("{}\n{}", charge.text, format("sc3000.credits.trial", &[("balance", &amount(0.0))]));
    let actions = [(tr("sc3000.credits.top_up"), Action::Primary), (tr("sc3000.credits.ok"), Action::Dismiss)];
    let extras = Extras { close: Some("sc3000_credits_note"), close_label: Some(tr("sc3000.credits.ok")), ..Extras::default() };
    let shown = ui_kit::Container::new(ctx, ui_kit::Surface::Popup)
        .area(egui::Id::unique("sc3000_credits_note"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| ui_kit::toast::show(ui, free, Place::Centre, text, Lead::Dot(false), &actions, extras))
        .inner?;
    crate::scenario::publish_rect("sc3000_credits:note", shown.rect);
    for (handle, rect) in ["sc3000_credits:note:top_up", "sc3000_credits:note:ok"].into_iter().zip(&shown.buttons) {
        crate::scenario::publish_target(handle, *rect);
    }
    match shown.clicked {
        Some(0) => Some(true),
        Some(_) => Some(false),
        None => shown.closed.then_some(false),
    }
}

#[cfg(test)]
#[path = "sc3000_credits_tests.rs"]
mod tests;
