//! Temporary parody form. Deliberately has no browser, persistence, logging or service APIs.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
use solidcraft_ui_egui::ui_kit::{self, Container, Surface};
use solidcraft_ui_text::tr;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Step {
    Number,
    Code,
}

#[derive(Default)]
pub(crate) struct Phone {
    started: bool,
    due: Option<f64>,
    pub(crate) step: Option<Step>,
    pub(crate) number: String,
    pub(crate) code: String,
    country: usize,
    pub(crate) surprise_calls: bool,
}

impl Phone {
    /// Arm only after a design opens; wait for the other parody modals to finish.
    pub(crate) fn tick(&mut self, now: f64, design_open: bool, available: bool) {
        if !self.started && design_open {
            self.started = true;
            self.due = Some(now + 3.0);
        }
        if design_open && available && self.due.is_some_and(|due| now >= due) {
            self.due = None;
            self.step = Some(Step::Number);
            self.surprise_calls = true;
        }
    }

    pub(crate) fn send_code(&mut self) {
        self.number.clear();
        self.step = Some(Step::Code);
    }

    pub(crate) fn verify(&mut self) -> bool {
        if self.step == Some(Step::Code) && self.code.len() == 6 && self.code.bytes().all(|b| b.is_ascii_digit()) {
            self.clear();
            true
        } else {
            false
        }
    }

    fn clear(&mut self) {
        self.number.clear();
        self.code.clear();
        self.country = 0;
        self.surprise_calls = false;
        self.step = None;
    }

    pub(crate) fn skip(&mut self, now: f64) {
        self.clear();
        self.due = Some(now + 180.0);
    }

    /// Returns a local toast signal, never the input. Text is discarded on every transition.
    pub(crate) fn show(&mut self, ctx: &egui::Context, now: f64) -> bool {
        let Some(step) = self.step else { return false };
        let t = solidcraft_theme::Tokens::get();
        let mut verified = false;
        let response = Container::new(ctx, Surface::Window).modal(egui::Id::unique("sc3000_phone")).show(ctx, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.set_max_width((ctx.content_rect().width() - t.space_16 * 3.0).clamp(t.form_card_width / 3.0, t.form_card_width * 1.5));
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - t.space_24 * 2.0).show(ui, |ui| {
                ui.heading(tr("sc3000.phone.title"));
                ui.add_space(t.space_12);
                match step {
                    Step::Number => {
                        let countries =
                            [tr("sc3000.phone.us"), tr("sc3000.phone.uk"), tr("sc3000.phone.de"), tr("sc3000.phone.jp"), tr("sc3000.phone.other")];
                        ui_kit::choice_row(ui, tr("sc3000.phone.country"), "sc3000_phone_country", &mut self.country, &countries);
                        private_field(ui, "sc3000.phone.number", &mut self.number, 64);
                        if ui_kit::button(ui, tr("sc3000.phone.send"), ui_kit::Button::Normal).clicked() {
                            self.send_code();
                        }
                    }
                    Step::Code => {
                        ui.label(tr("sc3000.phone.enter"));
                        private_field(ui, "sc3000.phone.code", &mut self.code, 6);
                        ui.label(egui::RichText::new(tr("sc3000.phone.any")).small().color(t.text_dim));
                        if ui_kit::button(ui, tr("sc3000.phone.verify"), ui_kit::Button::Normal).clicked() {
                            verified = self.verify();
                        }
                    }
                }
                ui.add_space(t.space_12);
                ui.label(tr("sc3000.phone.terms"));
                ui_kit::checkbox(ui, tr("sc3000.phone.calls"), &mut self.surprise_calls);
                if ui.link(tr("sc3000.phone.skip")).clicked() {
                    self.skip(now);
                }
                ui.separator();
                ui.label(egui::RichText::new(tr("sc3000.phone.footer")).small().color(t.text_dim));
            });
        });
        if response.should_close() && !verified {
            self.skip(now);
        }
        verified
    }
}

/// Mask input in platform accessibility events and disable the editor's retained undo history.
fn private_field(ui: &mut egui::Ui, label: &str, value: &mut String, limit: usize) {
    ui_kit::row(ui, tr(label), |ui| {
        let mut output = egui::TextEdit::singleline(value).password(true).char_limit(limit).show(ui);
        output.state.clear_undoer();
        output.state.store(ui.ctx(), output.response.id);
    });
}
