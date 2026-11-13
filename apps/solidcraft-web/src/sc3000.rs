//! Browser-local cloud theatre. No request or design data enters this state.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
use solidcraft_ui_egui::ui_kit::{self, Container, Surface};
use solidcraft_ui_text::{format, tr};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Scene {
    Update(f64),
    Expired,
    Sync(f64),
    Conflict,
    Outage(f64),
    FilletOutage,
}

pub(crate) fn update_progress(elapsed: f64) -> f32 {
    (elapsed / 2.0).clamp(0.0, 0.99) as f32
}

pub(crate) struct Cloud {
    pub(crate) scene: Option<Scene>,
    pub(crate) banner_at: f64,
    pub(crate) dismissals: u8,
    active: f64,
    previous: Option<f64>,
    next_session: f64,
    next_outage: f64,
    saves: u64,
    pending_save: bool,
    identity: String,
    pending_outage: bool,
    pending_fillet: bool,
    fillets_seen: u64,
    fillet_times: std::collections::VecDeque<f64>,
    fillet_notice_after: f64,
    date: String,
    build: String,
    toast: Option<(Toast, f64)>,
    /// The units menu's parsecs picks seen so far.
    parsecs: u64,
    konami: usize,
    last_search: String,
    pending_tip: bool,
    tip_index: usize,
    /// The About window sometimes shows the cloud version string; set once per load.
    pub(crate) odd_version: bool,
    offline: bool,
    offline_dismissed: bool,
}

/// Small dismissible notes in a bottom strip. None of them touches the design.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Toast {
    Salmon,
    Credits,
    Upload,
    Tip,
    Parsecs,
}

impl Toast {
    fn seconds(self) -> f64 {
        match self {
            Toast::Salmon => 5.0,
            Toast::Credits => 6.0,
            Toast::Upload => 2.0,
            Toast::Tip => 10.0,
            Toast::Parsecs => 10.0,
        }
    }
}

const TIPS: [&str; 4] = ["sc3000.web.tip_0", "sc3000.web.tip_1", "sc3000.web.tip_2", "sc3000.web.tip_3"];
const KONAMI: [egui::Key; 10] = [
    egui::Key::ArrowUp,
    egui::Key::ArrowUp,
    egui::Key::ArrowDown,
    egui::Key::ArrowDown,
    egui::Key::ArrowLeft,
    egui::Key::ArrowRight,
    egui::Key::ArrowLeft,
    egui::Key::ArrowRight,
    egui::Key::B,
    egui::Key::A,
];

/// Progress through the Konami code after `key`; a full match returns `KONAMI.len()`.
pub(crate) fn konami_step(progress: usize, key: egui::Key) -> usize {
    if KONAMI.get(progress) == Some(&key) {
        progress + 1
    } else if key == KONAMI[0] {
        1
    } else {
        0
    }
}

/// The fake upload bar's percentage: 100, then 99, then 100 again over two seconds.
pub(crate) fn upload_percent(elapsed: f64) -> u32 {
    if (0.67..1.33).contains(&elapsed) { 99 } else { 100 }
}

/// True for the search words that dispatch the salmon.
pub(crate) fn salmon_query(text: &str) -> bool {
    matches!(text.trim().to_lowercase().as_str(), "salmon" | "fish")
}

#[cfg(target_arch = "wasm32")]
fn chance(probability: f64) -> bool {
    js_sys::Math::random() < probability
}

#[cfg(not(target_arch = "wasm32"))]
fn chance(_probability: f64) -> bool {
    false
}

#[cfg(target_arch = "wasm32")]
fn browser_online() -> bool {
    let Some(window) = web_sys::window() else { return true };
    let Ok(navigator) = js_sys::Reflect::get(&window, &"navigator".into()) else { return true };
    js_sys::Reflect::get(&navigator, &"onLine".into()).ok().and_then(|v| v.as_bool()).unwrap_or(true)
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_online() -> bool {
    true
}

impl Cloud {
    pub(crate) fn new(daily_update: bool, date: String, build: String) -> Self {
        Self {
            scene: daily_update.then_some(Scene::Update(0.0)),
            banner_at: 0.0,
            dismissals: 0,
            active: 0.0,
            previous: None,
            next_session: 600.0,
            next_outage: 600.0,
            identity: String::new(),
            pending_outage: false,
            pending_fillet: false,
            fillets_seen: 0,
            fillet_times: Default::default(),
            fillet_notice_after: 0.0,
            saves: 0,
            pending_save: false,
            date,
            build,
            toast: None,
            parsecs: 0,
            konami: 0,
            last_search: String::new(),
            pending_tip: false,
            tip_index: 0,
            odd_version: false,
            offline: false,
            offline_dismissed: false,
        }
    }

    /// Starts a note, replacing any note already showing.
    pub(crate) fn toast(&mut self, toast: Toast, now: f64) {
        self.toast = Some((toast, now));
    }

    /// Hidden triggers and timed notes. Reads input only; it never changes the design.
    fn extras(&mut self, ctx: &egui::Context, now: f64, search: &str) {
        self.toast = self.toast.filter(|(toast, start)| now - start < toast.seconds());
        let keys: Vec<egui::Key> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed: true, repeat: false, .. } => Some(*key),
                    _ => None,
                })
                .collect()
        });
        for key in keys {
            self.konami = konami_step(self.konami, key);
            if self.konami == KONAMI.len() {
                self.konami = 0;
                self.toast(Toast::Credits, now);
            }
        }
        let wants_salmon = salmon_query(search);
        if wants_salmon && !salmon_query(&self.last_search) {
            self.toast(Toast::Salmon, now);
        }
        if search != self.last_search {
            self.last_search = search.to_owned();
        }
        if self.pending_tip && self.scene.is_none() && self.toast.is_none() {
            self.pending_tip = false;
            self.toast(Toast::Tip, now);
        }
        if browser_online() {
            self.offline = false;
            self.offline_dismissed = false;
        } else {
            self.offline = true;
        }
    }

    /// The bottom strip for notes and the offline banner. Dismissible, never modal.
    pub(crate) fn strip(&mut self, ui: &mut egui::Ui) {
        let now = ui.input(|i| i.time);
        if let Some((toast, start)) = self.toast {
            let elapsed = now - start;
            let mut dismiss = false;
            Container::new(ui.ctx(), Surface::Chrome).edge("sc3000_toast", ui_kit::Edge::Bottom).show(ui, |ui| match toast {
                Toast::Salmon => {
                    salmon_swim(ui, (elapsed / toast.seconds()) as f32, elapsed, false);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(tr("sc3000.web.salmon_dispatched"));
                        dismiss |= ui_kit::button(ui, tr("sc3000.web.dismiss"), ui_kit::Button::Normal).clicked();
                    });
                }
                _ => {
                    ui.horizontal_wrapped(|ui| {
                        match toast {
                            Toast::Credits => ui.label(tr("sc3000.web.credits")),
                            Toast::Tip => ui.label(tr(TIPS[self.tip_index % TIPS.len()])),
                            Toast::Parsecs => ui.label(tr("sc3000.web.parsecs")),
                            _ => {
                                let percent = upload_percent(elapsed);
                                ui.label(format("sc3000.web.upload", &[("percent", &percent.to_string())]));
                                ui.add(egui::ProgressBar::new(percent as f32 / 100.0).desired_width(ui.spacing().slider_width));
                                ui.spinner()
                            }
                        };
                        dismiss |= ui_kit::button(ui, tr("sc3000.web.dismiss"), ui_kit::Button::Normal).clicked();
                    });
                }
            });
            if dismiss {
                self.toast = None;
            }
        }
        if self.offline && !self.offline_dismissed {
            Container::new(ui.ctx(), Surface::Chrome).edge("sc3000_offline", ui_kit::Edge::Bottom).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(tr("sc3000.web.offline"));
                    if ui_kit::button(ui, tr("sc3000.web.dismiss"), ui_kit::Button::Normal).clicked() {
                        self.offline_dismissed = true;
                    }
                });
            });
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn browser() -> Self {
        let now = js_sys::Date::new_0();
        let date = format!("{}-{:02}-{:02}", now.get_full_year(), now.get_month() + 1, now.get_date());
        let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
        let seen = storage.as_ref().and_then(|s| s.get_item("sc3000_update_day").ok().flatten());
        let tip_seen = storage.as_ref().and_then(|s| s.get_item("sc3000_tip_day").ok().flatten());
        if let Some(storage) = storage {
            let _ = storage.set_item("sc3000_update_day", &date);
            let _ = storage.set_item("sc3000_tip_day", &date);
        }
        // UTC dates avoid daylight-saving shifts when counting the calendar day.
        let start = js_sys::Date::utc(f64::from(now.get_full_year()), 0.0);
        let today =
            js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(js_sys::Date::utc(f64::from(now.get_full_year()), f64::from(now.get_month()))));
        today.set_utc_date(now.get_date());
        let day = ((today.get_time() - start) / 86_400_000.0) as u32 + 1;
        let new_tip = tip_seen.as_deref() != Some(&date);
        let mut cloud = Self::new(seen.as_deref() != Some(&date), date, format!("3000.{day}.{}", (js_sys::Math::random() * 10000.0) as u32));
        cloud.pending_outage = js_sys::Math::random() < 0.05;
        cloud.pending_tip = new_tip;
        cloud.tip_index = day as usize;
        cloud.odd_version = js_sys::Math::random() < 0.125;
        cloud
    }

    pub(crate) fn tick(&mut self, now: f64, focused: bool, outage_roll: bool) {
        let delta = self.previous.map_or(0.0, |last| (now - last).clamp(0.0, 1.0));
        self.previous = Some(now);
        if focused {
            self.active += delta;
        }
        match self.scene {
            Some(Scene::Update(elapsed)) => self.scene = Some(Scene::Update(elapsed + delta)),
            Some(Scene::Sync(elapsed)) if elapsed + delta >= 3.0 => self.scene = Some(Scene::Conflict),
            Some(Scene::Sync(elapsed)) => self.scene = Some(Scene::Sync(elapsed + delta)),
            Some(Scene::Outage(elapsed)) if elapsed + delta >= 5.0 => self.scene = None,
            Some(Scene::Outage(elapsed)) => self.scene = Some(Scene::Outage(elapsed + delta)),
            _ => {}
        }
        if self.scene.is_none() {
            if self.pending_fillet {
                self.pending_fillet = false;
                self.scene = Some(Scene::FilletOutage);
            } else if self.pending_save {
                self.pending_save = false;
                self.scene = Some(Scene::Sync(0.0));
            } else if self.pending_outage {
                self.pending_outage = false;
                self.scene = Some(Scene::Outage(0.0));
            } else if self.active >= self.next_session {
                self.next_session = self.active + 600.0;
                self.scene = Some(Scene::Expired);
            } else if self.active >= self.next_outage {
                self.next_outage = self.active + 600.0;
                if outage_roll {
                    self.scene = Some(Scene::Outage(0.0));
                }
            }
        }
    }

    /// A rolling browser-local hour, with at most four timestamps and one notice per hour.
    /// This observes successful operations; it never cancels or changes a CAD command.
    pub(crate) fn record_fillets(&mut self, now: f64, count: u64) -> bool {
        let added = count.saturating_sub(self.fillets_seen).min(4);
        self.fillets_seen = count;
        self.fillet_times.retain(|at| now - at < 3600.0);
        for _ in 0..added {
            if self.fillet_times.len() == 4 {
                self.fillet_times.pop_front();
            }
            self.fillet_times.push_back(now);
        }
        if added > 0 && self.fillet_times.len() == 4 && now >= self.fillet_notice_after {
            self.fillet_notice_after = now + 3600.0;
            true
        } else {
            false
        }
    }

    pub(crate) fn banner(&mut self, ui: &mut egui::Ui) {
        if ui.input(|i| i.time) < self.banner_at {
            return;
        }
        Container::new(ui.ctx(), Surface::Chrome).edge("sc3000_banner", ui_kit::Edge::Top).show(ui, |ui| {
            ui.add_space(f32::from(self.dismissals.min(8)) * ui.spacing().item_spacing.y * 0.25);
            ui.horizontal_wrapped(|ui| {
                ui.label(format("sc3000.web.banner", &[("date", &self.date)]));
                if ui_kit::button(ui, tr("sc3000.web.dismiss"), ui_kit::Button::Normal).clicked() {
                    self.banner_at = ui.input(|i| i.time) + 60.0;
                    self.dismissals = self.dismissals.saturating_add(1);
                }
            });
        });
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context, saves: u64, fillets: u64, parsecs: u64, search: &str) {
        let now = ctx.input(|i| i.time);
        if parsecs != self.parsecs {
            self.parsecs = parsecs;
            self.toast(Toast::Parsecs, now);
        }
        if saves != self.saves {
            self.saves = saves;
            self.pending_save = true;
            if chance(0.25) {
                self.toast(Toast::Upload, now);
            }
        }
        self.extras(ctx, now, search);
        #[cfg(target_arch = "wasm32")]
        let roll = js_sys::Math::random() < 0.05;
        #[cfg(not(target_arch = "wasm32"))]
        let roll = false;
        if self.record_fillets(ctx.input(|i| i.time), fillets) {
            self.pending_fillet = true;
        }
        self.tick(ctx.input(|i| i.time), ctx.input(|i| i.focused), roll);
        let swimming = self.toast.is_some_and(|(t, _)| t == Toast::Salmon) || matches!(self.scene, Some(Scene::Outage(_)));
        ctx.request_repaint_after(std::time::Duration::from_millis(if swimming { 33 } else { 100 }));
        let Some(scene) = self.scene else { return };
        let mut close = false;
        let response = Container::new(ctx, Surface::Window).modal(egui::Id::unique("sc3000_cloud")).show(ctx, |ui| {
            // Wrap at the available width so the joke also fits a small browser window.
            let t = solidcraft_theme::Tokens::get();
            ui.set_max_width((ctx.content_rect().width() - t.space_16 * 3.0).clamp(t.form_card_width / 3.0, t.form_card_width * 1.5));
            match scene {
                Scene::Update(elapsed) => {
                    ui.heading(format("sc3000.web.update", &[("build", &self.build)]));
                    if elapsed < 10.0 {
                        ui.add(egui::ProgressBar::new(update_progress(elapsed)).show_percentage());
                    } else {
                        for id in ["sc3000.web.change_extrude", "sc3000.web.change_cloud", "sc3000.web.change_fillet"] {
                            ui.label(tr(id));
                        }
                    }
                }
                Scene::Expired => {
                    ui.heading(tr("sc3000.web.expired"));
                    ui.label(tr("sc3000.web.signin"));
                    ui.label(tr("sc3000.web.anything"));
                    // A transient joke identity, cleared on every dismissal; never persisted.
                    ui.text_edit_singleline(&mut self.identity);
                    close |= ui_kit::button(ui, tr("sc3000.web.signin"), ui_kit::Button::Normal).clicked();
                }
                Scene::Sync(_) => {
                    ui.label(tr("sc3000.web.sync"));
                    ui.spinner();
                }
                Scene::Conflict => {
                    ui.heading(tr("sc3000.web.conflict"));
                    ui.label(tr("sc3000.web.identical"));
                    for id in ["sc3000.web.yours", "sc3000.web.yours_copy"] {
                        close |= ui_kit::button(ui, tr(id), ui_kit::Button::Normal).clicked();
                    }
                }
                Scene::FilletOutage => {
                    salmon(ui);
                    ui.label(tr("sc3000.web.fillet_outage"));
                }
                Scene::Outage(elapsed) => {
                    ui.heading(tr("sc3000.web.outage"));
                    salmon_swim(ui, (elapsed / 5.0) as f32, elapsed, true);
                    ui.spinner();
                }
            }
            close |= ui_kit::button(ui, tr("sc3000.web.continue"), ui_kit::Button::Normal).clicked();
        });
        if close || response.should_close() {
            self.identity.clear();
            self.scene = None;
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn brand(document: &web_sys::Document) {
    document.set_title(solidcraft_ui_text::tr("sc3000.web.title"));
    if let Some(loading) = document.get_element_by_id("solidcraft_loading")
        && let Ok(text) = document.create_element("p")
    {
        text.set_text_content(Some(solidcraft_ui_text::tr("sc3000.web.splash")));
        let _ = loading.append_child(&text);
        // A rare cameo: about one load in twenty, the same code-drawn salmon swims past.
        if js_sys::Math::random() < 0.05
            && let Ok(fish) = document.create_element("div")
        {
            let _ = fish.set_attribute("style", "overflow:hidden;width:min(320px,80vw);margin:8px auto");
            fish.set_inner_html(&salmon_svg());
            let _ = loading.append_child(&fish);
        }
    }
}

// The salmon, as local proportions (0..1 across, 0..1 down) shared by the painter and the splash SVG.
const FINS: [[(f32, f32); 3]; 5] = [
    [(0.32, 0.5), (0.10, 0.23), (0.16, 0.5)],
    [(0.32, 0.5), (0.16, 0.5), (0.10, 0.77)],
    [(0.47, 0.36), (0.50, 0.15), (0.63, 0.36)],
    [(0.36, 0.38), (0.38, 0.28), (0.43, 0.38)],
    [(0.52, 0.65), (0.60, 0.86), (0.66, 0.65)],
];
const SPOTS: [(f32, f32); 4] = [(0.43, 0.42), (0.51, 0.36), (0.58, 0.43), (0.65, 0.37)];
const EYE: (f32, f32) = (0.77, 0.43);
const SMILE: [(f32, f32); 3] = [(0.78, 0.58), (0.81, 0.61), (0.85, 0.58)];

fn oval() -> Vec<(f32, f32)> {
    (0..48)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / 48.0;
            (0.58 + angle.cos() * 0.31, 0.5 + angle.sin() * 0.23)
        })
        .collect()
}

/// The same salmon as inline SVG for the page's splash screen (fixed colours; no theme there).
fn salmon_svg() -> String {
    let points = |p: &[(f32, f32)]| p.iter().map(|(x, y)| format!("{:.1},{:.1}", x * 200.0, y * 100.0)).collect::<Vec<_>>().join(" ");
    let mut svg = String::from(
        "<style>@keyframes sc3000swim{from{transform:translateX(-110%)}to{transform:translateX(110%)}}</style>\
         <svg viewBox='0 0 200 100' width='120' role='img' aria-label='A friendly service salmon' \
         style='animation:sc3000swim 4s linear infinite' fill='#e8684a' stroke='#1b1b1b' stroke-width='2'>",
    );
    for fin in FINS {
        svg.push_str(&format!("<polygon points='{}'/>", points(&fin)));
    }
    svg.push_str(&format!("<polygon points='{}'/>", points(&oval())));
    svg.push_str(&format!("<polyline fill='none' points='{}'/>", points(&[(0.37, 0.53), (0.68, 0.53)])));
    for (x, y) in SPOTS {
        svg.push_str(&format!("<circle cx='{:.1}' cy='{:.1}' r='1.6' fill='#1b1b1b' stroke='none'/>", x * 200.0, y * 100.0));
    }
    svg.push_str(&format!("<circle cx='{:.1}' cy='{:.1}' r='3.2' fill='#1b1b1b' stroke='none'/>", EYE.0 * 200.0, EYE.1 * 100.0));
    svg.push_str(&format!("<polyline fill='none' points='{}'/></svg>", points(&SMILE)));
    svg
}

/// Original illustration: an analytic oval, forked tail, salmon fins, spots and a smile.
/// Coordinates are local drawing proportions; colours follow the active theme.
fn draw_salmon(painter: &egui::Painter, rect: egui::Rect) {
    let t = solidcraft_theme::Tokens::get();
    let width = rect.width();
    let point = |(x, y): (f32, f32)| rect.min + egui::vec2(x * rect.width(), y * rect.height());
    let body = t.error;
    let ink = t.text;
    let stroke = egui::Stroke::new(t.space_2, ink);
    for fin in FINS {
        painter.add(egui::Shape::convex_polygon(fin.into_iter().map(point).collect(), body, stroke));
    }
    painter.add(egui::Shape::convex_polygon(oval().into_iter().map(point).collect(), body, stroke));
    painter.line_segment([point((0.37, 0.53)), point((0.68, 0.53))], stroke);
    for spot in SPOTS {
        painter.circle_filled(point(spot), width * 0.008, ink);
    }
    painter.circle_filled(point(EYE), width * 0.016, ink);
    painter.circle_filled(point((0.774, 0.42)), width * 0.005, t.panel);
    painter.add(egui::Shape::line(SMILE.into_iter().map(point).collect(), stroke));
}

fn salmon(ui: &mut egui::Ui) {
    let t = solidcraft_theme::Tokens::get();
    let width = ui.available_width().min(t.form_card_width);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, width * 0.5), egui::Sense::hover());
    draw_salmon(ui.painter(), rect);
    ui.label(tr("sc3000.web.salmon"));
}

/// The salmon swimming left to right across the available width, bobbing as it goes.
fn salmon_swim(ui: &mut egui::Ui, progress: f32, elapsed: f64, caption: bool) {
    let t = solidcraft_theme::Tokens::get();
    let width = ui.available_width().min(t.form_card_width * 1.5);
    let fish = (width * 0.35).min(t.form_card_width * 0.5);
    let (row, _) = ui.allocate_exact_size(egui::vec2(width, fish * 0.5 + t.space_8), egui::Sense::hover());
    let bob = (elapsed as f32 * 6.0).sin() * t.space_4;
    let x = row.min.x + (width - fish) * progress.clamp(0.0, 1.0);
    let rect = egui::Rect::from_min_size(egui::pos2(x, row.min.y + t.space_4 + bob), egui::vec2(fish, fish * 0.5));
    draw_salmon(&ui.painter().with_clip_rect(row.expand(t.space_8)), rect);
    if caption {
        ui.label(tr("sc3000.web.salmon"));
    }
}
