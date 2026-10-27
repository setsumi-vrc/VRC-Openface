//! Small controls and frames drawn in the palette, used across the views.

use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, Vec2,
};

use super::{App, percent};
use crate::config::Settings;
use crate::ui::icons::{self, Glyph};
use crate::ui::theme::mix;
use crate::ui::widgets::{self, Icon};

/// Panel header: the title on the left, truncated to the room the controls on the right
/// leave. When that room gets too small the controls move to a second line.
pub(super) fn header(
    ui: &mut egui::Ui,
    id: &str,
    title: impl FnOnce(&mut egui::Ui),
    controls: impl FnOnce(&mut egui::Ui),
) {
    let key = egui::Id::unique(("header-controls", id));
    let needed: f32 = ui.data(|d| d.get_temp(key)).unwrap_or(0.0);
    let right_to_left = Layout::right_to_left(Align::Center);
    let measured = if needed + 150.0 > ui.available_width() {
        ui.horizontal(|ui| ui.with_layout(Layout::left_to_right(Align::Center), title));
        // Wrapping keeps the controls inside the panel; overflowing a right-to-left layout
        // would push the whole panel off to the left.
        let wrapping = Layout::right_to_left(Align::Center).with_main_wrap(true);
        // Start one row high so the rows are not centred in the whole panel.
        let size = Vec2::new(ui.available_width(), ui.spacing().interact_size.y);
        let rect = ui.allocate_ui_with_layout(size, wrapping, |ui| ui.scope(controls).response.rect).inner;
        // A wrapped row is narrower than the controls really are; keep the old width then.
        (rect.height() < ui.spacing().interact_size.y * 1.6).then_some(rect.width())
    } else {
        ui.horizontal(|ui| {
            ui.with_layout(right_to_left, |ui| {
                let width = ui.scope(controls).response.rect.width();
                ui.with_layout(Layout::left_to_right(Align::Center), title);
                Some(width)
            })
            .inner
        })
        .inner
    };
    if let Some(width) = measured {
        ui.data_mut(|d| d.insert_temp(key, width));
    }
}

/// Single-line text cut with "…" to fit `width`.
pub(super) fn truncated(
    painter: &egui::Painter,
    text: String,
    font: FontId,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    painter.layout_job(job)
}

impl App {
    /// Filled with the accent colour when on, like a pressed key.
    pub(super) fn toggle(&self, ui: &mut egui::Ui, on: &mut bool, text: &str) -> egui::Response {
        let (fg, bg) = if *on { (self.pal.on_accent, self.pal.accent) } else { (self.pal.text, self.pal.bg) };
        let response = ui.add(egui::Button::new(RichText::new(text).color(fg)).fill(bg));
        if response.clicked() {
            *on = !*on;
        }
        response
    }

    /// A labelled box that works as a slider: click or drag across it to set `value`
    /// (0..1). The filled part shows the value; 0 reads as `zero_text`.
    pub(super) fn slider_box(
        &self,
        ui: &mut egui::Ui,
        value: &mut f32,
        label: &str,
        zero_text: &str,
        width: f32,
    ) -> egui::Response {
        let pal = self.pal;
        let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click_and_drag());
        response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if (response.dragged() || response.clicked())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            // Snap the first few percent to zero so "off" is easy to hit.
            let v = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let v = if v < 0.03 { 0.0 } else { (v * 100.0).round() / 100.0 };
            if v != *value {
                *value = v;
                response.mark_changed();
            }
        }
        let painter = ui.painter_at(rect);
        let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * *value, rect.height()));
        painter.rect_filled(fill, CornerRadius::same(widgets::RADIUS), pal.accent.gamma_multiply(0.35));
        let hot = response.hovered() || response.dragged();
        if hot {
            painter.rect_filled(rect, CornerRadius::same(widgets::RADIUS), pal.accent.gamma_multiply(0.08));
        }
        if *value > 0.0 {
            let x = (rect.left() + rect.width() * *value).clamp(rect.left() + 2.0, rect.right() - 2.0);
            painter.vline(x, rect.top() + 6.0..=rect.bottom() - 6.0, Stroke::new(2.0, pal.accent));
        }
        let border = if hot { pal.dim } else { pal.line };
        painter.rect_stroke(rect, CornerRadius::same(widgets::RADIUS), Stroke::new(1.0, border), StrokeKind::Inside);
        let on = *value > 0.0;
        let icon = Rect::from_center_size(rect.left_center() + Vec2::new(16.0, 0.0), Vec2::splat(15.0));
        icons::paint(&painter, Glyph::Smooth, icon, if on { pal.accent } else { pal.dim });
        let font = FontId::new(13.0, FontFamily::Proportional);
        painter.text(rect.left_center() + Vec2::new(32.0, 0.0), Align2::LEFT_CENTER, label, font.clone(), pal.text);
        let shown = if on { percent(*value) } else { zero_text.to_string() };
        let color = if on { pal.text } else { pal.dim };
        painter.text(rect.right_center() - Vec2::new(10.0, 0.0), Align2::RIGHT_CENTER, shown, font, color);
        response
    }

    /// Small pill switch used in panel headers and settings.
    pub(super) fn switch(&self, ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
        let pal = self.pal;
        let (rect, mut response) = ui.allocate_exact_size(Vec2::new(32.0, 18.0), Sense::click());
        response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            *on = !*on;
            response.mark_changed();
        }
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let painter = ui.painter_at(rect.expand(1.0));
        let track = if response.hovered() { mix(pal.line, pal.dim, 0.3) } else { pal.line };
        painter.rect_filled(rect, CornerRadius::same(9), mix(track, pal.accent, t));
        painter.circle_filled(
            Pos2::new(rect.left() + 9.0 + t * 14.0, rect.center().y),
            6.5,
            mix(pal.dim, pal.on_accent, t),
        );
        response
    }

    pub(super) fn labeled_switch(&self, ui: &mut egui::Ui, on: &mut bool, text: &str, hint: Option<&str>) {
        ui.horizontal(|ui| {
            let label = ui.label(text);
            if let Some(hint) = hint {
                label.on_hover_text(hint);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| self.switch(ui, on));
        });
    }

    pub(super) fn panel_frame(&self) -> Frame {
        Frame::new()
            .fill(self.pal.panel)
            .stroke(Stroke::new(1.0, self.pal.line.gamma_multiply(0.8)))
            .corner_radius(CornerRadius::same(widgets::RADIUS + 4))
            .inner_margin(Margin::symmetric(8, 10))
    }

    /// A yes and a no, small, for the actions that cannot be undone: (yes, no) pressed.
    pub(super) fn confirm_buttons(&self, ui: &mut egui::Ui, s: &Settings) -> (bool, bool) {
        let pal = self.pal;
        let yes = widgets::icon_button(ui, &pal, Icon::Draw(Glyph::Check), true).on_hover_text(self.t(s, "yes"));
        let no = widgets::icon_button(ui, &pal, Icon::Draw(Glyph::Close), false).on_hover_text(self.t(s, "no"));
        (yes.clicked(), no.clicked())
    }

    /// A done state: a green check and a line of text.
    pub(super) fn ok_label(&self, ui: &mut egui::Ui, text: &str) {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        icons::paint(ui.painter(), Glyph::Check, rect, self.pal.ok);
        ui.label(RichText::new(text).color(self.pal.dim));
    }

    /// Heading of a group in the settings.
    pub(super) fn section(&self, ui: &mut egui::Ui, title: &str) {
        ui.add_space(16.0);
        ui.label(RichText::new(title).font(FontId::new(12.0, widgets::semibold())).color(self.pal.dim));
        ui.add_space(2.0);
    }
}
