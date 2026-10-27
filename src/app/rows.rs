//! Table rows: a name, the value and a thin meter, as used by every parameter panel.

use eframe::egui::{self, Align2, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use super::controls::truncated;
use super::{ROW_HEIGHT, percent};
use crate::ui::icons::{self, Glyph};
use crate::ui::theme::Palette;
use crate::ui::widgets;

/// One row of a value table: name and value. `bits` draws a binary encoding as boxes.
pub(super) struct Row<'a> {
    pub(super) name: &'a str,
    pub(super) value: f32,
    /// Used by the avatar (or nothing is known yet); unused rows are dimmed.
    pub(super) active: bool,
    /// Runs -1..1 instead of 0..1.
    pub(super) signed: bool,
    pub(super) bits: Option<&'a [bool]>,
    /// Expandable row: `Some(open)` draws the chevron.
    pub(super) chevron: Option<bool>,
    pub(super) highlight: bool,
}

impl<'a> Row<'a> {
    pub(super) fn new(name: &'a str, value: f32) -> Self {
        Self { name, value, active: true, signed: false, bits: None, chevron: None, highlight: false }
    }
}

pub(super) fn value_row(ui: &mut egui::Ui, pal: &Palette, row: Row<'_>) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
    let painter = ui.painter_at(rect);
    paint_value_row(&painter, rect, pal, &row, row.highlight || response.hovered());
    response
}

/// Rows are inset a little so the meters and highlights read as rounded bars.
pub(super) fn row_inset(rect: Rect) -> Rect {
    rect.shrink2(Vec2::new(2.0, 1.0))
}

/// The value as a thin bar under the row text: from the left for 0..1, from the middle for -1..1.
pub(super) fn paint_meter(painter: &egui::Painter, rect: Rect, pal: &Palette, value: f32, signed: bool, active: bool) {
    let track = Rect::from_min_max(
        Pos2::new(rect.left() + 12.0, rect.bottom() - 6.0),
        Pos2::new(rect.right() - 12.0, rect.bottom() - 3.0),
    );
    let radius = CornerRadius::same(2);
    painter.rect_filled(track, radius, pal.line.gamma_multiply(0.45));
    let v = value.clamp(-1.0, 1.0);
    let (from, to) = if signed {
        let middle = track.center().x;
        let end = middle + v * track.width() / 2.0;
        (middle.min(end), middle.max(end))
    } else {
        (track.left(), track.left() + v.max(0.0) * track.width())
    };
    if to - from >= 1.0 {
        let bar = Rect::from_min_max(Pos2::new(from, track.top()), Pos2::new(to, track.bottom()));
        painter.rect_filled(bar, radius, if active { pal.accent } else { pal.dim });
    }
    if signed {
        // Marks the zero point.
        let middle = track.center().x;
        painter.vline(middle, track.y_range().expand(1.5), Stroke::new(1.0, pal.dim));
    }
}

fn paint_value_row(painter: &egui::Painter, rect: Rect, pal: &Palette, row: &Row<'_>, highlight: bool) {
    if highlight {
        painter.rect_filled(row_inset(rect), CornerRadius::same(5), pal.line.gamma_multiply(0.5));
    }
    paint_meter(painter, rect, pal, row.value, row.signed, row.active);
    let color = if row.active { pal.text } else { pal.dim };
    let reserved = 70.0 + row.bits.map_or(0.0, |b| b.len() as f32 * 11.0 + 4.0) + row.chevron.map_or(0.0, |_| 16.0);
    let mut left = rect.left() + 12.0;
    if let Some(open) = row.chevron {
        let glyph = if open { Glyph::ChevronDown } else { Glyph::ChevronRight };
        let area = Rect::from_center_size(Pos2::new(left + 5.0, rect.center().y), Vec2::splat(12.0));
        icons::paint(painter, glyph, area, pal.dim);
        left += 16.0;
    }
    let galley = truncated(painter, row.name.to_string(), FontId::proportional(13.0), color, rect.width() - reserved);
    painter.galley(Pos2::new(left, rect.center().y - galley.size().y / 2.0), galley, color);
    let mut right = rect.right() - 12.0;
    if let Some(bits) = row.bits {
        for &on in bits.iter().rev() {
            let b = Rect::from_center_size(Pos2::new(right - 4.0, rect.center().y), Vec2::splat(8.0));
            if on {
                painter.rect_filled(b, CornerRadius::same(2), pal.text);
            } else {
                painter.rect_stroke(b, CornerRadius::same(2), Stroke::new(1.0, pal.dim), StrokeKind::Inside);
            }
            right -= 11.0;
        }
        right -= 4.0;
    }
    painter.text(
        Pos2::new(right, rect.center().y),
        Align2::RIGHT_CENTER,
        percent(row.value),
        FontId::proportional(12.5),
        color,
    );
}

/// One synced bool of a binary parameter, shown under its parameter: the parameter
/// name without its path, and ON / OFF.
pub(super) fn bit_row(ui: &mut egui::Ui, pal: &Palette, address: &str, on: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    let name = address.rsplit('/').next().unwrap_or(address).to_string();
    let galley = truncated(&painter, name, FontId::proportional(12.5), pal.dim, rect.width() - 90.0);
    painter.galley(rect.left_center() + Vec2::new(30.0, -galley.size().y / 2.0), galley, pal.dim);
    let (text, color) = if on { ("ON", pal.ok) } else { ("OFF", pal.dim) };
    painter.text(
        rect.right_center() - Vec2::new(12.0, 0.0),
        Align2::RIGHT_CENTER,
        text,
        FontId::new(11.5, widgets::semibold()),
        color,
    );
}
