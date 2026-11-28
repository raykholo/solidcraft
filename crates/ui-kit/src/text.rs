//! Text elements for dense cards; see `docs/ui-kit.md` for tokens and usage.
use egui::Ui;
use solidcraft_theme::Tokens;

/// Shorten `text` with an ellipsis in the middle so both ends stay visible (#9081).
pub fn ellipsize_middle(ui: &Ui, text: &str, font: egui::FontId, max_width: f32) -> String {
    if !(max_width.is_finite() && max_width > 0.0) {
        return String::new();
    }
    let width = |s: &str| ui.fonts_mut(|fonts| fonts.layout_no_wrap(s.to_owned(), font.clone(), ui.visuals().text_color()).size().x);
    if width(text) <= max_width + 0.5 {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 4 {
        return "…".into();
    }
    let mut best = "…".to_string();
    let mut lo = 1usize;
    let mut hi = chars.len() / 2;
    while lo <= hi {
        let keep = (lo + hi) / 2;
        let left: String = chars.iter().take(keep).collect();
        let right: String = chars.iter().skip(chars.len() - keep).collect();
        let candidate = format!("{left}…{right}");
        if width(&candidate) <= max_width + 0.5 {
            best = candidate;
            lo = keep + 1;
        } else if keep == 0 {
            break;
        } else {
            hi = keep - 1;
        }
    }
    best
}

/// Paint `text` in up to two lines across `rect`, from its top: wrapped at word breaks, and
/// when two lines are not enough, the second is shortened in the middle so the end of the
/// text (a size, a variant) stays visible (#23596).
pub(crate) fn paint_two_lines(ui: &Ui, rect: egui::Rect, text: &str, font: egui::FontId, ink: egui::Color32) {
    let width = rect.width().max(0.0);
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font.clone(), ink));
    job.wrap.max_width = width;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    if galley.rows.len() <= 2 {
        let at = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.top());
        painter.galley(at, galley, ink);
        return;
    }
    let first_chars = galley.rows.first().map_or(0, |row| row.glyphs.len());
    let first: String = text.chars().take(first_chars).collect();
    let rest: String = text.chars().skip(first_chars).collect();
    let second = ellipsize_middle(ui, rest.trim_start(), font.clone(), width);
    let mut y = rect.top();
    for line in [first.trim_end(), second.as_str()] {
        let line = painter.layout_no_wrap(line.to_owned(), font.clone(), ink);
        let height = line.size().y;
        painter.galley(egui::pos2(rect.center().x - line.size().x / 2.0, y), line, ink);
        y += height;
    }
}

/// Paint one pixel-aligned text run with the requested interface weight.
/// Clipped, device-pixel-snapped weighted text. Tokens: `font_section_tracking; caller supplies font/ink tokens`.
pub fn paint_text(ui: &Ui, rect: egui::Rect, text: &str, font: egui::FontId, ink: egui::Color32, spaced: bool) {
    paint_text_aligned(ui, rect, text, font, ink, spaced, false);
}

/// Like [`paint_text`], but with `middle` an ellipsis sits in the middle so a path's file name
/// stays visible (#9081).
pub(super) fn paint_text_aligned(ui: &Ui, rect: egui::Rect, text: &str, font: egui::FontId, ink: egui::Color32, spaced: bool, middle: bool) {
    let size = font.size;
    let mut format = egui::TextFormat::simple(font.clone(), ink);
    if spaced {
        format.extra_letter_spacing = size * Tokens::get().font_section_tracking;
    }
    // Optical centring: egui centres the row box, but this font's capitals sit low in it (the
    // ascent exceeds the cap height, and a descent hangs under the baseline), so the row box
    // goes up by the measured offset and the ink reads centred (#23381).
    let rise = egui::cap_ink_rise(ui.painter(), &font);
    let shown = if middle { ellipsize_middle(ui, text, font, rect.width().max(0.0)) } else { text.to_owned() };
    let mut job = egui::text::LayoutJob::single_section(shown, format);
    job.wrap.max_width = rect.width().max(0.0);
    job.wrap.max_rows = 1;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    // Snap to whole device pixels: a fractional origin blurs every glyph edge.
    let ppp = ui.ctx().pixels_per_point().max(0.5);
    let snap = |v: f32| (v * ppp).round() / ppp;
    let at = egui::pos2(snap(rect.left()), snap(rect.center().y - galley.size().y / 2.0 - rise));
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    painter.galley(at, galley, ink);
}

/// [`egui::Painter::text`] whose vertically centred anchors (`LEFT_CENTER`, `CENTER_CENTER`,
/// `RIGHT_CENTER`) are lifted by [`egui::galley_cap_ink_rise`], so the capitals read centred on
/// `pos` the way [`centred_in`] centres them (#23399). Returns the rectangle painted in.
pub fn paint_anchored(
    painter: &egui::Painter,
    pos: egui::Pos2,
    anchor: egui::Align2,
    text: impl ToString,
    font: egui::FontId,
    ink: egui::Color32,
) -> egui::Rect {
    let galley = painter.layout_no_wrap(text.to_string(), font, ink);
    let rise = if anchor.y() == egui::Align::Center { egui::galley_cap_ink_rise(painter, &galley) } else { 0.0 };
    let rect = anchor.anchor_size(pos, galley.size()).translate(egui::vec2(0.0, -rise));
    painter.galley(rect.min, galley, ink);
    rect
}

/// Where to paint `galley` so its capitals read centred on `centre` (#23381): egui's geometric
/// centre of the row box, lifted by [`egui::galley_cap_ink_rise`].
pub(crate) fn centred_in(painter: &egui::Painter, galley: &egui::Galley, centre: egui::Pos2) -> egui::Pos2 {
    centre - galley.size() / 2.0 - egui::vec2(0.0, egui::galley_cap_ink_rise(painter, galley))
}
