//! SolidCraft 3000: Cumulo, the cloud helper, and the updates that insist.
//!
//! **Cumulo** is a small cloud with a face who pops up now and then, after a command, with a tip
//! about what you just did that helps with nothing. "Tell me more" gives another one; Not now sends
//! it away for two minutes of use, Take a break for ten. It never covers the design's centre and never
//! takes a click meant for the CAD.
//!
//! **Updates.** After a few minutes of use, "SolidCraft 3000 3000.0.1 is available". Each Later
//! closes it, and the next time it asks a little more insistently, five levels in all. Later always
//! works, at every level. Update now fills a progress bar and then reports that the update is
//! installed and that nothing has changed, which is true: nothing is downloaded, installed or sent.
//!
//! Neither touches the design, keeps anything or makes a request. Saving and exporting work with
//! either open.

use solidcraft_ui_text::{format, tr};

use crate::SolveApp;
use crate::ui_kit;

/// Seconds of use before Cumulo's first visit.
pub const HELPER_FIRST: f64 = 45.0;
/// Seconds of use Not now and Take a break send Cumulo away for.
pub const HELPER_NOT_NOW: f64 = 120.0;
pub const HELPER_BREAK: f64 = 600.0;
/// Seconds of use before the first update notice, and between a Later and the next one.
pub const UPDATE_FIRST: f64 = 300.0;
pub const UPDATE_AGAIN: f64 = 180.0;
/// Seconds of use after an installed update before the next one is "available".
pub const UPDATE_NEXT: f64 = 900.0;
/// How long Update now's progress bar takes, in seconds.
pub const INSTALL_SECONDS: f64 = 4.0;

/// The update notice's levels of insistence, the mildest first.
pub const LEVELS: [&str; 5] =
    ["sc3000.helper.update_1", "sc3000.helper.update_2", "sc3000.helper.update_3", "sc3000.helper.update_4", "sc3000.helper.update_5"];

/// Tips that fit any moment, for Tell me more and commands with no tip of their own.
const GENERAL: [&str; 4] =
    ["sc3000.helper.tip_general_1", "sc3000.helper.tip_general_2", "sc3000.helper.tip_general_3", "sc3000.helper.tip_general_4"];

/// The tip for the last command `id` (a message id).
pub fn tip_for(id: &str) -> Option<&'static str> {
    Some(match id {
        "solid.extrude" => "sc3000.helper.tip_extrude",
        "solid.fillet" | "sketch.fillet" => "sc3000.helper.tip_fillet",
        "file.save" | "file.save_as" => "sc3000.helper.tip_save",
        _ if id.starts_with("sketch.") => "sc3000.helper.tip_sketch",
        _ if id.starts_with("appearance.") => "sc3000.helper.tip_appearance",
        _ if id.contains(".pattern.") => "sc3000.helper.tip_pattern",
        _ => return None,
    })
}

/// Seconds of use: counts while the window is focused; gaps over a second (a sleeping tab) count as one.
#[derive(Debug, Default)]
struct Clock {
    active: f64,
    previous: Option<f64>,
}

impl Clock {
    fn tick(&mut self, now: f64, focused: bool) {
        let delta = self.previous.map_or(0.0, |last| (now - last).clamp(0.0, 1.0));
        self.previous = Some(now);
        if focused {
            self.active += delta;
        }
    }
}

#[derive(Debug)]
pub struct Helper {
    clock: Clock,
    /// The tip shown (a message id), while Cumulo is out.
    pub tip: Option<&'static str>,
    /// When (seconds of use) Cumulo may come next, and whether it has said hello.
    next_at: f64,
    greeted: bool,
    /// The last command, for the next visit's tip; and how many general tips have been told.
    last: Option<&'static str>,
    told: usize,
}

impl Default for Helper {
    fn default() -> Self {
        Helper { clock: Clock::default(), tip: None, next_at: HELPER_FIRST, greeted: false, last: None, told: 0 }
    }
}

impl Helper {
    pub fn active(&self) -> f64 {
        self.clock.active
    }

    /// A command ran: Cumulo has something to say about it next time.
    pub fn command(&mut self, id: &str) {
        if let Some(tip) = tip_for(id) {
            self.last = Some(tip);
        }
    }

    pub fn tick(&mut self, now: f64, focused: bool) {
        self.clock.tick(now, focused);
        if self.tip.is_some() || self.clock.active < self.next_at {
            return;
        }
        if !self.greeted {
            self.greeted = true;
            self.tip = Some("sc3000.helper.hello");
        } else if let Some(tip) = self.last.take() {
            self.tip = Some(tip);
        }
    }

    /// Tell me more: another tip that helps with nothing.
    pub fn more(&mut self) {
        self.tip = Some(GENERAL[self.told % GENERAL.len()]);
        self.told += 1;
    }

    /// Not now (`false`) or Take a break (`true`).
    pub fn away(&mut self, long: bool) {
        self.tip = None;
        self.next_at = self.clock.active + if long { HELPER_BREAK } else { HELPER_NOT_NOW };
    }

    /// Bring Cumulo's next visit forward to now (for tests and the browser check).
    pub fn visit_now(&mut self) {
        self.next_at = self.clock.active;
    }
}

#[derive(Debug)]
pub struct Updates {
    clock: Clock,
    pub open: bool,
    /// How insistent the notice is (an index into [`LEVELS`]).
    pub level: usize,
    /// The patch number of the update on offer: SolidCraft 3000 3000.0.<n>.
    pub version: u32,
    /// Update now's progress (0 to 1), while it runs and once it is done.
    pub progress: Option<f64>,
    next_at: f64,
}

impl Default for Updates {
    fn default() -> Self {
        Updates { clock: Clock::default(), open: false, level: 0, version: 1, progress: None, next_at: UPDATE_FIRST }
    }
}

impl Updates {
    pub fn active(&self) -> f64 {
        self.clock.active
    }

    pub fn version_text(&self) -> String {
        std::format!("3000.0.{}", self.version)
    }

    pub fn installed(&self) -> bool {
        self.progress.is_some_and(|p| p >= 1.0)
    }

    pub fn tick(&mut self, now: f64, focused: bool) {
        let before = self.clock.active;
        self.clock.tick(now, focused);
        if let Some(progress) = &mut self.progress {
            *progress = (*progress + (self.clock.active - before) / INSTALL_SECONDS).min(1.0);
        }
        if !self.open && self.clock.active >= self.next_at {
            self.open = true;
        }
    }

    /// Later: closes the notice, at every level. The next one asks a little more firmly.
    pub fn later(&mut self) {
        self.open = false;
        self.progress = None;
        self.level = (self.level + 1).min(LEVELS.len() - 1);
        self.next_at = self.clock.active + UPDATE_AGAIN;
    }

    pub fn update_now(&mut self) {
        self.progress = Some(0.0);
    }

    /// Close after "installing": the next update is on offer in a while, politely again.
    pub fn done(&mut self) {
        self.open = false;
        self.progress = None;
        self.level = 0;
        self.version += 1;
        self.next_at = self.clock.active + UPDATE_NEXT;
    }

    /// Bring the next notice forward to now (for tests and the browser check).
    pub fn due_now(&mut self) {
        self.next_at = self.clock.active;
    }
}

pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let focused = ctx.input(|i| i.focused);
    let help = &mut app.help;
    // One parody at a time: neither comes while an account, credits or support page is open.
    let busy = help.sc3000_account.screen.is_some() || help.sc3000_account.recovery || help.sc3000_credits.top_up;
    help.sc3000_helper.tick(now, focused && !busy);
    help.sc3000_updates.tick(now, focused && !busy);
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
    if help.sc3000_updates.open {
        updates(&mut help.sc3000_updates, ctx);
    }
    if help.sc3000_helper.tip.is_some() {
        helper(&mut help.sc3000_helper, ctx);
    }
}

fn button(ui: &mut egui::Ui, id: &str, kind: ui_kit::Button, handle: &str) -> bool {
    let response = ui_kit::button(ui, tr(id), kind);
    crate::scenario::publish_target(handle, response.rect);
    response.clicked()
}

fn updates(updates: &mut Updates, ctx: &egui::Context) {
    let t = solidcraft_theme::Tokens::get();
    let version = updates.version_text();
    let mut later = false;
    let mut now = false;
    let mut done = false;
    let closed = ui_kit::Window::new(egui::Id::unique("sc3000_updates"), "help", tr("sc3000.helper.update_title")).show_body(ctx, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        match updates.progress {
            None => {
                let text = ui.label(egui::RichText::new(format(LEVELS[updates.level], &[("version", &version)])).strong());
                crate::scenario::publish_rect("sc3000_updates:text", text.rect);
                ui.horizontal_wrapped(|ui| {
                    now |= button(ui, "sc3000.helper.update_now", ui_kit::Button::Primary, "sc3000_updates:now");
                    later |= button(ui, "sc3000.helper.later", ui_kit::Button::Normal, "sc3000_updates:later");
                });
            }
            Some(progress) if progress < 1.0 => {
                ui.label(format("sc3000.helper.installing", &[("version", &version)]));
                ui.add(egui::ProgressBar::new(progress as f32).show_percentage());
                later |= button(ui, "sc3000.helper.later", ui_kit::Button::Normal, "sc3000_updates:later");
            }
            Some(_) => {
                let text = ui.label(egui::RichText::new(format("sc3000.helper.installed", &[("version", &version)])).strong());
                crate::scenario::publish_rect("sc3000_updates:installed", text.rect);
                done |= button(ui, "sc3000.account.close", ui_kit::Button::Primary, "sc3000_updates:close");
            }
        }
        ui.separator();
        ui.label(egui::RichText::new(tr("sc3000.helper.update_parody")).strong().color(t.warning));
    });
    if done || (closed && updates.installed()) {
        updates.done();
    } else if later || closed {
        updates.later();
    } else if now {
        updates.update_now();
    }
}

/// Cumulo: a small grey cloud with two eyes and a smile.
fn paint_cumulo(painter: &egui::Painter, rect: egui::Rect) {
    let t = solidcraft_theme::Tokens::get();
    let body = egui::Color32::from_gray(222);
    let ink = egui::Color32::from_gray(60);
    let c = rect.center();
    let r = rect.height() * 0.22;
    for (dx, dy, k) in [(-1.3, 0.35, 1.0), (1.3, 0.35, 1.0), (-0.45, -0.25, 1.35), (0.6, -0.05, 1.2)] {
        painter.circle(c + egui::vec2(dx * r, dy * r), r * k, body, egui::Stroke::new(1.0, t.border));
    }
    painter.rect_filled(egui::Rect::from_center_size(c + egui::vec2(0.0, 0.55 * r), egui::vec2(2.6 * r, 1.1 * r)), 0.0, body);
    for dx in [-0.45, 0.45] {
        painter.circle_filled(c + egui::vec2(dx * r, -0.05 * r), r * 0.14, ink);
    }
    let smile: Vec<egui::Pos2> = (0..=8)
        .map(|i| {
            let a = std::f32::consts::PI * (0.15 + 0.7 * i as f32 / 8.0);
            c + egui::vec2(a.cos() * 0.5 * r, 0.3 * r + a.sin() * 0.3 * r)
        })
        .collect();
    painter.add(egui::Shape::line(smile, egui::Stroke::new(1.5, ink)));
}

fn helper(helper: &mut Helper, ctx: &egui::Context) {
    let Some(tip) = helper.tip else { return };
    let t = solidcraft_theme::Tokens::get();
    let screen = ctx.content_rect();
    let (mut more, mut not_now, mut long) = (false, false, false);
    let shown = ui_kit::Container::new(ctx, ui_kit::Surface::Popup)
        .area(egui::Id::unique("sc3000_helper"))
        .order(egui::Order::Foreground)
        // Bottom right, clear of the view's toasts and the status bar.
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(egui::pos2(screen.right() - t.space_16, screen.bottom() - t.space_24 * 8.0))
        .show(ctx, |ui| {
            let frame = ui_kit::Container::popup_frame(ui.style());
            ui_kit::Container::show_backing(ui, frame, |ui| {
                ui.set_max_width(t.space_24 * 14.0);
                ui.horizontal_top(|ui| {
                    let (face, _) = ui.allocate_exact_size(egui::vec2(t.space_24 * 2.5, t.space_24 * 2.0), egui::Sense::hover());
                    paint_cumulo(ui.painter(), face);
                    crate::scenario::publish_rect("sc3000_helper:face", face);
                    ui.vertical(|ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.label(egui::RichText::new(tr("sc3000.helper.name")).strong().color(t.accent));
                        let text = ui.label(tr(tip));
                        crate::scenario::publish_rect("sc3000_helper:tip", text.rect);
                        ui.horizontal_wrapped(|ui| {
                            more |= button(ui, "sc3000.helper.more", ui_kit::Button::Primary, "sc3000_helper:more");
                            not_now |= button(ui, "sc3000.account.not_now", ui_kit::Button::Normal, "sc3000_helper:not_now");
                            long |= button(ui, "sc3000.helper.break", ui_kit::Button::Normal, "sc3000_helper:break");
                        });
                        ui.label(egui::RichText::new(tr("sc3000.helper.parody")).small().color(t.text_dim));
                    });
                });
            });
        });
    crate::scenario::publish_rect("sc3000_helper", shown.response.rect);
    if more {
        helper.more();
    } else if not_now || long {
        helper.away(long);
    }
}

#[cfg(test)]
#[path = "sc3000_helper_tests.rs"]
mod tests;
