//! The settings window: connection, interface, camera, processing devices, updates and data.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Sense, Stroke, Vec2,
};

use super::preview::resolution_label;
use super::{App, CAMERA_RATES, RESOLUTIONS, UpdateState, percent};
use crate::config::{self, Device, IDLE_MINUTES, Settings};
use crate::perf::Stage;
use crate::tracker::{self, Command, Snapshot, TongueStatus};
use crate::ui::icons::Glyph;
use crate::ui::theme::mix;
use crate::ui::widgets::{self, Icon};
use crate::updater::{self, Channel, Release};

const SCALES: [f32; 6] = [0.75, 0.9, 1.0, 1.1, 1.25, 1.5];

fn mib(bytes: u64) -> String {
    format!("{:.3} MB", bytes as f64 / (1024.0 * 1024.0))
}

impl App {
    pub(super) fn settings_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        ui.horizontal(|ui| {
            ui.label(widgets::title(&pal, self.t(s, "settings")));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, &pal, Icon::Draw(Glyph::Close), false).clicked() {
                    self.settings_open = false;
                }
            });
        });
        let tallest = (ui.ctx().content_rect().height() - 150.0).max(200.0);
        egui::ScrollArea::vertical().id_salt("settings").max_height(tallest).min_scrolled_height(tallest).show(
            ui,
            |ui| {
                self.section(ui, self.t(s, "section_connection"));
                egui::Grid::new("ports").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                    ui.label(RichText::new(self.t(s, "vrchat_port")).color(pal.dim));
                    ui.add(egui::DragValue::new(&mut s.send_port).range(1..=65535));
                    ui.end_row();
                    ui.label(RichText::new(self.t(s, "in_port")).color(pal.dim));
                    ui.add(egui::DragValue::new(&mut s.receive_port).range(1..=65535));
                    ui.end_row();
                    ui.label(RichText::new(self.t(s, "osc_host")).color(pal.dim));
                    ui.add(egui::TextEdit::singleline(&mut s.osc_host).desired_width(110.0));
                    ui.end_row();
                });
                ui.add_space(6.0);
                let (interp, interp_hint) = (self.t(s, "interpolation"), self.t(s, "interpolation_hint"));
                let (no_avatar, no_avatar_hint) =
                    (self.t(s, "send_without_avatar"), self.t(s, "send_without_avatar_hint"));
                self.labeled_switch(ui, &mut s.interpolation, interp, Some(interp_hint));
                self.labeled_switch(ui, &mut s.send_without_avatar, no_avatar, Some(no_avatar_hint));

                self.section(ui, self.t(s, "section_interface"));
                egui::Grid::new("interface").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                    ui.label(RichText::new(self.t(s, "interface_scale")).color(pal.dim));
                    egui::ComboBox::from_id_salt("scale").selected_text(percent(s.interface_scale)).show_ui(ui, |ui| {
                        for scale in SCALES {
                            ui.selectable_value(&mut s.interface_scale, scale, percent(scale));
                        }
                    });
                    ui.end_row();
                    ui.label(RichText::new(self.t(s, "idle_sleep")).color(pal.dim))
                        .on_hover_text(self.t(s, "idle_hint"));
                    let current = if s.idle_sleep {
                        format!("{} {}", s.idle_minutes, self.t(s, "min"))
                    } else {
                        self.t(s, "never").into()
                    };
                    egui::ComboBox::from_id_salt("idle").selected_text(current).show_ui(ui, |ui| {
                        if ui.selectable_label(!s.idle_sleep, self.t(s, "never")).clicked() {
                            s.idle_sleep = false;
                        }
                        for m in IDLE_MINUTES {
                            let label = format!("{m} {}", self.t(s, "min"));
                            if ui.selectable_label(s.idle_sleep && s.idle_minutes == m, label).clicked() {
                                s.idle_sleep = true;
                                s.idle_minutes = m;
                            }
                        }
                    });
                    ui.end_row();
                });
                ui.add_space(6.0);
                let friendly = self.t(s, "friendly_names");
                let (to_tray, to_tray_hint) = (self.t(s, "close_to_tray"), self.t(s, "close_to_tray_hint"));
                let debug = self.t(s, "debug_performance");
                self.labeled_switch(ui, &mut s.friendly_names, friendly, None);
                self.labeled_switch(ui, &mut s.close_to_tray, to_tray, Some(to_tray_hint));
                self.labeled_switch(ui, &mut s.debug, debug, None);

                self.section(ui, self.t(s, "camera"));
                egui::Grid::new("camera-settings").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                    ui.label(RichText::new(self.t(s, "resolution")).color(pal.dim));
                    let current = resolution_label(s.resolution);
                    egui::ComboBox::from_id_salt("resolution").selected_text(current).show_ui(ui, |ui| {
                        for r in RESOLUTIONS {
                            ui.selectable_value(&mut s.resolution, r, resolution_label(r));
                        }
                    });
                    ui.end_row();
                    ui.label(RichText::new(self.t(s, "camera_fps")).color(pal.dim));
                    egui::ComboBox::from_id_salt("fps").selected_text(s.camera_fps.to_string()).show_ui(ui, |ui| {
                        for fps in CAMERA_RATES {
                            ui.selectable_value(&mut s.camera_fps, fps, fps.to_string());
                        }
                    });
                    ui.end_row();
                });
                ui.add_space(6.0);
                let blur = self.t(s, "blur_preview");
                self.labeled_switch(ui, &mut s.camera_blur, blur, None);

                self.section(ui, self.t(s, "section_processing"));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(self.t(s, "processing_devices")).color(pal.dim))
                        .on_hover_text(self.t(s, "devices_hint"));
                    let mut all = if s.face_device == s.tongue_device { s.face_device } else { Device::Auto };
                    let before = all;
                    self.device_picker(ui, s, "all-devices", &mut all, self.t(s, "all"), false);
                    if all != before {
                        s.face_device = all;
                        s.tongue_device = all;
                    }
                });
                ui.add_space(8.0);
                if cfg!(not(feature = "directml")) {
                    ui.label(RichText::new(self.t(s, "directml_missing")).color(pal.dim).small());
                }
                let perf = self.shared.perf.lock().clone();
                let avg = |stage: Stage| perf.stages.get(&stage).and_then(|st| st.average(Duration::from_secs(1)));
                let (capture, capture_hint) = (self.t(s, "camera_capture"), self.t(s, "capture_hint"));
                let camera_ms = avg(Stage::ReadWait).map(|read| read + avg(Stage::Convert).unwrap_or(0.0));
                self.stage_box(ui, capture, camera_ms, Some(capture_hint), |ui| {
                    ui.label(RichText::new("Media Foundation").color(pal.dim));
                });
                self.stage_box(ui, "MediaPipe", avg(Stage::MediaPipe), Some(self.t(s, "mediapipe_hint")), |ui| {
                    let mut d = s.face_device;
                    self.device_picker(ui, s, "face-device", &mut d, "", true);
                    s.face_device = d;
                    if !snapshot.face_device.is_empty() && snapshot.face_device != self.device_name(s, d) {
                        ui.label(RichText::new(&snapshot.face_device).color(pal.dim).small());
                    }
                });
                let tongue_ms = avg(Stage::Tongue).filter(|_| snapshot.tongue == TongueStatus::Ready);
                let tongue = self.t(s, "tongue");
                self.stage_box(ui, tongue, tongue_ms, Some(self.t(s, "tongue_stage_hint")), |ui| {
                    match &snapshot.tongue {
                        TongueStatus::Ready => {
                            let mut d = s.tongue_device;
                            self.device_picker(ui, s, "tongue-device", &mut d, "", true);
                            s.tongue_device = d;
                        }
                        TongueStatus::Off => {
                            ui.label(RichText::new(self.t(s, "enable_tongue")).color(pal.dim));
                        }
                        TongueStatus::Missing => {
                            ui.label(RichText::new(self.t(s, "tongue_missing")).color(pal.dim));
                        }
                        TongueStatus::Error(e) => {
                            ui.label(RichText::new(e).color(pal.dim));
                        }
                    }
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(self.t(s, "import_tongue")).clicked() {
                            self.import_tongue();
                        }
                        if tracker::tongue_path(&self.model_dir).is_some() {
                            self.ok_label(ui, self.t(s, "tongue_ready"));
                        }
                    });
                });
                self.section(ui, self.t(s, "updates"));
                self.updates_section(ui, s);

                self.section(ui, self.t(s, "section_data"));
                if self.data_size.1.elapsed() > Duration::from_secs(10) {
                    self.data_size = (config::dir_size(&config::data_dir()), Instant::now());
                }
                let armed = self.erase_armed.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
                let text = if armed {
                    self.t(s, "erase_confirm").to_string()
                } else {
                    format!("{} ({})", self.t(s, "erase"), mib(self.data_size.0))
                };
                let button = egui::Button::new(text).min_size(Vec2::new(ui.available_width(), 28.0));
                if ui.add(button).on_hover_text(self.t(s, "erase_hint")).clicked() {
                    if armed {
                        if let Err(e) = config::erase_all() {
                            log::warn!("erasing app data: {e:#}");
                        }
                        // Restart without saving so nothing is written back.
                        let _ = updater::restart();
                    } else {
                        self.erase_armed = Some(Instant::now());
                    }
                }

                ui.add_space(14.0);
                ui.label(RichText::new(concat!("VRC Openface v", env!("CARGO_PKG_VERSION"))).color(pal.dim));
            },
        );
    }

    pub(super) fn import_tongue(&self) {
        let Some(path) = rfd::FileDialog::new().add_filter("ONNX model", &["onnx"]).pick_file() else { return };
        let dir = config::user_model_dir();
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&path, dir.join(tracker::TONGUE_FILE))) {
            Ok(_) => {
                let _ = self.commands.send(Command::ReloadModels);
            }
            Err(e) => log::warn!("importing tongue model: {e}"),
        }
    }

    fn device_name(&self, s: &Settings, device: Device) -> String {
        match device {
            Device::Auto => self.t(s, "auto").to_string(),
            Device::Cpu => self.t(s, "cpu").to_string(),
            Device::Gpu(i) => crate::system::gpu_label(&self.shared.gpus, i),
        }
    }

    /// Device choice. `plain` draws it as text with a ▾, for use inside a stage box.
    fn device_picker(&self, ui: &mut egui::Ui, s: &Settings, id: &str, device: &mut Device, prefix: &str, plain: bool) {
        let width = ui.available_width() - 4.0;
        ui.scope(|ui| {
            if plain {
                let w = &mut ui.visuals_mut().widgets;
                for state in [&mut w.inactive, &mut w.active] {
                    state.bg_stroke = Stroke::NONE;
                    state.weak_bg_fill = Color32::TRANSPARENT;
                }
                // Outlined and tinted on hover so it reads as something to open.
                for state in [&mut w.hovered, &mut w.open] {
                    state.bg_stroke = Stroke::new(1.0, self.pal.accent);
                    state.weak_bg_fill = mix(self.pal.raised, self.pal.accent, 0.22);
                    state.fg_stroke.color = self.pal.text;
                }
                w.inactive.fg_stroke.color = self.pal.dim;
            }
            egui::ComboBox::from_id_salt(id)
                .width(width.max(80.0))
                .selected_text(format!("{prefix} {}", self.device_name(s, *device)).trim_start().to_string())
                .truncate()
                .show_ui(ui, |ui| {
                    let mut options = vec![Device::Auto, Device::Cpu];
                    options.extend(self.shared.gpus.iter().map(|g| Device::Gpu(g.index)));
                    for option in options {
                        ui.selectable_value(device, option, self.device_name(s, option));
                    }
                });
        });
    }

    /// One processing stage: title (with an optional help mark) and time on the first
    /// line, then `body` (the device) below, in a roomy outlined box.
    fn stage_box(
        &self,
        ui: &mut egui::Ui,
        title: &str,
        ms: Option<f32>,
        hint: Option<&str>,
        body: impl FnOnce(&mut egui::Ui),
    ) {
        let pal = self.pal;
        Frame::new()
            .fill(pal.raised)
            .stroke(Stroke::new(1.0, pal.line))
            .corner_radius(CornerRadius::same(widgets::RADIUS + 2))
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).font(FontId::new(13.5, widgets::semibold())));
                    if let Some(hint) = hint {
                        let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                        let color = if response.hovered() { pal.text } else { pal.dim };
                        ui.painter().circle_stroke(rect.center(), 7.0, Stroke::new(1.0, color));
                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "?", FontId::proportional(10.0), color);
                        response.on_hover_text(hint);
                    }
                    if let Some(ms) = ms {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(format!("{ms:.2} ms"));
                        });
                    }
                });
                ui.add_space(4.0);
                body(ui);
            });
        ui.add_space(10.0);
    }

    fn updates_section(&mut self, ui: &mut egui::Ui, s: &mut Settings) {
        ui.horizontal(|ui| {
            ui.label(self.t(s, "channel"));
            for (channel, key) in [(Channel::Stable, "stable"), (Channel::Beta, "beta")] {
                let mut on = s.update_channel == channel;
                if self.toggle(ui, &mut on, self.t(s, key)).clicked() {
                    s.update_channel = channel;
                }
            }
        });
        let state = std::mem::take(&mut *self.updates.lock());
        let next = match state {
            UpdateState::Idle => {
                if ui.button(self.t(s, "check_updates")).clicked() {
                    self.spawn_check(ui.ctx().clone());
                    UpdateState::Checking
                } else {
                    UpdateState::Idle
                }
            }
            UpdateState::Checking => {
                ui.label(self.t(s, "checking"));
                UpdateState::Checking
            }
            UpdateState::Downloading => {
                ui.label(self.t(s, "downloading"));
                UpdateState::Downloading
            }
            UpdateState::Failed(e) => {
                ui.label(RichText::new(format!("{}: {e}", self.t(s, "update_failed"))).color(self.pal.dim).small());
                if ui.button(self.t(s, "check_updates")).clicked() {
                    self.spawn_check(ui.ctx().clone());
                    UpdateState::Checking
                } else {
                    UpdateState::Failed(e)
                }
            }
            UpdateState::Ready(releases) => {
                let install = self.release_picker(ui, s, &releases);
                if let Some(release) = install {
                    self.spawn_install(release);
                    UpdateState::Downloading
                } else {
                    UpdateState::Ready(releases)
                }
            }
        };
        // A background check may have finished while we drew; its result wins.
        let mut guard = self.updates.lock();
        if matches!(*guard, UpdateState::Idle) {
            *guard = next;
        }
    }

    /// Version list for the selected channel; returns a release to install.
    fn release_picker(&mut self, ui: &mut egui::Ui, s: &Settings, releases: &[Release]) -> Option<Release> {
        let candidates: Vec<&Release> =
            releases.iter().filter(|r| s.update_channel == Channel::Beta || !r.prerelease).collect();
        if candidates.is_empty() {
            ui.label(RichText::new(self.t(s, "no_releases")).color(self.pal.dim));
            return None;
        }
        let latest = updater::latest(releases, s.update_channel).map(|r| r.tag.clone());
        let selected = self
            .selected_release
            .clone()
            .filter(|t| candidates.iter().any(|r| &r.tag == t))
            .or(latest.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(self.t(s, "version"));
            let shown = if Some(&selected) == latest.as_ref() {
                format!("{} · {selected}", self.t(s, "latest"))
            } else {
                selected.clone()
            };
            egui::ComboBox::from_id_salt("release").selected_text(shown).show_ui(ui, |ui| {
                for r in &candidates {
                    if ui.selectable_label(r.tag == selected, &r.tag).clicked() {
                        self.selected_release = Some(r.tag.clone());
                    }
                }
            });
        });
        let release = candidates.iter().find(|r| r.tag == selected)?;
        let mut install = None;
        ui.horizontal(|ui| {
            if release.version() == env!("CARGO_PKG_VERSION") {
                self.ok_label(ui, self.t(s, "up_to_date"));
            } else {
                let newer = updater::is_newer(release.version(), env!("CARGO_PKG_VERSION"));
                let key = if newer { "update_now" } else { "install_restart" };
                if ui.add_enabled(release.zip_url.is_some(), egui::Button::new(self.t(s, key))).clicked() {
                    install = Some((*release).clone());
                }
            }
            if ui.link(self.t(s, "releases")).clicked() {
                crate::system::open_url(&release.page);
            }
        });
        install
    }

    fn spawn_check(&self, ctx: egui::Context) {
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            *updates.lock() = match updater::check() {
                Ok(releases) => UpdateState::Ready(releases),
                Err(e) => UpdateState::Failed(format!("{e:#}")),
            };
            ctx.request_repaint();
        });
    }

    fn spawn_install(&self, release: Release) {
        let updates = self.updates.clone();
        let settings = self.shared.settings.read().clone();
        let store = self.shared.calibrations.lock().clone();
        std::thread::spawn(move || {
            let _ = settings.save();
            let _ = config::save_calibrations(&store);
            if let Err(e) = updater::install(&release) {
                *updates.lock() = UpdateState::Failed(format!("{e:#}"));
            }
        });
    }
}
