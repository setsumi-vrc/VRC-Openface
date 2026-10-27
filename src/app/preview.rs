//! The camera card: device and mode controls, the preview with its overlays, and the strip of the
//! Advanced view.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, ColorImage, CornerRadius, FontId, Layout, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, TextureOptions, Vec2,
};

use super::{App, CAMERA_RATES, RESOLUTIONS, percent};
use crate::camera;
use crate::channels;
use crate::config::Settings;
use crate::mapping::output_index;
use crate::perf::{Stage, ms_since};
use crate::tracker::{Snapshot, Status, TongueStatus};
use crate::ui::icons::Glyph;
use crate::ui::widgets::{self, Icon};

/// Height of the preview in the camera strip across the top of the Advanced view.
const STRIP_PREVIEW: f32 = 230.0;
/// Width of the calibration card in the strip.
const CALIBRATION_WIDTH: f32 = 400.0;
/// The most the camera controls in the strip grow to.
const CAMERA_CONTROLS_MAX: f32 = 360.0;
/// How long the pointer rests on a blurred preview before it is shown unblurred, and
/// for how long.
const REVEAL_HOLD: Duration = Duration::from_millis(1500);
const REVEAL_FOR: Duration = Duration::from_secs(5 * 60);

/// Text on top of the camera image, on a dark chip so it stays readable on any frame.
fn overlay(painter: &egui::Painter, anchor: Pos2, align: Align2, text: String, size: f32, fg: Color32) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(size), Color32::PLACEHOLDER);
    let rect = align.anchor_size(anchor, galley.size() + Vec2::new(16.0, 8.0));
    painter.rect_filled(rect, CornerRadius::same(widgets::RADIUS), Color32::from_black_alpha(150));
    painter.galley(rect.min + Vec2::new(8.0, 4.0), galley, fg);
}

/// Width over height of the camera frame, 16:9 until one arrives.
fn frame_aspect(snapshot: &Snapshot) -> f32 {
    match snapshot.frame_size {
        (w, h) if w > 0 && h > 0 => w as f32 / h as f32,
        _ => 16.0 / 9.0,
    }
}

pub(super) fn resolution_label((width, height): (u32, u32)) -> String {
    format!("{width}\u{00d7}{height}")
}

impl App {
    pub(super) fn camera_card(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot, large: bool) {
        let pal = self.pal;
        widgets::card(&pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.camera_controls(ui, s);
            // The large preview takes the height the window leaves, minus the chips below it.
            let tallest = if large { (ui.available_height() - 48.0).max(200.0) } else { 280.0 };
            self.preview(ui, s, snapshot, tallest);
            self.camera_chips(ui, s);
        });
    }

    /// The camera's title row, device, resolution and rate.
    fn camera_controls(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        let pal = self.pal;
        ui.horizontal(|ui| {
            ui.label(widgets::title(&pal, self.t(s, "camera")));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let refresh = widgets::icon_button(ui, &pal, Icon::Draw(Glyph::Refresh), false);
                if refresh.on_hover_text(self.t(s, "refresh_cameras")).clicked() {
                    self.cameras = camera::list();
                }
                let mirror = widgets::icon_button(ui, &pal, Icon::Draw(Glyph::Mirror), s.mirror);
                if mirror.on_hover_text(self.t(s, "mirror")).clicked() {
                    s.mirror = !s.mirror;
                }
            });
        });
        let selected = self
            .cameras
            .iter()
            .find(|c| c.index == s.camera_index)
            .map(|c| format!("{}: {}", c.index, c.name))
            .unwrap_or_else(|| format!("{}: {}", s.camera_index, self.t(s, "camera")));
        egui::ComboBox::from_id_salt("camera").width(ui.available_width()).selected_text(selected).truncate().show_ui(
            ui,
            |ui| {
                for camera in &self.cameras {
                    ui.selectable_value(
                        &mut s.camera_index,
                        camera.index,
                        format!("{}: {}", camera.index, camera.name),
                    );
                }
            },
        );
        // Resolution and rate, so they do not need a trip to the settings.
        ui.horizontal(|ui| {
            let gap = ui.spacing().item_spacing.x;
            let half = ((ui.available_width() - gap) / 2.0).max(80.0);
            let current = resolution_label(s.resolution);
            let resolution =
                egui::ComboBox::from_id_salt("quick-resolution").width(half).selected_text(current).show_ui(ui, |ui| {
                    for r in RESOLUTIONS {
                        ui.selectable_value(&mut s.resolution, r, resolution_label(r));
                    }
                });
            resolution.response.on_hover_text(self.t(s, "resolution"));
            let fps = egui::ComboBox::from_id_salt("quick-fps")
                .width(half)
                .selected_text(format!("{} FPS", s.camera_fps))
                .show_ui(ui, |ui| {
                    for rate in CAMERA_RATES {
                        ui.selectable_value(&mut s.camera_fps, rate, format!("{rate} FPS"));
                    }
                });
            fps.response.on_hover_text(self.t(s, "camera_fps"));
        });
    }

    /// Preview, landmarks, cat ears and (in the Simple view) tongue switches.
    fn camera_chips(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        let pal = self.pal;
        ui.horizontal_wrapped(|ui| {
            let (preview, landmarks) = (self.t(s, "camera_preview"), self.t(s, "face_landmarks"));
            widgets::chip(ui, &pal, Glyph::Eye, preview, &mut s.show_camera);
            widgets::chip(ui, &pal, Glyph::Face, landmarks, &mut s.show_landmarks);
            let ears = format!("{}\n{}", self.t(s, "cat_ears"), self.t(s, "cat_ears_hint"));
            widgets::chip(ui, &pal, Glyph::Ears, self.t(s, "cat_ears"), &mut s.cat_ears).on_hover_text(ears);
            widgets::chip(ui, &pal, Glyph::Tongue, self.t(s, "tongue"), &mut s.tongue);
        });
    }

    /// Across the top of the Advanced view: the calibration with its profiles on the left,
    /// then the camera centered in the space that is left: controls that grow a little with
    /// the window, and a preview of the frame's own shape.
    pub(super) fn camera_strip(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        let preview = Vec2::new(STRIP_PREVIEW * frame_aspect(snapshot), STRIP_PREVIEW);
        let gap = 12.0;
        // What the card adds to its content: the margins and the stroke.
        let frame = 2.0 * 14.0 + 2.0;
        let column = Layout::top_down(Align::Min);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(Vec2::new(CALIBRATION_WIDTH, 0.0), column, |ui| {
                self.calibration_card(ui, s, snapshot, true);
            });
            let rest = ui.available_width();
            let controls = (rest - frame - gap - preview.x).clamp(260.0, CAMERA_CONTROLS_MAX);
            let card = controls + gap + preview.x + frame;
            ui.add_space(((rest - card) / 2.0 - gap).max(0.0));
            widgets::card(&pal).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(Vec2::new(controls, 0.0), column, |ui| {
                        ui.set_min_width(controls);
                        self.camera_controls(ui, s);
                        self.camera_chips(ui, s);
                    });
                    ui.allocate_ui_with_layout(preview, column, |ui| self.preview(ui, s, snapshot, STRIP_PREVIEW));
                });
            });
        });
    }

    pub(super) fn preview(&mut self, ui: &mut egui::Ui, s: &Settings, snapshot: &Snapshot, tallest: f32) {
        let pal = self.pal;
        let aspect = frame_aspect(snapshot);
        let width = ui.available_width();
        let height = (width / aspect).min(tallest);
        let (outer, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let painter = ui.painter_at(outer.expand(2.0));
        let radius = CornerRadius::same(widgets::RADIUS);
        painter.rect_filled(outer, radius, if pal.dark { Color32::from_gray(5) } else { Color32::from_gray(226) });
        let image_w = (height * aspect).min(width);
        let image = Rect::from_center_size(outer.center(), Vec2::new(image_w, image_w / aspect));

        if s.show_camera
            && let Some(texture) = &self.texture
        {
            egui::Image::new((texture.id(), image.size())).corner_radius(radius).paint_at(ui, image);
        }
        let to_screen =
            |p: &[f32; 3]| Pos2::new(image.left() + p[0] * image.width(), image.top() + p[1] * image.height());
        if s.show_landmarks {
            if s.tongue && snapshot.tongue == TongueStatus::Ready && snapshot.landmarks.len() > 291 {
                self.draw_tongue(&painter, snapshot, &to_screen);
            }
            let dot = if s.show_camera { pal.accent } else { pal.text };
            for p in &snapshot.landmarks {
                painter.circle_filled(to_screen(p), 0.9, dot);
            }
        }
        if s.cat_ears && snapshot.landmarks.len() > 400 {
            self.draw_ears(&painter, snapshot, &to_screen);
        }
        painter.rect_stroke(outer, radius, Stroke::new(1.0, pal.line), StrokeKind::Inside);
        self.reveal_on_hover(ui, &painter, &response, s, image);

        let status = match &snapshot.status {
            Status::Running => None,
            Status::Starting => Some(self.t(s, "starting").to_string()),
            Status::NoFace => Some(self.t(s, "no_face").to_string()),
            Status::CameraError(e) => Some(format!("{}: {e}", self.t(s, "camera_error"))),
            Status::ModelError(e) => Some(format!("{}: {e}", self.t(s, "model_error"))),
        };
        if let Some(status) = status {
            overlay(&painter, outer.left_top() + Vec2::new(8.0, 8.0), Align2::LEFT_TOP, status, 12.0, Color32::WHITE);
        }
        let banner = if let Some(progress) = snapshot.capture_progress {
            Some(format!("{} {}", self.t(s, "hold_still"), percent(progress)))
        } else {
            snapshot
                .message
                .filter(|(_, at)| at.elapsed() < Duration::from_secs(4))
                .map(|(key, _)| self.t(s, key).to_string())
        };
        if let Some(text) = banner {
            overlay(
                &painter,
                outer.center_bottom() - Vec2::new(0.0, 34.0),
                Align2::CENTER_BOTTOM,
                text,
                13.0,
                Color32::WHITE,
            );
        }
        let fps = format!(
            "{} {:.0} · {} {:.0} FPS",
            self.t(s, "tracking"),
            snapshot.tracking_fps,
            self.t(s, "camera"),
            snapshot.camera_fps
        );
        overlay(
            &painter,
            outer.left_bottom() + Vec2::new(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            fps,
            11.0,
            Color32::from_white_alpha(200),
        );
    }

    /// Uploads a new preview image; returns whether there was one.
    pub(super) fn update_texture(&mut self, ctx: &egui::Context, snapshot: &Snapshot) -> bool {
        let Some(frame) = &snapshot.preview else { return false };
        if snapshot.preview_seq == self.texture_seq {
            return false;
        }
        let started = Instant::now();
        self.texture_seq = snapshot.preview_seq;
        let image = ColorImage::from_rgb([frame.width, frame.height], &frame.data);
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::LINEAR),
            None => self.texture = Some(ctx.load_texture("preview", image, TextureOptions::LINEAR)),
        }
        self.shared.perf.lock().record(Stage::Upload, ms_since(started));
        true
    }

    /// Holding the pointer over a blurred preview for `REVEAL_HOLD` shows the camera
    /// unblurred for `REVEAL_FOR`; a ring around the pointer fills while it waits.
    fn reveal_on_hover(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        response: &egui::Response,
        s: &Settings,
        image: Rect,
    ) {
        let revealed = self.shared.unblurred_until.lock().is_some_and(|until| Instant::now() < until);
        let pointer = response.hover_pos().filter(|p| image.contains(*p));
        let (Some(pointer), true, true, false) = (pointer, s.show_camera, s.camera_blur, revealed) else {
            self.reveal_started = None;
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let started = *self.reveal_started.get_or_insert_with(Instant::now);
        let progress = (started.elapsed().as_secs_f32() / REVEAL_HOLD.as_secs_f32()).min(1.0);
        if progress >= 1.0 {
            *self.shared.unblurred_until.lock() = Some(Instant::now() + REVEAL_FOR);
            self.reveal_started = None;
            return;
        }
        let center = pointer + Vec2::new(0.0, -26.0);
        let radius = 18.0;
        painter.circle_stroke(center, radius, Stroke::new(3.0, Color32::from_white_alpha(40)));
        let steps = 48;
        let arc: Vec<Pos2> = (0..=(steps as f32 * progress) as usize)
            .map(|i| {
                let angle = -std::f32::consts::FRAC_PI_2 + i as f32 / steps as f32 * std::f32::consts::TAU;
                center + Vec2::angled(angle) * radius
            })
            .collect();
        painter.add(Shape::line(arc, Stroke::new(3.0, Color32::WHITE)));
        ui.ctx().request_repaint();
    }

    /// A tongue coming out over the lower lip, drawn in the mouth's own frame (the corner line,
    /// and across it the side of the chin) so it follows a tilted or mirrored face. Out sets its
    /// length; it leaves the mouth downwards and its centre line curves toward the tip, which left
    /// and right move sideways and up lifts over the upper lip. Roll narrows it and curls its
    /// edges in.
    fn draw_tongue(&self, painter: &egui::Painter, snapshot: &Snapshot, to_screen: &dyn Fn(&[f32; 3]) -> Pos2) {
        const STEPS: usize = 16;
        let lm = &snapshot.landmarks;
        let at = |i: usize| to_screen(&lm[i]);
        let first = channels::MEDIAPIPE.len();
        let v = |i: usize| snapshot.input[first + i].clamp(0.0, 1.0);
        let (out, left, right, up, down_value, roll) = (v(0), v(1), v(2), v(3), v(4), v(5));
        let corners = at(291) - at(61);
        let width = corners.length();
        if out < 0.06 || width < 4.0 {
            return;
        }
        // Toward the subject's left, and toward the chin.
        let across = corners / width;
        let mut down = across.rot90();
        if down.dot(at(152) - at(14)) < 0.0 {
            down = -down;
        }
        let root = at(13).lerp(at(14), 0.7);
        let length = width * 0.95 * out;
        // A quadratic curve: it starts along `down` (less of it the more the tongue lifts) and
        // ends at the tip.
        let control = root + down * (length * 0.45 * (1.0 - 0.8 * up));
        let reach = down * (length * (1.0 - 1.6 * up + 0.25 * down_value)) + across * (length * 1.1 * (left - right));
        // Pointing sideways turns the tongue, it does not make it longer.
        let tip = root + reach * (length / reach.length().max(length));
        let narrow = 1.0 - 0.45 * roll;
        // Centre point, half-width vector and direction at each step from the lip to the tip.
        let spine: Vec<(Pos2, Vec2, Vec2)> = (0..=STEPS)
            .map(|k| {
                let t = k as f32 / STEPS as f32;
                let s = 1.0 - t;
                let point = root + (control - root) * (2.0 * s * t) + (tip - root) * (t * t);
                let direction = ((control - root) * s + (tip - control) * t).normalized();
                let direction = if direction.is_finite() { direction } else { down };
                (point, direction.rot90() * (width * (0.21 - 0.07 * t) * narrow), direction)
            })
            .collect();
        let (end, end_side, end_direction) = spine[STEPS];
        let cap: Vec<Pos2> = (1..8)
            .map(|k| {
                let angle = std::f32::consts::PI * k as f32 / 8.0;
                end + end_side * angle.cos() + end_direction * (end_side.length() * angle.sin())
            })
            .collect();

        // The body as a triangle strip along the curve (it can bend, so not one convex polygon),
        // then a fan for the round tip.
        let pink = Color32::from_rgb(240, 118, 150);
        let edge = Color32::from_rgb(196, 62, 104);
        let mut mesh = egui::Mesh::default();
        for &(point, side, _) in &spine {
            mesh.colored_vertex(point + side, pink);
            mesh.colored_vertex(point - side, pink);
        }
        for k in 0..STEPS as u32 {
            let i = 2 * k;
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i + 1, i + 3, i + 2);
        }
        let centre = mesh.vertices.len() as u32;
        mesh.colored_vertex(end, pink);
        let mut previous = 2 * STEPS as u32;
        for &point in &cap {
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(point, pink);
            mesh.add_triangle(centre, previous, i);
            previous = i;
        }
        mesh.add_triangle(centre, previous, 2 * STEPS as u32 + 1);
        painter.add(Shape::mesh(mesh));

        // The outline, open at the lip, the groove down the middle and, rolled, the curled edges.
        let mut outline: Vec<Pos2> = spine.iter().map(|&(point, side, _)| point + side).collect();
        outline.extend(&cap);
        outline.extend(spine.iter().rev().map(|&(point, side, _)| point - side));
        painter.add(Shape::line(outline, Stroke::new(1.4, edge)));
        let inner = &spine[2..STEPS - 2];
        painter.add(Shape::line(
            inner.iter().map(|&(point, _, _)| point).collect(),
            Stroke::new(1.0, edge.gamma_multiply(0.6)),
        ));
        if roll > 0.2 {
            let curl = Stroke::new(1.2, edge.gamma_multiply(roll));
            for sign in [1.0, -1.0] {
                painter.add(Shape::line(
                    inner.iter().map(|&(point, side, _)| point + side * (0.55 * sign)).collect(),
                    curl,
                ));
            }
        }
    }

    /// Two triangles above the forehead whose tilt follows the ear outputs.
    fn draw_ears(&self, painter: &egui::Painter, snapshot: &Snapshot, to_screen: &dyn Fn(&[f32; 3]) -> Pos2) {
        let lm = &snapshot.landmarks;
        let (forehead, chin) = (to_screen(&lm[10]), to_screen(&lm[152]));
        let (right_temple, left_temple) = (to_screen(&lm[103]), to_screen(&lm[332]));
        let up = (forehead - chin).normalized();
        let size = (forehead - chin).length() * 0.32;
        let value = |name: &str| output_index(name).and_then(|i| snapshot.outputs.get(i)).copied().unwrap_or(0.0);
        for (base, ear, outward) in [(left_temple, value("EarLeft"), 1.0f32), (right_temple, value("EarRight"), -1.0)] {
            let side = Vec2::new(up.y, -up.x) * -outward;
            // Positive values stand the ear up, negative values fold it outward.
            let tilt = (-ear * 0.6).clamp(-0.6, 0.6) * outward;
            let (sin, cos) = tilt.sin_cos();
            let dir = Vec2::new(up.x * cos - up.y * sin, up.x * sin + up.y * cos);
            let base = base + up * size * 0.15;
            let tip = base + dir * size * (1.0 + ear.max(0.0) * 0.25);
            let a = base - side * size * 0.35;
            let b = base + side * size * 0.35;
            painter.add(Shape::convex_polygon(
                vec![a, tip, b],
                self.pal.accent.gamma_multiply(0.35),
                Stroke::new(1.5, self.pal.accent),
            ));
        }
    }
}
