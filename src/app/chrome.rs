//! The frameless window: the top bar, its menus, dragging and resizing.

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Layout, Rect, RichText, Sense, Stroke, StrokeKind,
    Vec2,
};

use super::style::native_font;
use super::{App, MIN_WINDOW};
use crate::config::Settings;
use crate::i18n::{Language, tr};
use crate::tracker::Snapshot;
use crate::ui::icons::{self, Glyph};
use crate::ui::theme::{Palette, Scheme};
use crate::ui::widgets::{self, Button, Icon, Look};

/// Width of the strip along the window edge that starts a resize (the window has no frame).
const RESIZE_BAND: f32 = 5.0;

/// The empty part of a frameless window's top bar drags the window, and a double click
/// maximizes it. Call it before the controls drawn over the bar, so they win the pointer.
/// Returns whether the window is maximized.
pub(super) fn drag_window(ui: &mut egui::Ui, id: &str) -> bool {
    let drag = ui.interact(ui.max_rect(), ui.make_persistent_id(id), Sense::click_and_drag());
    let ctx = ui.ctx().clone();
    let inside = ctx.pointer_hover_pos().is_some_and(|p| ctx.content_rect().shrink(RESIZE_BAND * 2.0).contains(p));
    if drag.drag_started_by(egui::PointerButton::Primary) && inside {
        ui.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    let maximized = ui.input(|i| i.viewport().maximized).unwrap_or(false);
    if drag.double_clicked() {
        ui.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    maximized
}

/// Close, maximize or restore, and minimize, added right to left. Returns whether close
/// was pressed.
pub(super) fn window_controls(ui: &mut egui::Ui, pal: &Palette, maximized: bool) -> bool {
    let close = widgets::window_button(ui, pal, Glyph::Close, true).clicked();
    let glyph = if maximized { Glyph::Restore } else { Glyph::Maximize };
    if widgets::window_button(ui, pal, glyph, false).clicked() {
        ui.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    if widgets::window_button(ui, pal, Glyph::Minimize, false).clicked() {
        ui.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
    close
}

/// Turns the outer `RESIZE_BAND` points of a frameless window into resize handles.
pub(super) fn resize_edges(ctx: &egui::Context) {
    use egui::viewport::ResizeDirection as Edge;
    let (maximized, fullscreen) = ctx.input(|i| {
        let v = i.viewport();
        (v.maximized.unwrap_or(false), v.fullscreen.unwrap_or(false))
    });
    if maximized || fullscreen {
        return;
    }
    let Some(pos) = ctx.pointer_hover_pos() else { return };
    let rect = ctx.content_rect();
    let (left, right) = (pos.x - rect.left(), rect.right() - pos.x);
    let (top, bottom) = (pos.y - rect.top(), rect.bottom() - pos.y);
    let corner = RESIZE_BAND * 2.0;
    let edge = if left < corner && top < corner {
        Edge::NorthWest
    } else if right < corner && top < corner {
        Edge::NorthEast
    } else if left < corner && bottom < corner {
        Edge::SouthWest
    } else if right < corner && bottom < corner {
        Edge::SouthEast
    } else if left < RESIZE_BAND {
        Edge::West
    } else if right < RESIZE_BAND {
        Edge::East
    } else if top < RESIZE_BAND {
        Edge::North
    } else if bottom < RESIZE_BAND {
        Edge::South
    } else {
        return;
    };
    ctx.set_cursor_icon(match edge {
        Edge::North | Edge::South => egui::CursorIcon::ResizeVertical,
        Edge::East | Edge::West => egui::CursorIcon::ResizeHorizontal,
        Edge::NorthWest | Edge::SouthEast => egui::CursorIcon::ResizeNwSe,
        Edge::NorthEast | Edge::SouthWest => egui::CursorIcon::ResizeNeSw,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(edge));
    }
}

impl App {
    pub(super) fn top_bar(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        // The window has no system title bar.
        let maximized = drag_window(ui, "title_drag");
        ui.horizontal(|ui| {
            if let Some(mark) = &self.mark {
                ui.add(egui::Image::new((mark.id(), Vec2::splat(26.0))));
            }
            // A window saved narrower than the minimum still has to fit its bar: icons only.
            let tight = ui.available_width() < 940.0;
            if !tight {
                ui.label(RichText::new("VRC Openface").font(FontId::new(16.0, widgets::semibold())));
                ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).size(11.0).color(pal.dim));
                ui.add_space(12.0);
            }
            let (color, text) = self.status_line(s, snapshot);
            // Sized for the longest text it shows, so the controls after it stay put.
            let longest = format!("{} · 99 FPS", self.t(s, "tracking"));
            widgets::pill(ui, &pal, color, &text, &longest);
            let (look, glyph, label) = if s.ft_output {
                (Look::Secondary, Glyph::Pause, self.t(s, "tray_pause"))
            } else {
                (Look::Primary, Glyph::Play, self.t(s, "resume"))
            };
            let pause = if tight {
                widgets::icon_button(ui, &pal, Icon::Draw(glyph), !s.ft_output)
            } else {
                let other = if s.ft_output { self.t(s, "resume") } else { self.t(s, "tray_pause") };
                widgets::button(ui, &pal, Button::new(label).glyph(glyph).look(look).reserve(other))
            };
            if pause.on_hover_text(self.t(s, "ft_hint")).clicked() {
                s.ft_output = !s.ft_output;
            }
            ui.add_space(4.0);
            let labels = [self.t(s, "view_simple"), self.t(s, "view_advanced")];
            let mut view = usize::from(s.advanced);
            if widgets::segmented(ui, &pal, &labels, &mut view) {
                s.advanced = view == 1;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if window_controls(ui, &pal, maximized) {
                    ui.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                ui.add_space(8.0);
                let gear = widgets::icon_button(ui, &pal, Icon::Draw(Glyph::Gear), self.settings_open);
                if gear.on_hover_text(self.t(s, "settings")).clicked() {
                    self.settings_open = !self.settings_open;
                }
                let theme = widgets::icon_button(ui, &pal, Icon::Draw(Glyph::Swatch), false)
                    .on_hover_text(self.t(s, "color_scheme"));
                self.theme_menu(&theme, s);
                self.language_menu(ui, s);
                if !tight {
                    ui.add_space(4.0);
                    for (texture, url) in self.links.iter().rev() {
                        if widgets::icon_button(ui, &pal, Icon::Image(texture.id()), false)
                            .on_hover_text(*url)
                            .clicked()
                        {
                            crate::system::open_url(url);
                        }
                    }
                }
            });
        });
    }

    pub(super) fn flag(&self, language: Language) -> Option<egui::Image<'static>> {
        let (_, texture) = self.flags.iter().find(|(code, _)| *code == language.code())?;
        Some(egui::Image::new((texture.id(), Vec2::new(24.0, 16.0))))
    }

    /// Current language with its flag; opens the list of languages.
    fn language_menu(&self, ui: &mut egui::Ui, s: &mut Settings) {
        let pal = self.pal;
        let name = s.language.native_name();
        let font = FontId::new(12.5, native_font(s.language));
        let text_width = widgets::text_width(ui, name, font.clone());
        let size = Vec2::new(8.0 + 24.0 + 8.0 + text_width + 8.0 + 12.0 + 8.0, 30.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        let painter = ui.painter_at(rect.expand(1.0));
        let fill = if response.hovered() { pal.hover() } else { pal.raised };
        painter.rect_filled(rect, CornerRadius::same(widgets::RADIUS), fill);
        let border = if response.hovered() { pal.dim } else { pal.line };
        painter.rect_stroke(rect, CornerRadius::same(widgets::RADIUS), Stroke::new(1.0, border), StrokeKind::Inside);
        let flag_rect = Rect::from_min_size(rect.left_center() + Vec2::new(8.0, -8.0), Vec2::new(24.0, 16.0));
        if let Some(flag) = self.flag(s.language) {
            flag.paint_at(ui, flag_rect);
        }
        painter.text(flag_rect.right_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, name, font, pal.text);
        let chevron = Rect::from_center_size(rect.right_center() - Vec2::new(14.0, 0.0), Vec2::splat(12.0));
        icons::paint(&painter, Glyph::ChevronDown, chevron, pal.dim);
        egui::Popup::menu(&response).show(|ui| {
            ui.set_width(190.0);
            for language in Language::ALL {
                let flag = self.flag(language);
                if self
                    .menu_row(ui, s.language == language, language.native_name(), native_font(language), flag, None)
                    .clicked()
                {
                    s.language = language;
                }
            }
        });
    }

    /// Color scheme button's dropdown: one row per scheme, then the custom colors.
    fn theme_menu(&self, response: &egui::Response, s: &mut Settings) {
        let language = s.language;
        egui::Popup::menu(response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_width(210.0);
            for scheme in Scheme::ALL {
                let swatch = Palette::of(scheme, &s.custom_colors);
                let label = tr(language, scheme.text_key());
                if self
                    .menu_row(
                        ui,
                        s.scheme == scheme,
                        label,
                        FontFamily::Proportional,
                        None,
                        Some((swatch.bg, swatch.accent)),
                    )
                    .clicked()
                {
                    s.scheme = scheme;
                }
            }
            if s.scheme == Scheme::Custom {
                ui.add_space(6.0);
                // The picker opens inside this menu: a popup of its own would count as a click outside
                // it, and the menu would close with the picker.
                let open_id = egui::Id::unique("custom-color-open");
                let mut open: Option<usize> = ui.data(|d| d.get_temp(open_id)).flatten();
                let outline = Stroke::new(1.0, self.pal.dim);
                let c = &mut s.custom_colors;
                egui::Grid::new("custom-colors").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                    for (i, (key, color)) in [
                        ("theme_background", &c.background),
                        ("theme_panel", &c.panel),
                        ("theme_text", &c.text),
                        ("theme_accent", &c.accent),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        ui.label(tr(language, key));
                        let (rect, response) = ui.allocate_exact_size(Vec2::new(44.0, 24.0), Sense::click());
                        let radius = CornerRadius::same(widgets::RADIUS);
                        ui.painter().rect_filled(rect, radius, Color32::from_rgb(color[0], color[1], color[2]));
                        let edge = if open == Some(i) { Stroke::new(1.5, self.pal.accent) } else { outline };
                        ui.painter().rect_stroke(rect, radius, edge, StrokeKind::Outside);
                        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            open = if open == Some(i) { None } else { Some(i) };
                        }
                        ui.end_row();
                    }
                });
                if let Some(i) = open {
                    let color = match i {
                        0 => &mut c.background,
                        1 => &mut c.panel,
                        2 => &mut c.text,
                        _ => &mut c.accent,
                    };
                    ui.add_space(6.0);
                    ui.spacing_mut().slider_width = 180.0;
                    let mut picked = Color32::from_rgb(color[0], color[1], color[2]);
                    if egui::color_picker::color_picker_color32(ui, &mut picked, egui::color_picker::Alpha::Opaque) {
                        *color = [picked.r(), picked.g(), picked.b()];
                    }
                }
                ui.data_mut(|d| d.insert_temp(open_id, open));
            }
        });
    }

    /// Row of a dropdown list: tinted on hover so the one about to be picked is obvious,
    /// filled with the accent when it is the current choice.
    fn menu_row(
        &self,
        ui: &mut egui::Ui,
        selected: bool,
        label: &str,
        family: FontFamily,
        flag: Option<egui::Image<'static>>,
        swatch: Option<(Color32, Color32)>,
    ) -> egui::Response {
        let pal = self.pal;
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        let (fg, fill) = if selected {
            (pal.on_accent, Some(pal.accent))
        } else if response.hovered() {
            (pal.text, Some(pal.accent.gamma_multiply(0.4)))
        } else {
            (pal.text, None)
        };
        let painter = ui.painter();
        if let Some(fill) = fill {
            painter.rect_filled(rect, CornerRadius::same(widgets::RADIUS), fill);
        }
        let icon = Rect::from_min_size(rect.left_center() + Vec2::new(8.0, -8.0), Vec2::new(24.0, 16.0));
        if let Some(flag) = flag {
            flag.paint_at(ui, icon);
        } else if let Some((background, accent)) = swatch {
            let painter = ui.painter();
            painter.rect_filled(icon, CornerRadius::same(widgets::RADIUS), background);
            painter.rect_filled(
                Rect::from_min_max(icon.center_top(), icon.right_bottom()),
                CornerRadius::same(widgets::RADIUS),
                accent,
            );
            painter.rect_stroke(
                icon,
                CornerRadius::same(widgets::RADIUS),
                Stroke::new(1.0, pal.dim),
                StrokeKind::Inside,
            );
        }
        let font = FontId::new(13.0, family);
        ui.painter().text(icon.right_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, label, font, fg);
        response
    }

    /// The window has no system frame, so its edges and corners resize it here. The minimum
    /// size is enforced by the system while dragging; a size restored from an older run
    /// (or from a smaller minimum) is raised once at the start.
    pub(super) fn window_edges(&mut self, ctx: &egui::Context) {
        let inner = ctx.input(|i| i.viewport().inner_rect);
        if !self.size_checked
            && let Some(inner) = inner
        {
            self.size_checked = true;
            if inner.width() < MIN_WINDOW.x || inner.height() < MIN_WINDOW.y {
                let size = inner.size().max(MIN_WINDOW);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
        }
        resize_edges(ctx);
    }
}
