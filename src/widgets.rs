//! Controls shared by the interface: buttons, chips, icon buttons, the status pill and the
//! card frame. They are painted from the palette, with the same radius, states and focus ring.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    Vec2,
};

use crate::icons::{self, Glyph};
use crate::theme::{Palette, mix};

/// Corner radius of controls.
pub const RADIUS: u8 = 6;

/// Family for titles and emphasis.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Card: a panel-colored surface with a hairline border.
pub fn card(pal: &Palette) -> Frame {
    Frame::new()
        .fill(pal.panel)
        .stroke(Stroke::new(1.0, pal.line.gamma_multiply(0.8)))
        .corner_radius(CornerRadius::same(RADIUS + 4))
        .inner_margin(Margin::same(14))
}

pub fn title(pal: &Palette, text: &str) -> RichText {
    RichText::new(text).font(FontId::new(13.5, semibold())).color(pal.text)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// The one action that matters on the screen: accent fill.
    Primary,
    Secondary,
    /// No fill until hovered.
    Quiet,
}

pub struct Button<'a> {
    pub text: &'a str,
    pub glyph: Option<Glyph>,
    pub look: Look,
    /// Fixed width; the button fits its content when `None`.
    pub width: Option<f32>,
    /// Shows a check mark at the right edge.
    pub done: bool,
    /// Another text the button will show; it keeps room for the wider of the two so the
    /// controls after it do not move when the text changes.
    pub reserve: Option<&'a str>,
}

impl<'a> Button<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text, glyph: None, look: Look::Secondary, width: None, done: false, reserve: None }
    }

    pub fn glyph(mut self, glyph: Glyph) -> Self {
        self.glyph = Some(glyph);
        self
    }

    pub fn look(mut self, look: Look) -> Self {
        self.look = look;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn done(mut self, done: bool) -> Self {
        self.done = done;
        self
    }

    pub fn reserve(mut self, text: &'a str) -> Self {
        self.reserve = Some(text);
        self
    }
}

fn focus_ring(painter: &egui::Painter, rect: Rect, pal: &Palette, radius: u8) {
    painter.rect_stroke(
        rect.expand(1.0),
        CornerRadius::same(radius + 1),
        Stroke::new(2.0, pal.accent),
        StrokeKind::Outside,
    );
}

pub fn button(ui: &mut egui::Ui, pal: &Palette, spec: Button<'_>) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        spec.text.to_string(),
        FontId::new(13.0, FontFamily::Proportional),
        Color32::PLACEHOLDER,
    );
    let icon = if spec.glyph.is_some() { 24.0 } else { 0.0 };
    let reserved = spec.reserve.map_or(0.0, |text| {
        let font = FontId::new(13.0, FontFamily::Proportional);
        ui.painter().layout_no_wrap(text.to_string(), font, Color32::PLACEHOLDER).size().x
    });
    let width = spec.width.unwrap_or(galley.size().x.max(reserved) + icon + 28.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hot = response.hovered() || response.has_focus();
    let (fill, stroke, fg) = match spec.look {
        Look::Primary => (if hot { mix(pal.accent, pal.text, 0.18) } else { pal.accent }, Stroke::NONE, pal.on_accent),
        Look::Secondary => (
            if hot { pal.hover() } else { pal.raised },
            Stroke::new(1.0, if hot { pal.dim } else { pal.line }),
            pal.text,
        ),
        Look::Quiet => {
            (if hot { pal.raised } else { Color32::TRANSPARENT }, Stroke::NONE, if hot { pal.text } else { pal.dim })
        }
    };
    let painter = ui.painter_at(rect.expand(3.0));
    painter.rect(rect, CornerRadius::same(RADIUS), fill, stroke, StrokeKind::Inside);
    if response.has_focus() {
        focus_ring(&painter, rect, pal, RADIUS);
    }
    let mut x =
        if spec.width.is_some() { rect.left() + 12.0 } else { rect.center().x - (icon + galley.size().x) / 2.0 };
    if let Some(glyph) = spec.glyph {
        icons::paint(
            &painter,
            glyph,
            Rect::from_center_size(Pos2::new(x + 8.0, rect.center().y), Vec2::splat(16.0)),
            fg,
        );
        x += icon;
    }
    painter.galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    if spec.done {
        let check = Rect::from_center_size(Pos2::new(rect.right() - 18.0, rect.center().y), Vec2::splat(16.0));
        icons::paint(&painter, Glyph::Check, check, pal.ok);
    }
    response
}

pub enum Icon {
    Draw(Glyph),
    Image(egui::TextureId),
}

/// Square icon button. `on` marks a toggled-on or open state with the accent.
pub fn icon_button(ui: &mut egui::Ui, pal: &Palette, icon: Icon, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hot = response.hovered() || response.has_focus();
    let painter = ui.painter_at(rect.expand(3.0));
    let (fill, stroke, fg) = if on {
        (mix(pal.panel, pal.accent, 0.2), Stroke::new(1.0, pal.accent), pal.accent)
    } else if hot {
        (pal.raised, Stroke::new(1.0, pal.line), pal.text)
    } else {
        (Color32::TRANSPARENT, Stroke::NONE, pal.text.gamma_multiply(0.8))
    };
    painter.rect(rect, CornerRadius::same(RADIUS), fill, stroke, StrokeKind::Inside);
    if response.has_focus() {
        focus_ring(&painter, rect, pal, RADIUS);
    }
    let target = Rect::from_center_size(rect.center(), Vec2::splat(18.0));
    match icon {
        Icon::Draw(glyph) => icons::paint(&painter, glyph, target, fg),
        Icon::Image(texture) => {
            painter.image(texture, target, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), fg);
        }
    }
    response
}

/// Choice between a few views: a track with the selected one marked. Returns true when
/// `selected` changed.
pub fn segmented(ui: &mut egui::Ui, pal: &Palette, labels: &[&str], selected: &mut usize) -> bool {
    let font = |on: bool| FontId::new(12.5, if on { semibold() } else { FontFamily::Proportional });
    // Measured in the heavier face so the track does not resize when the selection moves.
    let widths: Vec<f32> = labels
        .iter()
        .map(|label| ui.painter().layout_no_wrap(label.to_string(), font(true), Color32::PLACEHOLDER).size().x + 28.0)
        .collect();
    let (track, _) = ui.allocate_exact_size(Vec2::new(widths.iter().sum::<f32>() + 6.0, 32.0), Sense::hover());
    let painter = ui.painter_at(track.expand(3.0));
    painter.rect(track, CornerRadius::same(RADIUS + 2), pal.raised, Stroke::new(1.0, pal.line), StrokeKind::Inside);
    let mut x = track.left() + 3.0;
    let mut changed = false;
    for (i, label) in labels.iter().enumerate() {
        let segment = Rect::from_min_size(Pos2::new(x, track.top() + 3.0), Vec2::new(widths[i], 26.0));
        x += widths[i];
        let response = ui
            .interact(segment, ui.make_persistent_id(("segment", i)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let on = *selected == i;
        if on {
            let fill = mix(pal.raised, pal.accent, 0.28);
            painter.rect(segment, CornerRadius::same(RADIUS), fill, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
        } else if response.hovered() {
            painter.rect_filled(segment, CornerRadius::same(RADIUS), pal.hover());
        }
        if response.has_focus() {
            focus_ring(&painter, segment, pal, RADIUS);
        }
        let color = if on { pal.text } else { pal.dim };
        painter.text(segment.center(), egui::Align2::CENTER_CENTER, *label, font(on), color);
        if response.clicked() && !on {
            *selected = i;
            changed = true;
        }
    }
    changed
}

/// Minimize, maximize and close for the drawn title bar. Close turns red under the pointer.
pub fn window_button(ui: &mut egui::Ui, pal: &Palette, glyph: Glyph, close: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(38.0, 32.0), Sense::click());
    let hot = response.hovered() || response.has_focus();
    let painter = ui.painter_at(rect.expand(3.0));
    let fg = if hot && close {
        Color32::WHITE
    } else if hot {
        pal.text
    } else {
        pal.text.gamma_multiply(0.8)
    };
    if hot {
        painter.rect_filled(rect, CornerRadius::same(RADIUS), if close { pal.err } else { pal.raised });
    }
    if response.has_focus() {
        focus_ring(&painter, rect, pal, RADIUS);
    }
    icons::paint(&painter, glyph, Rect::from_center_size(rect.center(), Vec2::splat(16.0)), fg);
    response
}

/// Pill-shaped toggle with an icon: outlined when off, accent-tinted when on.
pub fn chip(ui: &mut egui::Ui, pal: &Palette, glyph: Glyph, text: &str, on: &mut bool) -> egui::Response {
    chip_sized(ui, pal, glyph, text, on, 28.0)
}

/// A switch with its label as one control: the whole thing is the click target, and the
/// knob slides.
pub fn switch_toggle(ui: &mut egui::Ui, pal: &Palette, on: &mut bool, text: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        FontId::new(13.0, FontFamily::Proportional),
        Color32::PLACEHOLDER,
    );
    let size = Vec2::new(10.0 + 30.0 + 8.0 + galley.size().x + 12.0, 32.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let hot = response.hovered() || response.has_focus();
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let painter = ui.painter_at(rect.expand(3.0));
    let (fill, stroke) =
        if hot { (pal.raised, Stroke::new(1.0, pal.line)) } else { (Color32::TRANSPARENT, Stroke::NONE) };
    painter.rect(rect, CornerRadius::same(RADIUS), fill, stroke, StrokeKind::Inside);
    if response.has_focus() {
        focus_ring(&painter, rect, pal, RADIUS);
    }
    let track = Rect::from_center_size(Pos2::new(rect.left() + 10.0 + 15.0, rect.center().y), Vec2::new(30.0, 16.0));
    painter.rect_filled(track, CornerRadius::same(8), mix(pal.line, pal.accent, t));
    painter.circle_filled(
        Pos2::new(track.left() + 8.0 + t * 14.0, track.center().y),
        5.5,
        mix(pal.dim, pal.on_accent, t),
    );
    let fg = if *on || hot { pal.text } else { pal.dim };
    painter.galley(Pos2::new(track.right() + 8.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    response
}

pub fn chip_sized(
    ui: &mut egui::Ui,
    pal: &Palette,
    glyph: Glyph,
    text: &str,
    on: &mut bool,
    height: f32,
) -> egui::Response {
    let radius = (height / 2.0) as u8;
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        FontId::new(12.5, FontFamily::Proportional),
        Color32::PLACEHOLDER,
    );
    let size = Vec2::new(12.0 + 15.0 + 6.0 + galley.size().x + 12.0, height);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let hot = response.hovered() || response.has_focus();
    let (fill, stroke, glyph_color, fg) = if *on {
        (mix(pal.panel, pal.accent, 0.2), Stroke::new(1.0, pal.accent), pal.accent, pal.text)
    } else {
        (if hot { pal.raised } else { Color32::TRANSPARENT }, Stroke::new(1.0, pal.line), pal.dim, pal.dim)
    };
    let painter = ui.painter_at(rect.expand(3.0));
    painter.rect(rect, CornerRadius::same(radius), fill, stroke, StrokeKind::Inside);
    if response.has_focus() {
        focus_ring(&painter, rect, pal, radius);
    }
    let center_y = rect.center().y;
    icons::paint(
        &painter,
        glyph,
        Rect::from_center_size(Pos2::new(rect.left() + 12.0 + 7.5, center_y), Vec2::splat(15.0)),
        glyph_color,
    );
    painter.galley(Pos2::new(rect.left() + 12.0 + 15.0 + 6.0, center_y - galley.size().y / 2.0), galley, fg);
    response
}

/// Status readout: a colored dot and a line of text in a pill.
pub fn pill(ui: &mut egui::Ui, pal: &Palette, color: Color32, text: &str, reserve: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        FontId::new(12.5, FontFamily::Proportional),
        Color32::PLACEHOLDER,
    );
    let reserved = ui
        .painter()
        .layout_no_wrap(reserve.to_string(), FontId::new(12.5, FontFamily::Proportional), Color32::PLACEHOLDER)
        .size()
        .x;
    let (rect, response) = ui
        .allocate_exact_size(Vec2::new(14.0 + 8.0 + 8.0 + galley.size().x.max(reserved) + 14.0, 28.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect(rect, CornerRadius::same(14), pal.raised, Stroke::new(1.0, pal.line), StrokeKind::Inside);
    painter.circle_filled(Pos2::new(rect.left() + 14.0 + 4.0, rect.center().y), 4.0, color);
    painter.galley(
        Pos2::new(rect.left() + 14.0 + 8.0 + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        pal.text,
    );
    response
}
