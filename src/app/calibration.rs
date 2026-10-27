//! The calibration card: the neutral and gaze steps, the profiles and Reset all.

use std::sync::atomic::Ordering;

use eframe::egui::{self, RichText, Vec2};

use super::App;
use crate::config::Settings;
use crate::processing::{self, Profile};
use crate::tracker::{Command, Snapshot};
use crate::ui::icons::Glyph;
use crate::ui::widgets::{self, Button, Icon, Look};

impl App {
    pub(super) fn calibration_card(
        &mut self,
        ui: &mut egui::Ui,
        s: &mut Settings,
        snapshot: &Snapshot,
        profiles: bool,
    ) {
        let pal = self.pal;
        // A step shows a check once this avatar has it saved (an unknown avatar is "", as in the tracker).
        let (neutral_done, gaze_done) = {
            let store = self.shared.calibrations.lock();
            store.avatars.get(snapshot.avatar_id.as_deref().unwrap_or_default()).map_or((false, false), |c| {
                (c.channels.values().any(|channel| !channel.is_default()), c.gaze_center.is_some())
            })
        };
        widgets::card(&pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(widgets::title(&pal, self.t(s, "calibrate")));
            ui.add_space(2.0);
            self.calibration_step(
                ui,
                s,
                (Glyph::Scan, "capture_neutral", "step_neutral", "neutral_hint"),
                (neutral_done, Command::CaptureNeutral, Command::UndoNeutral, "neutral_undo"),
            );
            ui.add_space(4.0);
            self.calibration_step(
                ui,
                s,
                (Glyph::Target, "center_gaze", "step_gaze", "center_gaze_hint"),
                (gaze_done, Command::CenterGaze, Command::ResetGaze, "reset_gaze"),
            );
            if profiles {
                self.profiles_block(ui, s, snapshot.avatar_id.as_deref().unwrap_or_default());
            }
        });
    }

    /// Saved profiles: pick one to load it onto the current avatar, or type a name and save.
    fn profiles_block(&mut self, ui: &mut egui::Ui, s: &mut Settings, avatar: &str) {
        let pal = self.pal;
        let names: Vec<String> = self.shared.calibrations.lock().profiles.keys().cloned().collect();
        let active = self.profile_active.clone().filter(|n| names.contains(n));
        ui.add_space(10.0);
        ui.label(RichText::new(self.t(s, "profiles")).color(pal.dim).small()).on_hover_text(self.t(s, "profile_hint"));
        let button_width = 84.0;
        let (mut load, mut save) = (None, None);
        let (mut ask, mut confirm, mut cancel) = (false, false, false);
        ui.horizontal(|ui| {
            let gap = ui.spacing().item_spacing.x;
            let reserved = if active.is_some() { button_width + gap } else { 0.0 };
            let shown = active.clone().unwrap_or_else(|| self.t(s, "profile_none").to_string());
            egui::ComboBox::from_id_salt("profile")
                .width((ui.available_width() - reserved).max(80.0))
                .selected_text(shown)
                .truncate()
                .show_ui(ui, |ui| {
                    for name in &names {
                        if ui.selectable_label(active.as_ref() == Some(name), name).clicked() {
                            load = Some(name.clone());
                        }
                    }
                });
            if active.is_some() {
                if self.profile_confirm {
                    // Delete asks first, with a yes and a no where the button was.
                    (confirm, cancel) = self.confirm_buttons(ui, s);
                } else {
                    let spec = Button::new(self.t(s, "profile_delete")).width(button_width);
                    ask = widgets::button(ui, &pal, spec).clicked();
                }
            }
        });
        ui.horizontal(|ui| {
            let gap = ui.spacing().item_spacing.x;
            let hint = self.t(s, "profile_name").to_string();
            let edit = egui::TextEdit::singleline(&mut self.profile_name).hint_text(hint).char_limit(40);
            let width = (ui.available_width() - button_width - gap).max(60.0);
            let field = ui.add_sized(Vec2::new(width, 28.0), edit);
            let name = self.profile_name.trim().to_string();
            let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let text = self.t(s, "profile_save");
            let clicked = ui
                .add_enabled_ui(!name.is_empty(), |ui| {
                    widgets::button(ui, &pal, Button::new(text).look(Look::Primary).width(button_width)).clicked()
                })
                .inner;
            if (clicked || submitted) && !name.is_empty() {
                save = Some(name);
            }
        });
        if let Some(name) = load {
            self.load_profile(s, avatar, &name);
            self.profile_name = name.clone();
            self.profile_active = Some(name);
            self.profile_confirm = false;
        }
        if ask {
            self.profile_confirm = true;
        }
        if cancel {
            self.profile_confirm = false;
        }
        if confirm && let Some(name) = active {
            self.shared.calibrations.lock().profiles.remove(&name);
            self.shared.calibrations_dirty.store(true, Ordering::Relaxed);
            self.profile_active = None;
            self.profile_confirm = false;
        }
        if let Some(name) = save {
            self.save_profile(s, avatar, &name);
            self.profile_active = Some(name);
            self.profile_confirm = false;
        }
    }

    /// Keeps the avatar's calibration and the processing options under `name`.
    fn save_profile(&self, s: &Settings, avatar: &str, name: &str) {
        let mut store = self.shared.calibrations.lock();
        let mut calibration = store.avatars.get(avatar).cloned().unwrap_or_default();
        // The undo of the last neutral capture belongs to that moment, not to the profile.
        calibration.neutral_undo = None;
        store.profiles.insert(name.to_string(), Profile { calibration, processing: s.processing.clone() });
        self.shared.calibrations_dirty.store(true, Ordering::Relaxed);
    }

    /// Replaces the avatar's calibration and the processing options with those of `name`.
    fn load_profile(&self, s: &mut Settings, avatar: &str, name: &str) {
        let mut store = self.shared.calibrations.lock();
        let Some(profile) = store.profiles.get(name).cloned() else { return };
        store.avatars.insert(avatar.to_string(), profile.calibration);
        s.processing = profile.processing;
        self.shared.calibrations_dirty.store(true, Ordering::Relaxed);
    }

    /// One calibration step: the action, a line saying how, and the undo next to it.
    fn calibration_step(
        &self,
        ui: &mut egui::Ui,
        s: &Settings,
        text: (Glyph, &str, &str, &str),
        action: (bool, Command, Command, &str),
    ) {
        let pal = self.pal;
        let (glyph, label, caption, hint) = text;
        let (done, run, undo, undo_hint) = action;
        ui.horizontal(|ui| {
            let width = ui.available_width() - 32.0 - ui.spacing().item_spacing.x;
            let step = Button::new(self.t(s, label)).glyph(glyph).width(width).done(done);
            if widgets::button(ui, &pal, step).on_hover_text(self.t(s, hint)).clicked() {
                let _ = self.commands.send(run);
            }
            let revert = widgets::icon_button(ui, &pal, Icon::Draw(Glyph::Undo), false);
            if revert.on_hover_text(self.t(s, undo_hint)).clicked() {
                let _ = self.commands.send(undo);
            }
        });
        ui.label(RichText::new(self.t(s, caption)).small().color(pal.dim));
    }

    /// Sets every parameter back to its defaults. It asks first, with a yes and a no, so a
    /// slip does not lose the calibration of the avatar.
    pub(super) fn reset_all_button(&mut self, ui: &mut egui::Ui, s: &mut Settings, avatar: &str) {
        let pal = self.pal;
        if self.reset_confirm {
            ui.label(RichText::new(format!("{}?", self.t(s, "reset_all"))).color(pal.dim));
            let (yes, no) = self.confirm_buttons(ui, s);
            if no {
                self.reset_confirm = false;
            }
            if !yes {
                return;
            }
            self.reset_confirm = false;
            s.processing = processing::ProcessingSettings::default();
            if let Some(c) = self.shared.calibrations.lock().avatars.get_mut(avatar) {
                c.channels.clear();
                c.unlinked.clear();
            }
            self.shared.calibrations_dirty.store(true, Ordering::Relaxed);
            return;
        }
        let spec = Button::new(self.t(s, "reset_all")).glyph(Glyph::Undo).look(Look::Quiet);
        if widgets::button(ui, &pal, spec).on_hover_text(self.t(s, "reset_all_hint")).clicked() {
            self.reset_confirm = true;
        }
    }
}
