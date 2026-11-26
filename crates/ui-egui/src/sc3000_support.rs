//! SolidCraft 3000: "Community Support". Every Discord link in the app (the title bar pill, the Help
//! menu, About and the start page) and Help › Documentation open this in-app chat instead of a
//! browser tab. A cheerful support bot and two community members answer whatever you type with
//! escalating non-answers.
//!
//! Nothing here leaves the app: no request, no storage, no log. What you type lives in this window
//! only and is gone when it closes. The only link out is the real Discord, named in the parody line
//! at the top, and it opens only when that link is clicked.

use crate::SolveApp;
use crate::ui_kit;
use solidcraft_ui_text::tr;

/// SolidCraft's own Discord, where real support is: named in the parody line.
pub const REAL_DISCORD_URL: &str = crate::links::DISCORD_URL;
/// How long the support bot "types" before it answers, in seconds.
pub const REPLY_SECONDS: f64 = 1.2;
/// How long after the support bot a community member chimes in, in seconds.
pub const COMMUNITY_SECONDS: f64 = 1.4;
/// Longest message the input takes.
/// The chat window's rows other than its lines (title, parody header, input), in control heights.
const CHROME_ROWS: f32 = 6.0;
const INPUT_LIMIT: usize = 500;

/// Who said it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    You,
    Support,
    /// "CloudFan2003", a community member.
    Fan,
    /// "xX_Extrude_Xx", a community member.
    Bumper,
    /// A thread notice ("marked Solved").
    Notice,
}

/// One line of the chat: who, and either a message id (a bot) or the person's own text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub who: Who,
    pub text: String,
}

impl Line {
    fn bot(who: Who, id: &str) -> Self {
        Self { who, text: tr(id).to_string() }
    }
}

/// The support bot's answers, in order; the conversation cycles through them. Each may bring a
/// community member along a moment later.
const SCRIPT: &[(Who, &str, Option<(Who, &str)>)] = &[
    (Who::Support, "sc3000.support.signout", None),
    (Who::Support, "sc3000.support.attach", Some((Who::Fan, "sc3000.support.same"))),
    (Who::Support, "sc3000.support.update", None),
    (Who::Support, "sc3000.support.cloud", Some((Who::Bumper, "sc3000.support.bump"))),
    (Who::Support, "sc3000.support.forwarded", None),
    (Who::Notice, "sc3000.support.solved", Some((Who::Fan, "sc3000.support.how"))),
    (Who::Support, "sc3000.support.reopen", None),
];

/// The chat's state. UI state only, never stored.
#[derive(Default)]
pub struct Chat {
    pub open: bool,
    pub lines: Vec<Line>,
    pub input: String,
    /// How many messages the person has sent this time.
    sent: usize,
    /// Bot lines still "being typed", with the time each appears.
    pending: Vec<(f64, Line)>,
}

impl Chat {
    /// Open the chat, greeted by the support bot.
    pub fn open(&mut self) {
        if !self.open {
            *self = Chat { open: true, lines: vec![Line::bot(Who::Support, "sc3000.support.greeting")], ..Chat::default() };
        }
    }

    /// Close it and forget everything that was said.
    pub fn close(&mut self) {
        *self = Chat::default();
    }

    /// The person sends `input`; blank input sends nothing. The bots answer later (see [`Self::tick`]).
    pub fn send(&mut self, now: f64) {
        let text = self.input.trim().to_string();
        self.input.clear();
        if text.is_empty() {
            return;
        }
        self.lines.push(Line { who: Who::You, text });
        let (who, id, community) = SCRIPT[self.sent % SCRIPT.len()];
        self.sent += 1;
        let reply = now + REPLY_SECONDS;
        self.pending.push((reply, Line::bot(who, id)));
        if let Some((who, id)) = community {
            self.pending.push((reply + COMMUNITY_SECONDS, Line::bot(who, id)));
        }
    }

    /// Move the lines that are due into the chat.
    pub fn tick(&mut self, now: f64) {
        self.pending.sort_by(|a, b| a.0.total_cmp(&b.0));
        while self.pending.first().is_some_and(|(at, _)| *at <= now) {
            let (_, line) = self.pending.remove(0);
            self.lines.push(line);
        }
    }

    /// Who is typing now, if anyone, and when the next line is due.
    pub fn typing(&self) -> Option<(Who, f64)> {
        self.pending.iter().min_by(|a, b| a.0.total_cmp(&b.0)).map(|(at, line)| (line.who, *at))
    }
}

/// The links that open the chat instead of leaving the app.
pub fn replaces(link: &str) -> bool {
    matches!(link, "discord" | "docs")
}

/// Ask for the chat from code that has only the context (the link buttons).
pub fn request(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(request_id(), true));
}

fn request_id() -> egui::Id {
    egui::Id::unique("sc3000_support_requested")
}

fn name(who: Who) -> &'static str {
    match who {
        Who::You => tr("sc3000.support.you"),
        Who::Support => tr("sc3000.support.bot_name"),
        Who::Fan => "CloudFan2003",
        Who::Bumper => "xX_Extrude_Xx",
        Who::Notice => "",
    }
}

fn role(who: Who) -> &'static str {
    match who {
        Who::Support => tr("sc3000.support.bot_role"),
        Who::Fan | Who::Bumper => tr("sc3000.support.member_role"),
        Who::You | Who::Notice => "",
    }
}

/// Draw the chat if it is open (or was just asked for).
fn chrome_id() -> egui::Id {
    egui::Id::unique("sc3000_support_chrome_bottom")
}

/// Where the app's top chrome (the app bar and the maintenance strip, under the page's banner)
/// ends this frame: the chat opens below it. The app bar sets it as it draws.
pub fn set_chrome_bottom(ctx: &egui::Context, y: f32) {
    ctx.data_mut(|d| d.insert_temp(chrome_id(), y));
}

/// The bottom of the top chrome, or the top of the screen before the app bar has drawn.
pub fn chrome_bottom(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(chrome_id())).unwrap_or_else(|| ctx.content_rect().top())
}

pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let chat = &mut app.help.sc3000_support;
    if ctx.data_mut(|d| d.remove_temp::<bool>(request_id())).unwrap_or(false) {
        chat.open();
    }
    if !chat.open {
        return;
    }
    let now = ctx.input(|i| i.time);
    chat.tick(now);
    if let Some((_, at)) = chat.typing() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((at - now).max(0.0)));
    }
    let t = solidcraft_theme::Tokens::get();
    let title = tr("sc3000.support.title");
    // Below the app bar, the maintenance strip and the page's own banner, never over them: their
    // buttons (the banner's Dismiss) stay clickable while the chat is open.
    let screen = ctx.content_rect();
    let top = chrome_bottom(ctx).clamp(screen.top(), screen.bottom());
    let gap = t.space_8;
    let window = ui_kit::Window::new(egui::Id::unique("sc3000_support"), "help", title)
        .cards(2.0)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, top - screen.top() + gap));
    let closed = window.show_body(ctx, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        // The parody line, with a link to SolidCraft's own Discord.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(egui::RichText::new(tr("sc3000.support.parody")).strong().color(t.warning));
            ui.label(" ");
            // One piece with its full stop: a link split across two rows gets a box spanning both,
            // and its middle is empty space (German puts it at the end of a row).
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                let link = ui.link(egui::RichText::new(tr("sc3000.support.real_discord")).color(t.accent));
                crate::scenario::publish_target("sc3000_support:real_discord", link.rect);
                if link.clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(REAL_DISCORD_URL));
                }
                ui.label(egui::RichText::new(".").strong().color(t.warning));
            });
        });
        ui.label(egui::RichText::new(tr("sc3000.support.private")).small().color(t.text_dim));
        ui.separator();
        // Tall enough for a whole first exchange (greeting, two questions, the answers and a community
        // reply) on a laptop screen, so nothing starts out cut off under the header.
        let height = (screen.bottom() - top - gap * 2.0 - t.control_height * CHROME_ROWS).clamp(t.control_height * 4.0, t.control_height * 20.0);
        let scroll =
            egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, false]).stick_to_bottom(true).show(
                ui,
                |ui| {
                    ui.add_space(t.space_4);
                    for (index, line) in chat.lines.iter().enumerate() {
                        let shown = ui.scope(|ui| {
                            if line.who == Who::Notice {
                                ui.add_space(t.space_4);
                                ui.vertical_centered(|ui| ui.label(egui::RichText::new(format!("✔ {}", line.text)).strong().color(t.accent)));
                                ui.add_space(t.space_4);
                                return;
                            }
                            ui.horizontal_wrapped(|ui| {
                                let ink = if line.who == Who::You { t.text } else { t.accent };
                                ui.label(egui::RichText::new(name(line.who)).strong().color(ink));
                                let role = role(line.who);
                                if !role.is_empty() {
                                    ui.label(egui::RichText::new(role).small().color(t.text_dim));
                                }
                            });
                            ui.label(&line.text);
                        });
                        crate::scenario::publish_rect(&format!("sc3000_support:line:{index}"), shown.response.rect);
                        ui.add_space(t.space_8);
                    }
                    if let Some((who, _)) = chat.typing() {
                        let text = if who == Who::Notice {
                            tr("sc3000.support.moderating").to_string()
                        } else {
                            solidcraft_ui_text::format("sc3000.support.typing", &[("name", name(who))])
                        };
                        ui.label(egui::RichText::new(text).italics().color(t.text_dim));
                    }
                },
            );
        crate::scenario::publish_rect("sc3000_support:lines", scroll.inner_rect);
        ui.separator();
        ui.horizontal(|ui| {
            let send_width = t.control_height * 3.0;
            let mut output = egui::TextEdit::singleline(&mut chat.input)
                .hint_text(tr("sc3000.support.hint"))
                .char_limit(INPUT_LIMIT)
                .desired_width((ui.available_width() - send_width - t.space_8).max(t.control_height))
                .show(ui);
            // Nothing typed here is kept: not even in the editor's undo history.
            output.state.clear_undoer();
            output.state.store(ui.ctx(), output.response.id);
            crate::scenario::publish_target("sc3000_support:input", output.response.rect);
            let entered = output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let send = ui_kit::button(ui, tr("sc3000.support.send"), ui_kit::Button::Normal);
            crate::scenario::publish_target("sc3000_support:send", send.rect);
            if entered || send.clicked() {
                chat.send(now);
                output.response.request_focus();
            }
        });
    });
    if closed {
        chat.close();
    }
}

#[cfg(test)]
#[path = "sc3000_support_tests.rs"]
mod tests;
