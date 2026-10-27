//! Bounded viewport decoration, painted before the model and excluded from exports.
use crate::{SolveApp, theme::Tokens};
use egui::{Align2, FontId, Painter, Rect};

const FRAME: f64 = 1.0 / 30.0;
const MAX_COLUMNS: usize = 96;
const TRAIL: usize = 14;

#[derive(Default)]
pub(crate) struct State {
    last_input: Option<f64>,
    last_frame: Option<f64>,
    phase: f64,
}

impl State {
    /// A paused frame requests no successor and forgets its clock, so resuming never jumps.
    pub(crate) fn tick(&mut self, now: f64, running: bool) -> Option<std::time::Duration> {
        if !running || !now.is_finite() {
            self.last_frame = None;
            return None;
        }
        // Unrelated UI repaints cannot accelerate the animation clock.
        let elapsed = now - self.last_frame.unwrap_or(now);
        if elapsed < 0.0 {
            self.last_frame = Some(now);
        } else if elapsed >= FRAME {
            self.phase = (self.phase + elapsed.min(0.1)).rem_euclid(3600.0);
            self.last_frame = Some(now);
        } else if self.last_frame.is_none() {
            self.last_frame = Some(now);
        }
        // Round up to whole milliseconds so a timer can never round 1/30 s down.
        Some(std::time::Duration::from_millis(34))
    }
}

pub(crate) fn active(theme: bool, enabled: bool, focused: bool, reduced: bool, idle: f64, capture: bool) -> bool {
    theme && enabled && focused && !reduced && idle.is_finite() && (0.0..30.0).contains(&idle) && !capture
}

pub(crate) fn columns(width: f32, spacing: f32) -> usize {
    if width.is_finite() && width > 0.0 && spacing.is_finite() && spacing > 0.0 { ((width / spacing).ceil() as usize).min(MAX_COLUMNS) } else { 0 }
}

pub(crate) fn paint(app: &mut SolveApp, ctx: &egui::Context, painter: &Painter, rect: Rect, t: &Tokens) {
    let (now, focused, input) = ctx.input(|i| (i.time, i.focused, !i.raw.events.is_empty()));
    let state = &mut app.viewport.rain;
    if input || state.last_input.is_none() {
        state.last_input = Some(now);
    }
    let theme = crate::theme_files::find(&app.preferences.theme, &app.preferences.custom_themes).is_some_and(|t| t.background.is_some());
    let capture = !app.pending_shots.is_empty();
    let requested = theme && app.preferences.animated_background && !capture;
    let running = active(theme, app.preferences.animated_background, focused, false, now - state.last_input.unwrap_or(now), capture)
        && !reduced_motion(ctx, state.last_input.unwrap_or(now));
    if let Some(delay) = state.tick(now, running) {
        ctx.request_repaint_after(delay);
    }
    if !requested {
        return;
    }
    // Paused frames retain their glyph positions and request no successor.
    if !rect.is_finite() || rect.height() <= 0.0 {
        return;
    }
    let painter = painter.with_clip_rect(rect);
    let glyphs = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let row_height = t.font_small + t.space_4;
    for col in 0..columns(rect.width(), t.space_24) {
        let seed = (col as u32).wrapping_mul(747796405).wrapping_add(2891336453);
        let speed = 28.0 + f64::from(seed % 37);
        let cycle = f64::from(rect.height()) + (TRAIL as f64 * f64::from(row_height));
        let head = (state.phase * speed + f64::from(seed % 10000)).rem_euclid(cycle);
        for row in 0..TRAIL {
            let y = rect.top() + head as f32 - row as f32 * row_height;
            if y < rect.top() || y > rect.bottom() {
                continue;
            }
            let n = seed.wrapping_add((row as u32).wrapping_mul(1664525)).wrapping_add((state.phase * 3.0) as u32);
            if let Some(glyph) = glyphs.get(n as usize % glyphs.len()) {
                // The leading glyph is bright text colour; the trail starts in full accent and fades out.
                let ink = if row == 0 { t.text } else { t.accent.gamma_multiply(0.85 * (1.0 - row as f32 / TRAIL as f32)) };
                painter.text(
                    egui::pos2(rect.left() + col as f32 * t.space_24 + t.space_8, y),
                    Align2::LEFT_TOP,
                    char::from(*glyph),
                    FontId::monospace(t.font_small),
                    ink,
                );
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn reduced_motion(_ctx: &egui::Context, _last_input: f64) -> bool {
    web_sys::window().and_then(|w| w.match_media("(prefers-reduced-motion: reduce)").ok().flatten()).is_some_and(|m| m.matches())
}

#[cfg(not(target_arch = "wasm32"))]
fn reduced_motion(ctx: &egui::Context, last_input: f64) -> bool {
    use std::sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::{Duration, Instant};
    struct Query {
        next: Instant,
        reduced: Arc<AtomicBool>,
        busy: Arc<AtomicBool>,
    }
    static QUERY: OnceLock<Mutex<Query>> = OnceLock::new();
    let query = QUERY
        .get_or_init(|| Mutex::new(Query { next: Instant::now(), reduced: Arc::new(AtomicBool::new(true)), busy: Arc::new(AtomicBool::new(false)) }));
    let Ok(mut query) = query.lock() else { return true };
    if Instant::now() >= query.next && !query.busy.swap(true, Ordering::Relaxed) {
        query.next = Instant::now() + Duration::from_secs(5);
        let reduced = query.reduced.clone();
        let busy = query.busy.clone();
        let idle = ctx.input(|i| i.time - last_input);
        let started = Instant::now();
        let ctx = ctx.clone();
        // Platform queries can wait on a settings service; never run them on the window thread.
        let result = std::thread::Builder::new().name("motion-preference".into()).spawn(move || {
            let value = platform_reduced_motion();
            let previous = reduced.swap(value, Ordering::Relaxed);
            if previous != value && ctx.input(|i| active(true, true, i.focused, false, idle + started.elapsed().as_secs_f64(), false)) {
                ctx.request_repaint();
            }
            busy.store(false, Ordering::Relaxed);
        });
        if result.is_err() {
            query.busy.store(false, Ordering::Relaxed);
        }
    }
    query.reduced.load(Ordering::Relaxed)
}

#[cfg(target_os = "linux")]
pub(crate) fn kde_motion(text: &str) -> Option<bool> {
    if text.len() > 64 * 1024 {
        return None;
    }
    let mut in_kde = false;
    let mut reduced = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_kde = line == "[KDE]";
        } else if in_kde
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "AnimationDurationFactor"
        {
            reduced = value.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).map(|v| v == 0.0);
        }
    }
    reduced
}

#[cfg(target_os = "linux")]
fn platform_reduced_motion() -> bool {
    use std::io::Read;
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.split(':').any(|name| name.eq_ignore_ascii_case("KDE")));
    if kde {
        // Read the small preference directly; launching a Qt settings utility can initialise graphics.
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config")));
        if let Some(config) = config
            && let Ok(file) = std::fs::File::open(config.join("kdeglobals"))
        {
            let mut text = String::new();
            if file.take(64 * 1024 + 1).read_to_string(&mut text).is_ok()
                && let Some(reduced) = kde_motion(&text)
            {
                return reduced;
            }
        }
    }
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
}

#[cfg(target_os = "macos")]
fn platform_reduced_motion() -> bool {
    std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
}

#[cfg(target_os = "windows")]
fn platform_reduced_motion() -> bool {
    windows::UI::ViewManagement::UISettings::new().and_then(|settings| settings.AnimationsEnabled()).is_ok_and(|enabled| !enabled)
}

#[cfg(not(any(target_arch = "wasm32", target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_reduced_motion() -> bool {
    false
}
