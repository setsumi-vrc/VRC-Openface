//! The three linked parameter panels (MediaPipe, Unified Expressions, Avatar), their toolbar
//! and the lines between them.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

use eframe::egui::{
    self, Align, Align2, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind, Vec2,
};

use super::controls::{header, truncated};
use super::rows::{Row, bit_row, paint_meter, row_inset, value_row};
use super::{App, Hover, REVEAL_AFTER, ROW_HEIGHT, Rows, percent};
use crate::channels::{self, COUNT};
use crate::config::Settings;
use crate::i18n::tr;
use crate::mapping::OUTPUTS;
use crate::processing::{self, AvatarCalibration, ChannelCalibration, Source};
use crate::tracker::{Snapshot, TongueStatus};
use crate::ui::icons::{self, Glyph};
use crate::ui::widgets;
use crate::vrchat::osc::Binding;

const MAPPING_DOCS: &str = "https://docs.vrcft.io/docs/tutorial-avatars/tutorial-avatars-extras/parameters";

/// The search text, lowercased once for all the rows it filters.
struct Search(String);

impl Search {
    fn new(text: &str) -> Self {
        Self(text.trim().to_lowercase())
    }

    fn matches(&self, text: &str) -> bool {
        self.0.is_empty() || text.to_lowercase().contains(&self.0)
    }

    /// A channel matches by its name or its friendly label.
    fn matches_channel(&self, index: usize) -> bool {
        self.0.is_empty() || self.matches(channels::name(index)) || self.matches(&channels::label(index))
    }

    fn matches_output(&self, output: &crate::mapping::Output) -> bool {
        self.matches(output.name) || self.matches(output.friendly)
    }
}

/// One channel in the MediaPipe panel and how to draw it.
struct Cell<'a> {
    index: usize,
    width: f32,
    /// The avatar reads it (or nothing is known yet); unused rows are dimmed.
    used: bool,
    /// Differs from the profile in use, or from the defaults.
    modified: bool,
    /// Linked to the hovered row.
    highlight: bool,
    /// Key of the avatar's calibration.
    avatar: &'a str,
}

/// Rows connected to the hovered one, highlighted in every panel.
struct Related {
    channels: [bool; COUNT],
    outputs: Vec<bool>,
}

impl App {
    fn related(&self) -> Related {
        let mut related = Related { channels: [false; COUNT], outputs: vec![false; OUTPUTS.len()] };
        match self.hover {
            Some(Hover::Channel(c)) => {
                related.channels[c] = true;
                for (o, flag) in related.outputs.iter_mut().enumerate() {
                    *flag = self.mapper.dependencies(o).contains(&c);
                }
            }
            Some(Hover::Output(o) | Hover::Avatar(o)) => {
                related.outputs[o] = true;
                self.mapper.dependencies(o).iter().for_each(|&c| related.channels[c] = true);
            }
            None => {}
        }
        related
    }

    /// Search and the switches that apply to all three tables: one row, or two when narrow.
    pub(super) fn tools_row(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        // Two groups that stay whole: one row when both fit, otherwise one row each.
        let stacked = ui.available_width() < 1140.0;
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        if stacked {
            ui.horizontal(|ui| self.tools_filters(ui, s));
            ui.horizontal(|ui| self.tools_options(ui, s, snapshot));
        } else {
            ui.horizontal(|ui| {
                self.tools_filters(ui, s);
                ui.add_space(6.0);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 20.0), Sense::hover());
                ui.painter().rect_filled(rect, 0, self.pal.line);
                ui.add_space(6.0);
                self.tools_options(ui, s, snapshot);
            });
        }
    }

    /// Search field: the whole box is the click target, it lights up while typing.
    fn search_field(&mut self, ui: &mut egui::Ui, s: &Settings) {
        let pal = self.pal;
        let hint = self.t(s, "search");
        let id = ui.make_persistent_id("search_field");
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width().min(260.0), 32.0), Sense::click());
        let focused = ui.memory(|m| m.has_focus(id));
        if response.clicked() {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let hot = response.hovered() || focused;
        let painter = ui.painter_at(rect.expand(3.0));
        let radius = CornerRadius::same(widgets::RADIUS);
        let fill = if focused {
            pal.bg
        } else if hot {
            pal.hover()
        } else {
            pal.raised
        };
        let stroke =
            if focused { Stroke::new(1.5, pal.accent) } else { Stroke::new(1.0, if hot { pal.dim } else { pal.line }) };
        painter.rect(rect, radius, fill, stroke, StrokeKind::Inside);
        let icon = Rect::from_center_size(rect.left_center() + Vec2::new(17.0, 0.0), Vec2::splat(15.0));
        icons::paint(&painter, Glyph::Search, icon, if focused { pal.accent } else { pal.dim });
        let inner = Rect::from_min_max(rect.min + Vec2::new(34.0, 0.0), rect.max - Vec2::new(32.0, 0.0));
        let mut field =
            ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
        field.add(
            egui::TextEdit::singleline(&mut self.search)
                .id(id)
                .hint_text(hint)
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .align(Align2::LEFT_CENTER)
                .min_size(Vec2::new(inner.width(), 32.0))
                .desired_width(inner.width()),
        );
        if !self.search.is_empty() {
            let close = Rect::from_center_size(rect.right_center() - Vec2::new(17.0, 0.0), Vec2::splat(22.0));
            let clear = ui.interact(close, id.with("clear"), Sense::click());
            if clear.hovered() {
                painter.circle_filled(close.center(), 10.0, pal.line);
            }
            let color = if clear.hovered() { pal.text } else { pal.dim };
            icons::paint(&painter, Glyph::Close, close.shrink(5.0), color);
            if clear.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.search.clear();
            }
        }
    }

    /// Search and the switches that choose which rows the tables show.
    fn tools_filters(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        let pal = self.pal;
        self.search_field(ui, s);
        let unused = self.t(s, "unused");
        widgets::switch_toggle(ui, &pal, &mut s.show_unused, unused);
        self.filter_controls(ui, s);
    }

    /// How the values are processed, and the reset for the rows.
    fn tools_options(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        let avatar = snapshot.avatar_id.clone().unwrap_or_default();
        let hint = self.t(s, "symmetric_hint");
        let label = self.t(s, "symmetric_face");
        widgets::chip_sized(ui, &pal, Glyph::Symmetry, label, &mut s.processing.symmetric_face, 32.0)
            .on_hover_text(hint);
        let (name, off) = (self.t(s, "smoothing"), self.t(s, "smoothing_off"));
        let hint = self.t(s, "smoothing_hint");
        self.slider_box(ui, &mut s.processing.smoothing, name, off, 190.0).on_hover_text(hint);
        self.reset_all_button(ui, s, &avatar);
    }

    /// The group filter and the switch that keeps only the changed parameters.
    fn filter_controls(&mut self, ui: &mut egui::Ui, s: &Settings) {
        let pal = self.pal;
        let all = self.t(s, "filter_all");
        let current = match self.filter_group {
            0 => all,
            group => self.t(s, channels::CATEGORIES[group - 1].0),
        };
        egui::ComboBox::from_id_salt("filter-group").width(120.0).selected_text(current).show_ui(ui, |ui| {
            ui.selectable_value(&mut self.filter_group, 0, all);
            for (group, (key, _)) in channels::CATEGORIES.iter().enumerate() {
                ui.selectable_value(&mut self.filter_group, group + 1, tr(s.language, key));
            }
        });
        let modified = self.t(s, "modified");
        widgets::switch_toggle(ui, &pal, &mut self.filter_modified, modified);
    }

    fn avatar_known(snapshot: &Snapshot) -> bool {
        !snapshot.avatar_ft_params.is_empty()
    }

    fn used_channels(&self, snapshot: &Snapshot) -> [bool; COUNT] {
        let known = Self::avatar_known(snapshot);
        let mut used = [!known; COUNT];
        if known {
            for (i, binding) in snapshot.bindings.iter().enumerate() {
                if binding.is_some() {
                    self.mapper.dependencies(i).iter().for_each(|&d| used[d] = true);
                }
            }
        }
        used
    }

    fn channel_label(&self, s: &Settings, index: usize) -> String {
        if s.friendly_names { channels::label(index) } else { channels::name(index).to_string() }
    }

    fn channel_cell(&self, ui: &mut egui::Ui, s: &Settings, snapshot: &Snapshot, cell: Cell<'_>) -> egui::Response {
        let Cell { index, width, used, modified, highlight, avatar } = cell;
        let pal = self.pal;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::click());
        let painter = ui.painter_at(rect);
        if response.hovered() || highlight {
            painter.rect_filled(row_inset(rect), CornerRadius::same(5), pal.line.gamma_multiply(0.5));
        }
        // The row whose editor is open, so it is clear which one the editor belongs to.
        if self.expanded.is_some_and(|open| open == index || channels::mirror(index) == Some(open)) {
            painter.rect_filled(row_inset(rect), CornerRadius::same(5), pal.accent.gamma_multiply(0.18));
            let bar = Rect::from_min_size(rect.left_top() + Vec2::new(2.0, 5.0), Vec2::new(3.0, rect.height() - 10.0));
            painter.rect_filled(bar, CornerRadius::same(2), pal.accent);
        }
        let value = snapshot.values[index];
        paint_meter(&painter, rect, &pal, value, false, used);
        let color = if used { pal.text } else { pal.dim };
        // Room for the value on the right and the markers after the name.
        let galley =
            truncated(&painter, self.channel_label(s, index), FontId::proportional(13.0), color, rect.width() - 96.0);
        let text_pos = rect.left_center() + Vec2::new(12.0, -galley.size().y / 2.0);
        let text_right = text_pos.x + galley.size().x;
        painter.galley(text_pos, galley, color);
        let mut x = text_right + 9.0;
        painter.circle_filled(Pos2::new(x, rect.center().y), 2.2, if used { pal.accent } else { pal.line });
        if modified {
            // Changed and not saved: a small pencil; the hover says what.
            x += 14.0;
            let icon = Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::splat(12.0));
            icons::paint(&painter, Glyph::Edited, icon, pal.accent);
            x += 2.0;
        }
        if s.processing.source(index) == Source::Mesh {
            // Taken from the face mesh instead of the model: a small outlined square.
            x += 12.0;
            let mark = Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::splat(7.0));
            painter.rect_stroke(mark, CornerRadius::same(2), Stroke::new(1.0, pal.dim), StrokeKind::Inside);
        }
        painter.text(
            rect.right_center() - Vec2::new(12.0, 0.0),
            Align2::RIGHT_CENTER,
            percent(value),
            FontId::proportional(12.5),
            if used { pal.text } else { pal.dim },
        );
        response.on_hover_ui(|ui| {
            let input = percent(snapshot.input[index]);
            ui.label(format!("{} · {input} {}", channels::name(index), self.t(s, "current_input")));
            let changes = {
                let store = self.shared.calibrations.lock();
                let calibration = store.avatars.get(avatar);
                self.modifications(s, calibration.unwrap_or(&AvatarCalibration::default()), index)
            };
            if !changes.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(self.t(s, "modified")).strong().color(pal.accent));
                for line in changes {
                    ui.label(line);
                }
            }
        })
    }

    /// Whether the parameter differs from the saved profile in use (the defaults when there
    /// is none): range, override, linked sides, smoothing, source or symmetry.
    fn is_modified(&self, s: &Settings, calibration: &AvatarCalibration, index: usize) -> bool {
        let base = &self.baseline;
        let name = channels::name(index);
        let paired = channels::mirror(index).is_some();
        calibration.get(index) != base.calibration.get(index)
            || (paired && calibration.linked(index) != base.calibration.linked(index))
            || s.processing.smoothing_channels.get(name) != base.processing.smoothing_channels.get(name)
            || s.processing.source(index) != base.processing.source(index)
            || channels::symmetry_group(index)
                .is_some_and(|group| s.processing.symmetry.get(group) != base.processing.symmetry.get(group))
    }

    /// One line for each way [`Self::is_modified`] finds the parameter changed.
    fn modifications(&self, s: &Settings, calibration: &AvatarCalibration, index: usize) -> Vec<String> {
        let base = &self.baseline;
        let (c, was) = (calibration.get(index), base.calibration.get(index));
        let name = channels::name(index);
        let mut lines = Vec::new();
        if (c.min, c.max) != (was.min, was.max) {
            lines.push(format!("{} {}–{}", self.t(s, "range"), percent(c.min), percent(c.max)));
        }
        if c.overriding() != was.overriding() {
            let shown = c.overriding().map_or_else(|| self.t(s, "smoothing_off").to_string(), percent);
            lines.push(format!("{} {shown}", self.t(s, "override")));
        }
        if channels::mirror(index).is_some() && calibration.linked(index) != base.calibration.linked(index) {
            let key = if calibration.linked(index) { "linked" } else { "unlinked" };
            lines.push(self.t(s, key).to_string());
        }
        if s.processing.smoothing_channels.get(name) != base.processing.smoothing_channels.get(name) {
            let strength = s.processing.smoothing_for(index);
            let shown = if strength > 0.0 { percent(strength) } else { self.t(s, "smoothing_off").to_string() };
            lines.push(format!("{} {shown}", self.t(s, "smoothing")));
        }
        if s.processing.source(index) != base.processing.source(index) {
            let source = if s.processing.source(index) == Source::Mesh { "mesh" } else { "model" };
            lines.push(format!("{}: {}", self.t(s, "source"), self.t(s, source)));
        }
        if let Some(group) = channels::symmetry_group(index)
            && s.processing.symmetry.get(group) != base.processing.symmetry.get(group)
        {
            let state = if s.processing.group_symmetric(group) { self.t(s, "on") } else { self.t(s, "smoothing_off") };
            lines.push(format!("{}: {state}", self.t(s, "symmetry")));
        }
        lines
    }

    /// Calibration, source, symmetry and smoothing for one row (one channel or a L/R pair).
    fn row_editor(&self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot, members: &[usize], avatar: &str) {
        let pal = self.pal;
        let mut store = self.shared.calibrations.lock();
        let calibration = store.avatars.entry(avatar.to_string()).or_default();
        let mut changed = false;
        let outline = Stroke::new(1.0, pal.accent.gamma_multiply(0.6));
        Frame::new().fill(pal.bg).stroke(outline).inner_margin(Margin::same(8)).show(ui, |ui| {
            // The channel list lays rows out without spacing; the editor needs the normal gaps.
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
            ui.horizontal_wrapped(|ui| {
                if members.len() > 1 {
                    let mut linked = calibration.linked(members[0]);
                    let text = if linked { "L = R" } else { "L ≠ R" };
                    let hint = if linked { self.t(s, "linked") } else { self.t(s, "unlinked") };
                    if self.toggle(ui, &mut linked, text).on_hover_text(hint).clicked() {
                        calibration.set_linked(members[0], linked);
                        changed = true;
                    }
                }
                if processing::has_mesh_source(members[0]) {
                    ui.label(RichText::new(self.t(s, "source")).color(pal.dim));
                    for (source, key) in [(Source::Model, "model"), (Source::Mesh, "mesh")] {
                        let mut on = s.processing.source(members[0]) == source;
                        if self.toggle(ui, &mut on, self.t(s, key)).clicked() {
                            for &m in members {
                                s.processing.sources.insert(channels::name(m).to_string(), source);
                            }
                        }
                    }
                }
                if let Some(group) = channels::symmetry_group(members[0]) {
                    let mut on = s.processing.group_symmetric(group);
                    if ui.checkbox(&mut on, self.t(s, "symmetry")).changed() {
                        s.processing.symmetry.insert(group.to_string(), on);
                    }
                }
                ui.label(RichText::new(self.t(s, "smoothing")).color(pal.dim));
                let mut strength = s.processing.smoothing_for(members[0]);
                if ui.add(egui::Slider::new(&mut strength, 0.0..=1.0).show_value(false)).changed() {
                    for &m in members {
                        s.processing.smoothing_channels.insert(channels::name(m).to_string(), strength);
                    }
                }
                ui.label(if strength > 0.0 { percent(strength) } else { self.t(s, "smoothing_off").to_string() });
            });
            ui.add_space(4.0);
            let sides: Vec<usize> =
                if members.len() > 1 && calibration.linked(members[0]) { vec![members[0]] } else { members.to_vec() };
            for &index in &sides {
                let mut c = calibration.get(index);
                ui.horizontal(|ui| {
                    if sides.len() > 1 {
                        ui.label(
                            RichText::new(if channels::name(index).ends_with("Left") { "L" } else { "R" }).strong(),
                        );
                    }
                    ui.label(RichText::new(self.t(s, "range")).color(pal.dim));
                    let mut min = c.min;
                    let mut max = c.max;
                    let a = ui.add(egui::Slider::new(&mut min, 0.0..=1.0).show_value(false));
                    let b = ui.add(egui::Slider::new(&mut max, 0.0..=1.0).show_value(false));
                    ui.label(format!("{}–{}", percent(min), percent(max)));
                    ui.label(RichText::new(format!("· {}", percent(snapshot.input[index]))).color(pal.dim))
                        .on_hover_text(self.t(s, "current_input"));
                    if a.changed() || b.changed() {
                        c.min = min.min(max - 0.05).max(0.0);
                        c.max = max.max(c.min + 0.05).min(1.0);
                        calibration.set(index, c);
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    let mut overriding = c.overriding().is_some();
                    if ui.checkbox(&mut overriding, self.t(s, "override")).changed() {
                        c.override_value = Some(c.override_value.unwrap_or(0.0));
                        c.override_disabled = !overriding;
                        calibration.set(index, c);
                        changed = true;
                    }
                    if overriding {
                        let mut value = c.override_value.unwrap_or(0.0);
                        if ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false)).changed() {
                            c.override_value = Some(value);
                            calibration.set(index, c);
                            changed = true;
                        }
                        ui.label(percent(value));
                    }
                });
            }
            if ui.button(self.t(s, "reset_rows")).clicked() {
                for &m in members {
                    calibration.set(m, ChannelCalibration::default());
                    let name = channels::name(m);
                    s.processing.smoothing_channels.remove(name);
                    s.processing.sources.remove(name);
                }
                if let Some(group) = channels::symmetry_group(members[0]) {
                    s.processing.symmetry.remove(group);
                }
                changed = true;
            }
        });
        if changed {
            self.shared.calibrations_dirty.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn mediapipe_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot, simple: bool) {
        let used = self.used_channels(snapshot);
        let tongue = s.tongue;
        let show_unused = if simple { s.simple_unused } else { s.show_unused };
        let (group, only_modified, open) = (self.filter_group, self.filter_modified, self.expanded);
        self.baseline = {
            let store = self.shared.calibrations.lock();
            self.profile_active.as_ref().and_then(|name| store.profiles.get(name)).cloned().unwrap_or_default()
        };
        let modified: Vec<bool> = {
            let store = self.shared.calibrations.lock();
            let calibration = store.avatars.get(snapshot.avatar_id.as_deref().unwrap_or_default());
            let fallback = AvatarCalibration::default();
            (0..COUNT).map(|i| self.is_modified(s, calibration.unwrap_or(&fallback), i)).collect()
        };
        // A row whose editor is open stays in the list, or resetting it would make it vanish
        // and close the editor under the pointer.
        let filtered = |i: usize| {
            let editing = open.is_some_and(|e| e == i || channels::mirror(i) == Some(e));
            (group == 0 || channels::category(i) == Some(group - 1)) && (!only_modified || modified[i] || editing)
        };
        let visible = move |i: usize| !channels::is_tongue(i) || tongue;
        let total = (1..COUNT).filter(|&i| visible(i)).count();
        let active = (1..COUNT).filter(|&i| visible(i) && used[i]).count();
        let avatar = snapshot.avatar_id.clone().unwrap_or_default();
        let related = self.related();
        let frame = self.panel_frame().show(ui, |ui| {
            let (dim, title) = (self.pal.dim, self.t(s, "mediapipe"));
            let title = |ui: &mut egui::Ui| {
                ui.label(RichText::new(format!("{active}/{total}")).color(dim));
                ui.add(egui::Label::new(RichText::new(title).font(FontId::new(13.5, widgets::semibold()))).truncate());
            };
            header(ui, "mediapipe", title, |_: &mut egui::Ui| {});
            if s.tongue {
                match &snapshot.tongue {
                    TongueStatus::Missing => {
                        // Wraps: a row wider than the panel would push the panels beside it off screen.
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(self.t(s, "tongue_missing")).color(self.pal.dim).small());
                            if ui.small_button(self.t(s, "import_tongue")).clicked() {
                                self.import_tongue();
                            }
                        });
                    }
                    TongueStatus::Error(e) => {
                        let text = format!("{}: {e}", self.t(s, "tongue_model"));
                        ui.label(RichText::new(text).color(self.pal.dim).small());
                    }
                    _ => {}
                }
            }
            if simple {
                // The Advanced view has its search in the toolbar above the tables.
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    self.search_field(ui, s);
                    let unused = self.t(s, "unused");
                    widgets::switch_toggle(ui, &self.pal, &mut s.simple_unused, unused);
                });
                ui.horizontal(|ui| {
                    self.filter_controls(ui, s);
                    self.reset_all_button(ui, s, &avatar);
                });
                ui.add_space(4.0);
            }
            ui.separator();
            // After the search field, which the Simple view draws above.
            let search = Search::new(&self.search);
            egui::ScrollArea::vertical().id_salt("mp").auto_shrink([false, false]).show(ui, |ui| {
                self.rows.view[0] = ui.clip_rect();
                // Rows touch, like table lines; each group gets a dashed box.
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let full = ui.available_width();
                let half = (full / 2.0).floor();
                // Too narrow for left and right side by side: one channel per line.
                let single = half < 160.0;
                let mut shown = 0;
                for group in channels::display_rows() {
                    let rows: Vec<_> = group
                        .iter()
                        .copied()
                        .filter(|(l, r)| {
                            let show = |i: usize| {
                                visible(i) && (show_unused || used[i]) && filtered(i) && search.matches_channel(i)
                            };
                            show(*l) || r.is_some_and(show)
                        })
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    shown += rows.len();
                    for (l, r) in rows {
                        let lines: Vec<Vec<(usize, f32)>> = match r {
                            Some(r) if single => vec![vec![(l, full)], vec![(r, full)]],
                            Some(r) => vec![vec![(l, half), (r, full - half)]],
                            None => vec![vec![(l, full)]],
                        };
                        let mut clicked = false;
                        for cells in lines {
                            clicked |= ui
                                .horizontal(|ui| {
                                    let mut clicked = false;
                                    for (i, width) in cells {
                                        let spec = Cell {
                                            index: i,
                                            width,
                                            used: used[i],
                                            modified: modified[i],
                                            highlight: related.channels[i],
                                            avatar: &avatar,
                                        };
                                        let cell = self.channel_cell(ui, s, snapshot, spec);
                                        self.rows.channels.push((i, cell.rect));
                                        self.reveal(ui, 0, i, cell.rect);
                                        if cell.hovered() {
                                            self.hover_now = Some(Hover::Channel(i));
                                        }
                                        clicked |= cell.clicked();
                                        if width == half {
                                            let divider = Stroke::new(1.0, self.pal.line.gamma_multiply(0.6));
                                            ui.painter().vline(cell.rect.right(), cell.rect.y_range(), divider);
                                        }
                                    }
                                    clicked
                                })
                                .inner;
                        }
                        if clicked {
                            self.expanded = if self.expanded == Some(l) { None } else { Some(l) };
                        }
                        if self.expanded == Some(l) {
                            let members: Vec<usize> = std::iter::once(l).chain(r).collect();
                            self.row_editor(ui, s, snapshot, &members, &avatar);
                        }
                    }
                    ui.add_space(10.0);
                }
                if shown == 0 {
                    ui.add_space(12.0);
                    ui.label(RichText::new(self.t(s, "filter_empty")).color(self.pal.dim));
                }
            });
        });
        self.rows.frame[0] = frame.response.rect;
    }

    pub(super) fn unified_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        let visible: Vec<usize> = (0..OUTPUTS.len()).filter(|&i| !OUTPUTS[i].cat_ears || s.cat_ears).collect();
        let bound = visible.iter().filter(|&&i| snapshot.bindings.get(i).is_some_and(Option::is_some)).count();
        let known = Self::avatar_known(snapshot);
        let related = self.related();
        let search = Search::new(&self.search);
        let frame = self.panel_frame().show(ui, |ui| {
            let active = if known { bound } else { 0 };
            let count = format!("{active}/{}", visible.len());
            let (title, hint) = (format!("{} ↗", self.t(s, "unified")), self.t(s, "mapping_docs"));
            let title = |ui: &mut egui::Ui| {
                ui.label(RichText::new(count).color(pal.dim));
                let link =
                    egui::Label::new(RichText::new(title).font(FontId::new(13.5, widgets::semibold())).color(pal.text))
                        .truncate()
                        .sense(Sense::click());
                let link = ui.add(link).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hint);
                if link.clicked() {
                    crate::system::open_url(MAPPING_DOCS);
                }
            };
            header(ui, "unified", title, |_: &mut egui::Ui| {});
            ui.separator();
            egui::ScrollArea::vertical().id_salt("ue").auto_shrink([false, false]).show(ui, |ui| {
                self.rows.view[1] = ui.clip_rect();
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let shown: Vec<usize> = visible
                    .iter()
                    .copied()
                    .filter(|&i| {
                        let output = &OUTPUTS[i];
                        let is_bound = known && snapshot.bindings.get(i).is_some_and(Option::is_some);
                        (s.show_unused || !known || is_bound) && search.matches_output(output)
                    })
                    .collect();
                for &i in &shown {
                    let output = &OUTPUTS[i];
                    let is_bound = known && snapshot.bindings.get(i).is_some_and(Option::is_some);
                    let value = snapshot.outputs.get(i).copied().unwrap_or_default();
                    let text = output.label(s.friendly_names);
                    let mut spec = Row::new(text, value);
                    spec.active = is_bound || !known;
                    spec.signed = output.signed;
                    spec.highlight = related.outputs[i];
                    let row = value_row(ui, &pal, spec);
                    self.rows.outputs.push((i, row.rect));
                    self.reveal(ui, 1, i, row.rect);
                    if row.hovered() {
                        self.hover_now = Some(Hover::Output(i));
                    }
                    row.on_hover_text(output.name);
                }
            });
        });
        self.rows.frame[1] = frame.response.rect;
    }

    pub(super) fn avatar_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        let known = Self::avatar_known(snapshot);
        let bound: Vec<(usize, &Binding)> =
            snapshot.bindings.iter().enumerate().filter_map(|(i, b)| b.as_ref().map(|b| (i, b))).collect();
        let matched: BTreeSet<&str> = bound.iter().map(|(i, _)| OUTPUTS[*i].name).collect();
        let unmatched: Vec<&String> = snapshot
            .avatar_ft_params
            .iter()
            .filter(|p| {
                let base = p.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches("Negative");
                !matched.contains(base) && !matched.contains(p.as_str())
            })
            .collect();
        let related = self.related();
        let search = Search::new(&self.search);
        let frame = self.panel_frame().show(ui, |ui| {
            let count = format!("{}/{}", if known { bound.len() } else { 0 }, bound.len() + unmatched.len());
            let name =
                snapshot.avatar_name.clone().or_else(|| snapshot.avatar_id.clone()).unwrap_or_else(|| "—".into());
            // Without a connected VRChat this is the avatar of the last session: grey, and it says so.
            let live = snapshot.avatar_live;
            let title = format!("{} {name}", self.t(s, if live { "avatar" } else { "avatar_last" }));
            let hint = self.t(s, "avatar_last_hint");
            let title = |ui: &mut egui::Ui| {
                ui.label(RichText::new(count).color(pal.dim));
                let color = if live { pal.text } else { pal.dim };
                let text = RichText::new(title).font(FontId::new(13.5, widgets::semibold())).color(color);
                let label = ui.add(egui::Label::new(text).truncate());
                if !live {
                    label.on_hover_text(hint);
                }
            };
            header(ui, "avatar", title, |_: &mut egui::Ui| {});
            ui.separator();
            egui::ScrollArea::vertical().id_salt("avatar").auto_shrink([false, false]).show(ui, |ui| {
                self.rows.view[2] = ui.clip_rect();
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                if !known {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(self.t(s, "no_params")).color(pal.dim));
                        if !snapshot.vrchat_found {
                            ui.label(RichText::new(self.t(s, "no_vrchat")).color(pal.dim).small());
                        }
                        if !bound.is_empty() {
                            ui.label(RichText::new(self.t(s, "sending_default")).color(pal.dim).small());
                        }
                    });
                    return;
                }
                if bound.is_empty() {
                    ui.vertical_centered(|ui| ui.label(RichText::new(self.t(s, "no_ft_params")).color(pal.dim)));
                }
                for (i, binding) in &bound {
                    let output = &OUTPUTS[*i];
                    if !search.matches_output(output) {
                        continue;
                    }
                    let value = snapshot.outputs.get(*i).copied().unwrap_or_default();
                    let states: Vec<(&str, bool)> = binding.states(value).collect();
                    let bits: Vec<bool> = states.iter().take(binding.bit_count()).map(|(_, on)| *on).collect();
                    let binary = !bits.is_empty();
                    let expanded = self.expanded_bindings.contains(i);
                    let mut spec = Row::new(output.label(s.friendly_names), value);
                    spec.signed = output.signed;
                    spec.bits = binary.then_some(bits.as_slice());
                    spec.chevron = binary.then_some(expanded);
                    spec.highlight = related.outputs[*i];
                    let response = value_row(ui, &pal, spec);
                    self.rows.avatar.push((*i, response.rect));
                    self.reveal(ui, 2, *i, response.rect);
                    if response.hovered() {
                        self.hover_now = Some(Hover::Avatar(*i));
                    }
                    if binary && response.clicked() && !self.expanded_bindings.remove(i) {
                        self.expanded_bindings.insert(*i);
                    }
                    if expanded {
                        for &(address, on) in &states {
                            bit_row(ui, &pal, address, on);
                        }
                    }
                }
                if s.show_unused {
                    for name in &unmatched {
                        if search.matches(name) {
                            let mut spec = Row::new(name, 0.0);
                            spec.active = false;
                            value_row(ui, &pal, spec);
                        }
                    }
                }
            });
        });
        self.rows.frame[2] = frame.response.rect;
    }

    /// Whether a row in `panel` is linked to the hovered row, which sits in another panel.
    pub(super) fn linked(&self, panel: usize, index: usize) -> bool {
        match (self.hover, panel) {
            // The hovered row's own panel.
            (Some(Hover::Channel(_)), 0) | (Some(Hover::Output(_)), 1) | (Some(Hover::Avatar(_)), 2) => false,
            (Some(Hover::Channel(c)), _) => self.mapper.dependencies(index).contains(&c),
            (Some(Hover::Output(o) | Hover::Avatar(o)), 0) => self.mapper.dependencies(o).contains(&index),
            (Some(Hover::Output(o) | Hover::Avatar(o)), _) => index == o,
            _ => false,
        }
    }

    /// While a row stays hovered, a linked row scrolled out of view in another panel is
    /// scrolled into it, so the link has two visible ends.
    fn reveal(&mut self, ui: &egui::Ui, panel: usize, index: usize, rect: Rect) {
        if self.revealed[panel] || self.dwell.1.elapsed() < REVEAL_AFTER || !self.linked(panel, index) {
            return;
        }
        let prev = &self.prev_rows;
        let list = [&prev.channels, &prev.outputs, &prev.avatar][panel];
        if Rows::off(list, index, prev.view[panel]) == 0.0 {
            return;
        }
        self.revealed[panel] = true;
        ui.scroll_to_rect(rect, Some(Align::Center));
    }

    /// Lines from the hovered row to the rows it feeds or is fed by.
    pub(super) fn draw_links(&self, ui: &egui::Ui) {
        let Some(hover) = self.hover_now else { return };
        let (pal, rows) = (self.pal, &self.rows);
        let [mp, ue, avatar] = rows.frame;
        let painter = ui.painter();
        let stroke = Stroke::new(1.0, pal.text.gamma_multiply(0.85));
        let link = |a: Pos2, b: Pos2| painter.line_segment([a, b], stroke);
        // A row scrolled out of its panel gets an arrow at the edge the link points to.
        let marker = |x: f32, list: &[(usize, Rect)], index: usize, view: Rect| {
            let dir = Rows::off(list, index, view);
            if dir == 0.0 {
                return;
            }
            let y = if dir > 0.0 { view.bottom() - 11.0 } else { view.top() + 11.0 };
            let pill = Rect::from_center_size(Pos2::new(x, y), Vec2::new(26.0, 16.0));
            painter.rect_filled(pill, CornerRadius::same(8), pal.accent);
            let tip = Pos2::new(x, y + dir * 3.0);
            let wing = |dx: f32| Pos2::new(x + dx, y - dir * 2.5);
            painter.add(Shape::convex_polygon(vec![wing(-4.5), tip, wing(4.5)], pal.on_accent, Stroke::NONE));
        };
        let to_avatar = |output: usize, y: f32| {
            // In the narrow layout the avatar panel sits under the expressions; a line
            // would cut across them.
            if self.narrow {
                return;
            }
            if let Some(ay) = Rows::y(&rows.avatar, output, rows.view[2]) {
                link(Pos2::new(ue.right(), y), Pos2::new(avatar.left(), ay));
                marker(avatar.left() + 20.0, &rows.avatar, output, rows.view[2]);
            }
        };
        match hover {
            Hover::Channel(c) => {
                let Some(y) = Rows::y(&rows.channels, c, rows.view[0]) else { return };
                marker(mp.right() - 20.0, &rows.channels, c, rows.view[0]);
                for (o, _) in rows.outputs.iter().filter(|(o, _)| self.mapper.dependencies(*o).contains(&c)) {
                    if let Some(oy) = Rows::y(&rows.outputs, *o, rows.view[1]) {
                        link(Pos2::new(mp.right(), y), Pos2::new(ue.left(), oy));
                        marker(ue.left() + 20.0, &rows.outputs, *o, rows.view[1]);
                        to_avatar(*o, oy);
                    }
                }
            }
            Hover::Output(o) | Hover::Avatar(o) => {
                let Some(y) = Rows::y(&rows.outputs, o, rows.view[1]) else { return };
                marker(ue.left() + 20.0, &rows.outputs, o, rows.view[1]);
                for &c in self.mapper.dependencies(o) {
                    if let Some(cy) = Rows::y(&rows.channels, c, rows.view[0]) {
                        link(Pos2::new(mp.right(), cy), Pos2::new(ue.left(), y));
                        marker(mp.right() - 20.0, &rows.channels, c, rows.view[0]);
                    }
                }
                to_avatar(o, y);
            }
        }
    }
}
