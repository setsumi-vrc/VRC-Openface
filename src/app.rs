use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use eframe::egui::{
    self, Align, Align2, Color32, ColorImage, CornerRadius, FontFamily, FontId, Frame, Layout, Margin, Pos2, Rect,
    RichText, Sense, Shape, Stroke, StrokeKind, TextStyle, TextureHandle, TextureOptions, Vec2,
};
use parking_lot::Mutex;

use crate::camera::{self, CameraEntry};
use crate::channels::{self, COUNT};
use crate::config::{self, Device, IDLE_MINUTES, Settings};
use crate::i18n::{Language, tr};
use crate::icons::{self, Glyph};
use crate::mapping::{Mapper, OUTPUTS};
use crate::osc::Binding;
use crate::perf::{self, Group, Path, Segment, Stage};
use crate::processing::{self, AvatarCalibration, ChannelCalibration, Profile, Source};
use crate::theme::{CustomColors, Palette, Scheme, mix};
use crate::tracker::{self, Command, Shared, Snapshot, Status, TongueStatus};
use crate::updater::{self, Channel, Release};
use crate::widgets::{self, Button, Icon, Look};

const RESOLUTIONS: [(u32, u32); 4] = [(640, 480), (848, 480), (1280, 720), (1920, 1080)];
const SCALES: [f32; 6] = [0.75, 0.9, 1.0, 1.1, 1.25, 1.5];
const MAPPING_DOCS: &str = "https://docs.vrcft.io/docs/tutorial-avatars/tutorial-avatars-extras/parameters";
const ROW_HEIGHT: f32 = 30.0;
const SIDEBAR_WIDTH: f32 = 460.0;
/// Height of the preview in the camera strip across the top of the Advanced view.
const STRIP_PREVIEW: f32 = 230.0;
/// Width of the calibration card in the strip.
const CALIBRATION_WIDTH: f32 = 400.0;
/// The most the camera controls in the strip grow to.
const CAMERA_CONTROLS_MAX: f32 = 360.0;
/// Smallest window the layout holds together in.
pub const MIN_WINDOW: Vec2 = Vec2::new(1220.0, 660.0);
/// Width of the strip along the window edge that starts a resize (the window has no frame).
const RESIZE_BAND: f32 = 5.0;
/// How long a row stays hovered before the rows linked to it scroll into view.
const REVEAL_AFTER: Duration = Duration::from_millis(250);
const X_PROFILE: &str = "https://x.com/setsumi_vrc";
/// How long the pointer rests on a blurred preview before it is shown unblurred, and
/// for how long.
const REVEAL_HOLD: Duration = Duration::from_millis(1500);
const REVEAL_FOR: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
enum UpdateState {
    #[default]
    Idle,
    Checking,
    Ready(Vec<Release>),
    Downloading,
    Failed(String),
}

/// The row under the pointer in one of the three parameter panels.
#[derive(Clone, Copy, PartialEq)]
enum Hover {
    Channel(usize),
    Output(usize),
    Avatar(usize),
}

/// Where this frame's rows were drawn, for the links between the panels.
struct Rows {
    channels: Vec<(usize, Rect)>,
    outputs: Vec<(usize, Rect)>,
    avatar: Vec<(usize, Rect)>,
    /// Scrolled area of the MediaPipe, Unified Expressions and Avatar panels.
    view: [Rect; 3],
    /// Outer frame of the same panels.
    frame: [Rect; 3],
}

impl Default for Rows {
    fn default() -> Self {
        Self {
            channels: Vec::new(),
            outputs: Vec::new(),
            avatar: Vec::new(),
            view: [Rect::NOTHING; 3],
            frame: [Rect::NOTHING; 3],
        }
    }
}

impl Rows {
    /// Vertical centre of a row, kept inside its panel so rows scrolled out of view
    /// still get a link pointing at the edge.
    fn y(list: &[(usize, Rect)], index: usize, view: Rect) -> Option<f32> {
        list.iter().find(|(i, _)| *i == index).map(|(_, r)| r.center().y.min(view.bottom()).max(view.top()))
    }

    /// Which way a row lies when it is scrolled out of its panel: -1 above, 1 below, 0 in view.
    fn off(list: &[(usize, Rect)], index: usize, view: Rect) -> f32 {
        match list.iter().find(|(i, _)| *i == index).map(|(_, r)| r.center().y) {
            Some(y) if y > view.bottom() => 1.0,
            Some(y) if y < view.top() => -1.0,
            _ => 0.0,
        }
    }
}

/// Rows connected to the hovered one, highlighted in every panel.
struct Related {
    channels: [bool; COUNT],
    outputs: Vec<bool>,
}

pub struct App {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    model_dir: PathBuf,
    cameras: Vec<CameraEntry>,
    texture: Option<TextureHandle>,
    /// App icon shown next to the title.
    mark: Option<TextureHandle>,
    /// The restored window size was checked against `MIN_WINDOW`.
    size_checked: bool,
    /// GitHub and X marks for the links in the settings.
    links: Vec<(TextureHandle, &'static str)>,
    /// Flag per language code, for the language menu.
    flags: Vec<(&'static str, TextureHandle)>,
    texture_seq: u64,
    search: String,
    settings_open: bool,
    expanded: Option<usize>,
    /// The profile last loaded or saved, and the name typed for the next save.
    /// The MediaPipe list's filters: a group of [`channels::CATEGORIES`] (0 is all) and only
    /// the parameters changed from their defaults.
    filter_group: usize,
    filter_modified: bool,
    profile_active: Option<String>,
    profile_name: String,
    /// Delete was pressed and waits for a yes or a no.
    profile_confirm: bool,
    /// The same for Reset all.
    reset_confirm: bool,
    /// What "modified" is measured against: the saved profile in use, or the defaults.
    baseline: Profile,
    expanded_bindings: BTreeSet<usize>,
    mapper: Mapper,
    pal: Palette,
    applied_style: Option<(Scheme, CustomColors, f32)>,
    saved: Settings,
    last_save: Instant,
    last_activity: Instant,
    updates: Arc<Mutex<UpdateState>>,
    selected_release: Option<String>,
    erase_armed: Option<Instant>,
    data_size: (u64, Instant),
    rows: Rows,
    /// Hover from the previous frame, which decides this frame's highlights.
    hover: Option<Hover>,
    hover_now: Option<Hover>,
    /// Row positions from the previous frame.
    prev_rows: Rows,
    /// While a channel stays hovered: where its expressions were moved to in the
    /// Unified Expressions list (`None` when they were all visible and stay in place).
    /// The hovered row and since when, so linked rows only scroll into view on a pause.
    dwell: (Option<Hover>, Instant),
    /// Per panel, a row was already scrolled into view for the hovered row.
    revealed: [bool; 3],
    /// When the pointer started resting on the blurred preview.
    reveal_started: Option<Instant>,
    /// Narrow layout: Unified Expressions above Avatar instead of side by side.
    narrow: bool,
    /// Last tracker frame counted in the interface latencies.
    seen_frame: u64,
    /// Language the CJK font order was built for.
    font_language: Language,
    /// Tray icon; closing the window hides it there instead of quitting.
    tray: Option<crate::tray::Tray>,
}

/// Flag images by language code (drawn by `tools/make_flags.py`).
const FLAGS: [(&str, &[u8]); 10] = [
    ("en", include_bytes!("../assets/flags/en.png")),
    ("es", include_bytes!("../assets/flags/es.png")),
    ("pt", include_bytes!("../assets/flags/pt.png")),
    ("fr", include_bytes!("../assets/flags/fr.png")),
    ("de", include_bytes!("../assets/flags/de.png")),
    ("it", include_bytes!("../assets/flags/it.png")),
    ("ru", include_bytes!("../assets/flags/ru.png")),
    ("zh", include_bytes!("../assets/flags/zh.png")),
    ("ja", include_bytes!("../assets/flags/ja.png")),
    ("ko", include_bytes!("../assets/flags/ko.png")),
];

fn load_png(ctx: &egui::Context, name: &str, png: &[u8]) -> Option<TextureHandle> {
    let icon = eframe::icon_data::from_png_bytes(png).map_err(|e| log::warn!("{name} icon: {e}")).ok()?;
    let image = ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
}

/// Loads the interface fonts: Segoe UI (regular and semibold, found on every Windows) for the
/// Latin and Cyrillic text, and Windows' CJK fonts as fallbacks so every language renders. For
/// Japanese, Korean and Chinese the CJK font of that language goes first: Han glyphs differ and
/// sit on different baselines between fonts, so mixing them within one line looks broken.
fn load_fonts(ctx: &egui::Context, language: Language) {
    let fonts = std::env::var_os("WINDIR").map(|w| PathBuf::from(w).join("Fonts")).unwrap_or_default();
    let load = |files: &[&str]| files.iter().find_map(|f| std::fs::read(fonts.join(f)).ok());
    let preferred = match language {
        Language::Ja => "japanese",
        Language::Ko => "korean",
        Language::Zh => "chinese",
        _ => "",
    };
    let mut candidates = [
        ("japanese", load(&["YuGothM.ttc", "msgothic.ttc"])),
        ("korean", load(&["malgun.ttf"])),
        ("chinese", load(&["msyh.ttc"])),
    ];
    candidates.sort_by_key(|(name, _)| *name != preferred);
    let mut definitions = egui::FontDefinitions::default();
    let defaults = |family: &FontFamily, definitions: &egui::FontDefinitions| {
        definitions.families.get(family).cloned().unwrap_or_default()
    };
    let (default_proportional, default_monospace) =
        (defaults(&FontFamily::Proportional, &definitions), defaults(&FontFamily::Monospace, &definitions));

    let mut cjk = Vec::new();
    for (name, bytes) in candidates {
        let Some(bytes) = bytes else { continue };
        definitions.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
        cjk.push(name.to_string());
    }
    let mut ui_fonts = Vec::new();
    for (name, files) in [("ui", ["segoeui.ttf"]), ("ui-semibold", ["seguisb.ttf"])] {
        if let Some(bytes) = load(&files) {
            definitions.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
            ui_fonts.push(name);
        }
    }
    let first_cjk = cjk.first().filter(|name| name.as_str() == preferred).cloned();
    let build = |ui_font: Option<&str>, base: &[String]| {
        let mut list: Vec<String> = Vec::new();
        list.extend(first_cjk.clone());
        list.extend(ui_font.filter(|n| ui_fonts.contains(n)).map(String::from));
        list.extend(base.iter().cloned());
        for name in &cjk {
            if !list.contains(name) {
                list.push(name.clone());
            }
        }
        list
    };
    definitions.families.insert(FontFamily::Proportional, build(Some("ui"), &default_proportional));
    definitions.families.insert(FontFamily::Monospace, build(None, &default_monospace));
    definitions.families.insert(widgets::semibold(), build(Some("ui-semibold"), &default_proportional));
    // One family per CJK font, for text that must keep a single font: a language's own name,
    // for example, since the fallback chain would take part of it from another font.
    for name in ["japanese", "korean", "chinese"] {
        let mut list = vec![name.to_string()];
        list.extend(build(Some("ui"), &default_proportional).into_iter().filter(|n| n != name));
        list.retain(|n| definitions.font_data.contains_key(n) || n == name && cjk.contains(&name.to_string()));
        definitions.families.insert(FontFamily::Name(name.into()), list);
    }
    ctx.set_fonts(definitions);
}

/// Font family for a language's own name: the CJK languages use their own font throughout.
fn native_font(language: Language) -> FontFamily {
    match language {
        Language::Ja => FontFamily::Name("japanese".into()),
        Language::Ko => FontFamily::Name("korean".into()),
        Language::Zh => FontFamily::Name("chinese".into()),
        _ => FontFamily::Proportional,
    }
}

fn apply_style(ctx: &egui::Context, pal: &Palette) {
    ctx.global_style_mut(|style| {
        for (text_style, font) in [
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(16.0, widgets::semibold())),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ] {
            style.text_styles.insert(text_style, font);
        }
        // Labels are text on a control surface, not something to select and copy.
        style.interaction.selectable_labels = false;
        let mut v = if pal.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = pal.bg;
        v.window_fill = pal.panel;
        v.extreme_bg_color = pal.bg;
        v.faint_bg_color = pal.panel;
        v.override_text_color = Some(pal.text);
        v.selection.bg_fill = pal.accent.gamma_multiply(0.45);
        v.selection.stroke = Stroke::new(1.0, pal.text);
        v.window_stroke = Stroke::new(1.0, pal.line);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        v.hyperlink_color = pal.accent;
        let radius = CornerRadius::same(widgets::RADIUS);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = radius;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.line);
        v.widgets.inactive.bg_fill = pal.raised;
        v.widgets.inactive.weak_bg_fill = pal.raised;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, pal.line);
        v.widgets.hovered.bg_fill = mix(pal.raised, pal.accent, 0.22);
        v.widgets.hovered.weak_bg_fill = mix(pal.raised, pal.accent, 0.22);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, pal.accent);
        // Scroll handles take these: quiet at rest, accent when grabbed.
        v.widgets.inactive.fg_stroke.color = pal.dim;
        v.widgets.hovered.fg_stroke.color = pal.accent;
        v.widgets.active.fg_stroke.color = pal.accent;
        v.widgets.active.bg_fill = pal.accent;
        v.widgets.active.weak_bg_fill = pal.accent;
        v.widgets.open.bg_fill = pal.hover();
        v.widgets.open.weak_bg_fill = pal.hover();
        style.visuals = v;
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.interact_size.y = 28.0;
        style.spacing.slider_width = 140.0;
        // Scroll bars keep their own strip, so they never sit on top of the content.
        style.spacing.scroll = egui::style::ScrollStyle {
            floating_allocated_width: 10.0,
            foreground_color: true,
            ..egui::style::ScrollStyle::thin()
        };
    });
}

/// Text on top of the camera image, on a dark chip so it stays readable on any frame.
fn overlay(painter: &egui::Painter, anchor: Pos2, align: Align2, text: String, size: f32, fg: Color32) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(size), Color32::PLACEHOLDER);
    let rect = align.anchor_size(anchor, galley.size() + Vec2::new(16.0, 8.0));
    painter.rect_filled(rect, CornerRadius::same(widgets::RADIUS), Color32::from_black_alpha(150));
    painter.galley(rect.min + Vec2::new(8.0, 4.0), galley, fg);
}

fn percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

/// Panel header: the title on the left, truncated to the room the controls on the right
/// leave. When that room gets too small the controls move to a second line.
fn header(ui: &mut egui::Ui, id: &str, title: impl FnOnce(&mut egui::Ui), controls: impl FnOnce(&mut egui::Ui)) {
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
fn truncated(painter: &egui::Painter, text: String, font: FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    painter.layout_job(job)
}

fn ms_since(started: Instant) -> f32 {
    started.elapsed().as_secs_f32() * 1000.0
}

fn mib(bytes: u64) -> String {
    format!("{:.3} MB", bytes as f64 / (1024.0 * 1024.0))
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: Arc<Shared>,
        commands: Sender<Command>,
        model_dir: PathBuf,
    ) -> Self {
        let saved = shared.settings.read().clone();
        let tray = crate::tray::Tray::new(cc, saved.language, shared.clone());
        let language = saved.language;
        load_fonts(&cc.egui_ctx, language);
        let pal = Palette::of(saved.scheme, &saved.custom_colors);
        Self {
            shared,
            commands,
            model_dir,
            cameras: camera::list(),
            texture: None,
            mark: load_png(&cc.egui_ctx, "mark", include_bytes!("../assets/icon.png")),
            size_checked: false,
            links: [
                ("github", include_bytes!("../assets/github.png").as_slice(), env!("CARGO_PKG_REPOSITORY")),
                ("x", include_bytes!("../assets/x.png").as_slice(), X_PROFILE),
            ]
            .into_iter()
            .filter_map(|(name, png, url)| Some((load_png(&cc.egui_ctx, name, png)?, url)))
            .collect(),
            flags: FLAGS.iter().filter_map(|(code, png)| Some((*code, load_png(&cc.egui_ctx, code, png)?))).collect(),
            texture_seq: 0,
            search: String::new(),
            settings_open: false,
            expanded: None,
            filter_group: 0,
            filter_modified: false,
            profile_active: None,
            profile_name: String::new(),
            profile_confirm: false,
            reset_confirm: false,
            baseline: Profile::default(),
            expanded_bindings: BTreeSet::new(),
            mapper: Mapper::new(),
            pal,
            applied_style: None,
            saved,
            last_save: Instant::now(),
            last_activity: Instant::now(),
            updates: Arc::default(),
            selected_release: None,
            erase_armed: None,
            data_size: (config::dir_size(&config::data_dir()), Instant::now()),
            rows: Rows::default(),
            hover: None,
            hover_now: None,
            prev_rows: Rows::default(),
            dwell: (None, Instant::now()),
            revealed: [false; 3],
            reveal_started: None,
            narrow: false,
            seen_frame: 0,
            font_language: language,
            tray,
        }
    }

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

    fn t(&self, s: &Settings, key: &str) -> &'static str {
        tr(s.language, key)
    }

    fn matches(&self, text: &str) -> bool {
        self.search.is_empty() || text.to_lowercase().contains(&self.search.to_lowercase())
    }

    /// Filled with the accent colour when on, like a pressed key.
    fn toggle(&self, ui: &mut egui::Ui, on: &mut bool, text: &str) -> egui::Response {
        let (fg, bg) = if *on { (self.pal.on_accent, self.pal.accent) } else { (self.pal.text, self.pal.bg) };
        let response = ui.add(egui::Button::new(RichText::new(text).color(fg)).fill(bg));
        if response.clicked() {
            *on = !*on;
        }
        response
    }

    fn flag(&self, language: Language) -> Option<egui::Image<'static>> {
        let (_, texture) = self.flags.iter().find(|(code, _)| *code == language.code())?;
        Some(egui::Image::new((texture.id(), Vec2::new(24.0, 16.0))))
    }

    /// Current language with its flag; opens the list of languages.
    fn language_menu(&self, ui: &mut egui::Ui, s: &mut Settings) {
        let pal = self.pal;
        let name = s.language.native_name();
        let font = FontId::new(12.5, native_font(s.language));
        let text_width = ui.fonts_mut(|f| f.layout_no_wrap(name.to_string(), font.clone(), pal.text).size().x);
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

    /// Square button showing an image or a glyph: outlined on hover, filled when `on`.
    /// A labelled box that works as a slider: click or drag across it to set `value`
    /// (0..1). The filled part shows the value; 0 reads as `zero_text`.
    fn slider_box(
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
    fn switch(&self, ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
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

    fn labeled_switch(&self, ui: &mut egui::Ui, on: &mut bool, text: &str, hint: Option<&str>) {
        ui.horizontal(|ui| {
            let label = ui.label(text);
            if let Some(hint) = hint {
                label.on_hover_text(hint);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| self.switch(ui, on));
        });
    }

    fn panel_frame(&self) -> Frame {
        Frame::new()
            .fill(self.pal.panel)
            .stroke(Stroke::new(1.0, self.pal.line.gamma_multiply(0.8)))
            .corner_radius(CornerRadius::same(widgets::RADIUS + 4))
            .inner_margin(Margin::symmetric(8, 10))
    }

    /// Uploads a new preview image; returns whether there was one.
    fn update_texture(&mut self, ctx: &egui::Context, snapshot: &Snapshot) -> bool {
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

    fn persist(&mut self) {
        if self.last_save.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_save = Instant::now();
        let current = self.shared.settings.read().clone();
        if current != self.saved {
            if let Err(e) = current.save() {
                log::warn!("saving settings: {e:#}");
            }
            self.saved = current;
        }
        if self.shared.calibrations_dirty.swap(false, Ordering::Relaxed) {
            let store = self.shared.calibrations.lock().clone();
            if let Err(e) = config::save_calibrations(&store) {
                log::warn!("saving calibrations: {e:#}");
            }
        }
    }

    /// The window has no system frame, so its edges and corners resize it here. The minimum
    /// size is enforced by the system while dragging; a size restored from an older run
    /// (or from a smaller minimum) is raised once at the start.
    fn window_edges(&mut self, ctx: &egui::Context) {
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

    /// Color and words of the status pill: the one place that says whether tracking works.
    fn status_line(&self, s: &Settings, snapshot: &Snapshot) -> (Color32, String) {
        status_line(&self.pal, s.language, s.ft_output, &snapshot.status, snapshot.tracking_fps)
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        // The window has no system title bar: the empty part of this bar drags it, a double
        // click maximizes it. Registered first so the controls drawn over it win the pointer.
        let drag = ui.interact(ui.max_rect(), ui.make_persistent_id("title_drag"), Sense::click_and_drag());
        let ctx = ui.ctx().clone();
        let inside = ctx.pointer_hover_pos().is_some_and(|p| ctx.content_rect().shrink(RESIZE_BAND * 2.0).contains(p));
        if drag.drag_started_by(egui::PointerButton::Primary) && inside {
            ui.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let maximized = ui.input(|i| i.viewport().maximized).unwrap_or(false);
        if drag.double_clicked() {
            ui.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
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
                if widgets::window_button(ui, &pal, Glyph::Close, true).clicked() {
                    ui.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                let (glyph, target) = if maximized { (Glyph::Restore, false) } else { (Glyph::Maximize, true) };
                if widgets::window_button(ui, &pal, glyph, false).clicked() {
                    ui.send_viewport_cmd(egui::ViewportCommand::Maximized(target));
                }
                if widgets::window_button(ui, &pal, Glyph::Minimize, false).clicked() {
                    ui.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
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

    fn camera_card(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot, large: bool) {
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
            let current = format!("{}\u{00d7}{}", s.resolution.0, s.resolution.1);
            let resolution =
                egui::ComboBox::from_id_salt("quick-resolution").width(half).selected_text(current).show_ui(ui, |ui| {
                    for r in RESOLUTIONS {
                        ui.selectable_value(&mut s.resolution, r, format!("{}\u{00d7}{}", r.0, r.1));
                    }
                });
            resolution.response.on_hover_text(self.t(s, "resolution"));
            let fps = egui::ComboBox::from_id_salt("quick-fps")
                .width(half)
                .selected_text(format!("{} FPS", s.camera_fps))
                .show_ui(ui, |ui| {
                    for rate in [30, 60] {
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
    fn camera_strip(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        let (fw, fh) = snapshot.frame_size;
        let aspect = if fw > 0 && fh > 0 { fw as f32 / fh as f32 } else { 16.0 / 9.0 };
        let preview = Vec2::new(STRIP_PREVIEW * aspect, STRIP_PREVIEW);
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

    fn calibration_card(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot, profiles: bool) {
        let pal = self.pal;
        // A step shows a check once this avatar has it saved.
        let (neutral_done, gaze_done) = {
            let store = self.shared.calibrations.lock();
            snapshot.avatar_id.as_ref().and_then(|id| store.avatars.get(id)).map_or((false, false), |c| {
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

    /// A yes and a no, small, for the actions that cannot be undone: (yes, no) pressed.
    fn confirm_buttons(&self, ui: &mut egui::Ui, s: &Settings) -> (bool, bool) {
        let pal = self.pal;
        let yes = widgets::icon_button(ui, &pal, Icon::Draw(Glyph::Check), true).on_hover_text(self.t(s, "yes"));
        let no = widgets::icon_button(ui, &pal, Icon::Draw(Glyph::Close), false).on_hover_text(self.t(s, "no"));
        (yes.clicked(), no.clicked())
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

    /// Search and the switches that apply to all three tables: one row, or two when narrow.
    fn tools_row(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
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

    /// Sets every parameter back to its defaults. It asks first, with a yes and a no, so a
    /// slip does not lose the calibration of the avatar.
    fn reset_all_button(&mut self, ui: &mut egui::Ui, s: &mut Settings, avatar: &str) {
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

    /// The first view: the camera as large as the window allows next to the calibration.
    fn simple_view(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot, compact: bool) {
        if compact {
            egui::ScrollArea::vertical().id_salt("simple").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                self.camera_card(ui, s, snapshot, false);
                self.calibration_card(ui, s, snapshot, true);
                let size = Vec2::new(ui.available_width(), 420.0);
                ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(size);
                    self.mediapipe_panel(ui, s, snapshot, true);
                });
            });
            return;
        }
        let gap = 16.0;
        let height = ui.available_height();
        let main = ui.available_width() - SIDEBAR_WIDTH - gap;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let column = Layout::top_down(Align::Min);
            ui.allocate_ui_with_layout(Vec2::new(main, height), column, |ui| self.camera_card(ui, s, snapshot, true));
            ui.allocate_ui_with_layout(Vec2::new(SIDEBAR_WIDTH, height), column, |ui| {
                self.calibration_card(ui, s, snapshot, true);
                ui.add_space(12.0);
                // The rest of the column: every MediaPipe parameter, to search and calibrate.
                let size = Vec2::new(SIDEBAR_WIDTH, ui.available_height().max(160.0));
                ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(size);
                    self.mediapipe_panel(ui, s, snapshot, true);
                });
            });
        });
    }

    fn preview(&mut self, ui: &mut egui::Ui, s: &Settings, snapshot: &Snapshot, tallest: f32) {
        let pal = self.pal;
        let (fw, fh) = snapshot.frame_size;
        let aspect = if fw > 0 && fh > 0 { fw as f32 / fh as f32 } else { 16.0 / 9.0 };
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
            Some(format!("{} {:.0}%", self.t(s, "hold_still"), progress * 100.0))
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
        let value = |name: &str| {
            OUTPUTS.iter().position(|o| o.name == name).and_then(|i| snapshot.outputs.get(i)).copied().unwrap_or(0.0)
        };
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

    #[allow(clippy::too_many_arguments)]
    fn channel_cell(
        &self,
        ui: &mut egui::Ui,
        s: &Settings,
        snapshot: &Snapshot,
        index: usize,
        used: bool,
        width: f32,
        avatar: &str,
        highlight: bool,
    ) -> egui::Response {
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
        let modified = {
            let store = self.shared.calibrations.lock();
            self.is_modified(s, store.avatars.get(avatar).unwrap_or(&AvatarCalibration::default()), index)
        };
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
                self.modifications(s, &store.avatars.get(avatar).cloned().unwrap_or_default(), index)
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

    fn mediapipe_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot, simple: bool) {
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
        let filtered = move |i: usize| {
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
                        .into_iter()
                        .filter(|(l, r)| {
                            let show = |i: usize| {
                                visible(i)
                                    && (show_unused || used[i])
                                    && filtered(i)
                                    && (self.matches(&channels::label(i)) || self.matches(channels::name(i)))
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
                                        let highlight = related.channels[i];
                                        let cell =
                                            self.channel_cell(ui, s, snapshot, i, used[i], width, &avatar, highlight);
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

    fn unified_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
        let pal = self.pal;
        let visible: Vec<usize> = (0..OUTPUTS.len()).filter(|&i| !OUTPUTS[i].cat_ears || s.cat_ears).collect();
        let bound = visible.iter().filter(|&&i| snapshot.bindings.get(i).is_some_and(Option::is_some)).count();
        let known = Self::avatar_known(snapshot);
        let related = self.related();
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
                        (s.show_unused || !known || is_bound)
                            && (self.matches(output.name) || self.matches(output.friendly))
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

    fn avatar_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
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
                    if !(self.matches(output.name) || self.matches(output.friendly)) {
                        continue;
                    }
                    let value = snapshot.outputs.get(*i).copied().unwrap_or_default();
                    let bits = (binding.bit_count() > 0).then(|| binding.encode(value));
                    let expanded = self.expanded_bindings.contains(i);
                    let mut spec = Row::new(output.label(s.friendly_names), value);
                    spec.signed = output.signed;
                    spec.bits = bits.as_ref().map(|(b, _)| b.as_slice());
                    spec.chevron = bits.is_some().then_some(expanded);
                    spec.highlight = related.outputs[*i];
                    let response = value_row(ui, &pal, spec);
                    self.rows.avatar.push((*i, response.rect));
                    self.reveal(ui, 2, *i, response.rect);
                    if response.hovered() {
                        self.hover_now = Some(Hover::Avatar(*i));
                    }
                    if bits.is_some() && response.clicked() && !self.expanded_bindings.remove(i) {
                        self.expanded_bindings.insert(*i);
                    }
                    if expanded {
                        let (set, sign) = binding.encode(value);
                        let bits = binding.bits.iter().zip(set).chain(binding.negative.iter().map(|a| (a, sign)));
                        for (address, on) in bits {
                            bit_row(ui, &pal, address, on);
                        }
                    }
                }
                if s.show_unused {
                    for name in &unmatched {
                        if self.matches(name) {
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
    fn linked(&self, panel: usize, index: usize) -> bool {
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
    fn draw_links(&self, ui: &egui::Ui) {
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

    fn import_tongue(&self) {
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
            Device::Gpu(i) => self
                .shared
                .gpus
                .iter()
                .find(|g| g.index == i)
                .map(|g| format!("GPU {i}: {}", g.name))
                .unwrap_or_else(|| format!("GPU {i}")),
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

    fn settings_panel(&mut self, ui: &mut egui::Ui, s: &mut Settings, snapshot: &Snapshot) {
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
                    let current = format!("{}×{}", s.resolution.0, s.resolution.1);
                    egui::ComboBox::from_id_salt("resolution").selected_text(current).show_ui(ui, |ui| {
                        for r in RESOLUTIONS {
                            ui.selectable_value(&mut s.resolution, r, format!("{}×{}", r.0, r.1));
                        }
                    });
                    ui.end_row();
                    ui.label(RichText::new(self.t(s, "camera_fps")).color(pal.dim));
                    egui::ComboBox::from_id_salt("fps").selected_text(s.camera_fps.to_string()).show_ui(ui, |ui| {
                        for fps in [30, 60] {
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
                    ui.label(
                        RichText::new("GPU inference needs a build with the `directml` feature.")
                            .color(pal.dim)
                            .small(),
                    );
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

    /// A done state: a green check and a line of text.
    fn ok_label(&self, ui: &mut egui::Ui, text: &str) {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        icons::paint(ui.painter(), Glyph::Check, rect, self.pal.ok);
        ui.label(RichText::new(text).color(self.pal.dim));
    }

    /// Heading of a group in the settings.
    fn section(&self, ui: &mut egui::Ui, title: &str) {
        ui.add_space(16.0);
        ui.label(RichText::new(title).font(FontId::new(12.0, widgets::semibold())).color(self.pal.dim));
        ui.add_space(2.0);
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

    fn performance_window(&self, ctx: &egui::Context, s: &mut Settings) {
        let pal = self.pal;
        let perf = self.shared.perf.lock().clone();
        let mut open = s.debug;
        let language = s.language;
        let t = |key: &str| tr(language, key);
        let (one, twelve) = (Duration::from_secs(1), Duration::from_secs(12));
        let builder = egui::ViewportBuilder::default()
            .with_title(t("debug_performance"))
            .with_decorations(false)
            .with_inner_size([620.0, 780.0])
            .with_min_inner_size([480.0, 420.0]);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("debug_performance"), builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                open = false;
            }
            resize_edges(ui.ctx());
            let bar = Frame::new().fill(pal.bg).inner_margin(Margin::symmetric(14, 8));
            egui::Panel::top("debug-bar").frame(bar).show(ui, |ui| {
                // Same as the main window: the bar drags it, a double click maximizes it.
                let drag = ui.interact(ui.max_rect(), ui.make_persistent_id("debug_drag"), Sense::click_and_drag());
                let ctx = ui.ctx().clone();
                let inside =
                    ctx.pointer_hover_pos().is_some_and(|p| ctx.content_rect().shrink(RESIZE_BAND * 2.0).contains(p));
                if drag.drag_started_by(egui::PointerButton::Primary) && inside {
                    ui.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                let maximized = ui.input(|i| i.viewport().maximized).unwrap_or(false);
                if drag.double_clicked() {
                    ui.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                ui.horizontal(|ui| {
                    if let Some(mark) = &self.mark {
                        ui.add(egui::Image::new((mark.id(), Vec2::splat(20.0))));
                    }
                    ui.label(RichText::new(t("debug_performance")).font(FontId::new(14.0, widgets::semibold())));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::window_button(ui, &pal, Glyph::Close, true).clicked() {
                            open = false;
                        }
                        let (glyph, target) = if maximized { (Glyph::Restore, false) } else { (Glyph::Maximize, true) };
                        if widgets::window_button(ui, &pal, glyph, false).clicked() {
                            ui.send_viewport_cmd(egui::ViewportCommand::Maximized(target));
                        }
                        if widgets::window_button(ui, &pal, Glyph::Minimize, false).clicked() {
                            ui.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    });
                });
            });
            egui::CentralPanel::default().frame(Frame::new().fill(pal.bg).inner_margin(Margin::same(14))).show(
                ui,
                |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        // Latency bars: where each frame spends its time on the way to each result.
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(t("latency")).strong());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                for (segment, key) in [
                                    (Segment::Waiting, "perf_waiting"),
                                    (Segment::Work(Group::Interface), "perf_interface"),
                                    (Segment::Work(Group::Core), "perf_core"),
                                    (Segment::Work(Group::Tracker), "perf_tracker"),
                                ] {
                                    ui.label(RichText::new(t(key)).small());
                                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                                    paint_segment(ui.painter(), rect, segment, &pal, None);
                                }
                            });
                        });
                        let rows: Vec<(Path, Option<perf::Breakdown>)> = Path::ALL
                            .iter()
                            .map(|p| (*p, perf.paths.get(p).and_then(|series| series.average(one))))
                            .collect();
                        let longest = rows
                            .iter()
                            .filter_map(|(_, b)| b.as_ref().map(|b| b.iter().map(|(_, v)| v).sum::<f32>()))
                            .fold(0.0f32, f32::max);
                        let scale = ((longest / 10.0).ceil() * 10.0).max(30.0);
                        let (label_w, value_w) = (130.0, 64.0);
                        let (axis, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::hover());
                        let bar_x =
                            |rect: Rect, v: f32| rect.left() + label_w + v / scale * (rect.width() - label_w - value_w);
                        let ticks: Vec<f32> = (0..=(scale / 10.0) as usize).map(|i| i as f32 * 10.0).collect();
                        for &tick in &ticks {
                            ui.painter().text(
                                Pos2::new(bar_x(axis, tick), axis.center().y),
                                Align2::CENTER_CENTER,
                                format!("{tick:.0}"),
                                FontId::proportional(10.5),
                                pal.dim,
                            );
                        }
                        ui.painter().text(
                            axis.right_center(),
                            Align2::RIGHT_CENTER,
                            "ms",
                            FontId::proportional(10.5),
                            pal.dim,
                        );
                        for (path, breakdown) in &rows {
                            let (row, _) =
                                ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
                            let painter = ui.painter_at(row);
                            let name = t(path.text_key());
                            painter.text(
                                row.left_center(),
                                Align2::LEFT_CENTER,
                                name,
                                FontId::proportional(11.5),
                                pal.text,
                            );
                            for &tick in &ticks {
                                painter.vline(
                                    bar_x(row, tick),
                                    row.y_range(),
                                    Stroke::new(1.0, pal.line.gamma_multiply(0.5)),
                                );
                            }
                            let Some(breakdown) = breakdown else {
                                painter.text(
                                    row.right_center(),
                                    Align2::RIGHT_CENTER,
                                    "—",
                                    FontId::proportional(11.5),
                                    pal.dim,
                                );
                                continue;
                            };
                            let mut x = bar_x(row, 0.0);
                            let mut total = 0.0;
                            for (segment, v) in breakdown {
                                total += v;
                                let end = bar_x(row, total);
                                let rect = Rect::from_x_y_ranges(x..=end, row.top() + 3.0..=row.bottom() - 3.0);
                                if rect.width() >= 0.5 {
                                    paint_segment(&painter, rect, *segment, &pal, Some(t("perf_queue")));
                                }
                                x = end;
                            }
                            let text = format!("{total:.1} ms");
                            painter.text(
                                row.right_center(),
                                Align2::RIGHT_CENTER,
                                text,
                                FontId::proportional(11.5),
                                pal.text,
                            );
                        }
                        ui.separator();

                        // Totals over time.
                        let history = |path: Path| -> Vec<(f32, f32)> {
                            perf.paths.get(&path).map(|series| series.total.history().collect()).unwrap_or_default()
                        };
                        let osc = perf.paths.get(&Path::MediaPipeOsc).and_then(|series| series.total.average(twelve));
                        ui.horizontal(|ui| {
                            ui.label(t("total_latency"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(osc.map(|v| format!("{v:.0} ms")).unwrap_or_else(|| "—".into()));
                            });
                        });
                        let lines = [
                            (history(Path::MediaPipeOsc), group_color(Group::Core), false, "OSC"),
                            (history(Path::Tables), group_color(Group::Interface), false, t("path_tables")),
                            (history(Path::Preview), pal.text, true, t("path_preview")),
                        ];
                        multi_graph(ui, &pal, &lines, 12.0);
                        ui.horizontal(|ui| {
                            for (_, color, dashed, name) in &lines {
                                let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 10.0), Sense::hover());
                                let points = [rect.left_center(), rect.right_center()];
                                if *dashed {
                                    ui.painter().extend(Shape::dashed_line(
                                        &points,
                                        Stroke::new(1.5, *color),
                                        3.0,
                                        2.0,
                                    ));
                                } else {
                                    ui.painter().line_segment(points, Stroke::new(1.5, *color));
                                }
                                ui.label(RichText::new(*name).small());
                                ui.add_space(6.0);
                            }
                        });
                        ui.separator();

                        // Memory over the last 10 minutes.
                        let now = Instant::now();
                        let to_mib = |b: u64| b as f32 / (1024.0 * 1024.0);
                        let series = |pick: fn(&perf::MemorySample) -> u64| -> Vec<(f32, f32)> {
                            perf.memory
                                .iter()
                                .map(|(at, m)| (now.duration_since(*at).as_secs_f32(), to_mib(pick(m))))
                                .collect()
                        };
                        let private = series(|m| m.private);
                        let working = series(|m| m.working);
                        let (lo, hi) =
                            private.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
                        ui.horizontal(|ui| {
                            ui.label(t("memory"));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if hi >= lo {
                                    ui.label(format!("±{:.0} MiB", (hi - lo) / 2.0));
                                }
                            });
                        });
                        graph(ui, &pal, &private, 600.0, None);
                        egui::Grid::new("memory-table").num_columns(4).spacing([18.0, 4.0]).show(ui, |ui| {
                            ui.label("");
                            for key in ["mem_now", "mem_change", "mem_trend"] {
                                ui.label(RichText::new(t(key)).color(pal.dim).small());
                            }
                            ui.end_row();
                            for (key, points) in [("mem_private", &private), ("mem_working", &working)] {
                                ui.label(t(key));
                                let (first, last) = (points.first().map(|p| p.1), points.last().map(|p| p.1));
                                ui.label(last.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into()));
                                ui.label(
                                    first
                                        .zip(last)
                                        .map(|(a, b)| format!("{:+.1}", b - a))
                                        .unwrap_or_else(|| "—".into()),
                                );
                                // Points carry their age; the trend needs time running forward, in minutes.
                                let forward: Vec<(f32, f32)> =
                                    points.iter().map(|(age, v)| (-age / 60.0, *v)).collect();
                                // The first minute is mostly start-up loading, not a trend.
                                let span = points.first().map_or(0.0, |p| p.0);
                                let trend = perf::trend_per_minute(&forward).filter(|_| span >= 60.0);
                                ui.label(trend.map(|v| format!("{v:+.2}")).unwrap_or_else(|| "—".into()));
                                ui.end_row();
                            }
                        });
                        ui.separator();

                        // Stage averages, by where they run.
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(t("averages")).color(pal.dim).small());
                            });
                        });
                        let value = |stage: Stage, over: Duration| {
                            perf.average(stage, over).map(|v| format!("{v:.2}")).unwrap_or_else(|| "—".into())
                        };
                        egui::Grid::new("stage-groups").num_columns(2).spacing([28.0, 12.0]).show(ui, |ui| {
                            for pair in Group::ALL.chunks(2) {
                                for &group in pair {
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                                            paint_segment(ui.painter(), rect, Segment::Work(group), &pal, None);
                                            ui.label(RichText::new(t(group.text_key())).strong());
                                        });
                                        egui::Grid::new(("stages", group)).num_columns(3).spacing([14.0, 2.0]).show(
                                            ui,
                                            |ui| {
                                                for stage in Stage::ALL.into_iter().filter(|st| st.group() == group) {
                                                    ui.label(t(stage.text_key()));
                                                    ui.label(value(stage, one));
                                                    ui.label(value(stage, twelve));
                                                    ui.end_row();
                                                }
                                            },
                                        );
                                    });
                                }
                                ui.end_row();
                            }
                        });
                    });
                },
            );
        });
        s.debug = open;
    }

    fn sleeping(&self, s: &Settings) -> bool {
        s.idle_sleep && self.last_activity.elapsed() > Duration::from_secs(s.idle_minutes as u64 * 60)
    }
}

/// Turns the outer `RESIZE_BAND` points of a frameless window into resize handles.
fn resize_edges(ctx: &egui::Context) {
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

/// Colour of a stage group in the debug view (fixed, so it reads the same in every scheme).
fn group_color(group: Group) -> Color32 {
    match group {
        Group::Camera => Color32::from_rgb(200, 200, 200),
        Group::Tracker => Color32::from_rgb(86, 156, 230),
        Group::Core => Color32::from_rgb(240, 160, 64),
        Group::Interface => Color32::from_rgb(168, 140, 255),
    }
}

/// A latency segment: solid in its group colour, or hatched while waiting (labelled
/// `queue` when there is room). Camera is drawn as an outline.
fn paint_segment(painter: &egui::Painter, rect: Rect, segment: Segment, pal: &Palette, queue: Option<&str>) {
    match segment {
        Segment::Work(Group::Camera) => {
            painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.text), StrokeKind::Inside);
        }
        Segment::Work(group) => {
            painter.rect_filled(rect, CornerRadius::ZERO, group_color(group));
        }
        Segment::Waiting => {
            let hatch = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
            let mut x = rect.left() - rect.height();
            while x < rect.right() {
                hatch.line_segment(
                    [Pos2::new(x, rect.bottom()), Pos2::new(x + rect.height(), rect.top())],
                    Stroke::new(1.0, pal.dim),
                );
                x += 5.0;
            }
            painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.dim), StrokeKind::Inside);
            if let Some(queue) = queue
                && rect.width() > 46.0
            {
                let label = Rect::from_center_size(rect.center(), Vec2::new(40.0, rect.height() - 2.0));
                painter.rect_filled(label, CornerRadius::ZERO, pal.bg);
                painter.text(rect.center(), Align2::CENTER_CENTER, queue, FontId::proportional(10.5), pal.text);
            }
        }
    }
}

/// One line of `multi_graph`: (age seconds, ms) points, colour, dashed, name.
type GraphLine<'a> = (Vec<(f32, f32)>, Color32, bool, &'a str);

/// Several lines on one chart from 0 ms, newest on the right.
fn multi_graph(ui: &mut egui::Ui, pal: &Palette, lines: &[GraphLine<'_>], span: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.line), StrokeKind::Inside);
    let hi = lines.iter().flat_map(|l| l.0.iter().map(|p| p.1)).fold(1.0f32, f32::max);
    for (points, color, dashed, _) in lines {
        if points.len() < 2 {
            continue;
        }
        let line: Vec<Pos2> = points
            .iter()
            .map(|(age, v)| {
                Pos2::new(
                    rect.right() - age / span * rect.width(),
                    rect.bottom() - 4.0 - v / hi * (rect.height() - 8.0),
                )
            })
            .collect();
        let stroke = Stroke::new(1.2, *color);
        if *dashed {
            painter.extend(Shape::dashed_line(&line, stroke, 4.0, 3.0));
        } else {
            painter.add(Shape::line(line, stroke));
        }
    }
}

/// Line chart of (age seconds, value) points, newest on the right.
fn graph(ui: &mut egui::Ui, pal: &Palette, points: &[(f32, f32)], span: f32, floor: Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, pal.line), StrokeKind::Inside);
    if points.len() < 2 {
        return;
    }
    let lo = floor.unwrap_or_else(|| points.iter().map(|p| p.1).fold(f32::MAX, f32::min));
    let hi = points.iter().map(|p| p.1).fold(f32::MIN, f32::max).max(lo + 1e-3);
    let line: Vec<Pos2> = points
        .iter()
        .map(|(age, v)| {
            Pos2::new(
                rect.right() - age / span * rect.width(),
                rect.bottom() - 4.0 - (v - lo) / (hi - lo) * (rect.height() - 8.0),
            )
        })
        .collect();
    painter.add(Shape::line(line, Stroke::new(1.2, pal.accent)));
}

/// Parameter row: name and value, as one line of a table. `bits` draws a binary
/// encoding as boxes.
/// One row of a value table.
struct Row<'a> {
    name: &'a str,
    value: f32,
    /// Used by the avatar (or nothing is known yet); unused rows are dimmed.
    active: bool,
    /// Runs -1..1 instead of 0..1.
    signed: bool,
    bits: Option<&'a [bool]>,
    /// Expandable row: `Some(open)` draws the chevron.
    chevron: Option<bool>,
    highlight: bool,
}

impl<'a> Row<'a> {
    fn new(name: &'a str, value: f32) -> Self {
        Self { name, value, active: true, signed: false, bits: None, chevron: None, highlight: false }
    }
}

fn value_row(ui: &mut egui::Ui, pal: &Palette, row: Row<'_>) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
    let painter = ui.painter_at(rect);
    paint_value_row(&painter, rect, pal, &row, row.highlight || response.hovered());
    response
}

/// Rows are inset a little so the meters and highlights read as rounded bars.
fn row_inset(rect: Rect) -> Rect {
    rect.shrink2(Vec2::new(2.0, 1.0))
}

/// The value as a thin bar under the row text: from the left for 0..1, from the middle for -1..1.
fn paint_meter(painter: &egui::Painter, rect: Rect, pal: &Palette, value: f32, signed: bool, active: bool) {
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
fn bit_row(ui: &mut egui::Ui, pal: &Palette, address: &str, on: bool) {
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

/// Color and words for the tracking state, shared by the status pill and the tray menu.
pub fn status_line(pal: &Palette, language: Language, ft_output: bool, status: &Status, fps: f32) -> (Color32, String) {
    if !ft_output {
        return (pal.warn, tr(language, "paused").to_string());
    }
    match status {
        Status::Running => (pal.ok, format!("{} · {:.0} FPS", tr(language, "tracking"), fps)),
        Status::Starting => (pal.dim, tr(language, "starting").to_string()),
        Status::NoFace => (pal.warn, tr(language, "no_face").to_string()),
        Status::CameraError(_) => (pal.err, tr(language, "camera_error").to_string()),
        Status::ModelError(_) => (pal.err, tr(language, "model_error").to_string()),
    }
}

impl eframe::App for App {
    /// Also runs while the window is hidden in the tray: keeps the tray icon's dot current.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let Some(tray) = &mut self.tray else { return };
        tray.refresh(&self.pal, self.mark.as_ref().map(|m| m.id()));
        if !tray.window_visible() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let draw_started = Instant::now();
        let ctx = ui.ctx().clone();
        if ctx.input(|i| !i.events.is_empty() || i.pointer.velocity() != Vec2::ZERO) {
            self.last_activity = Instant::now();
        }
        let mut s = self.shared.settings.read().clone();
        let before = s.clone();

        if let Some(tray) = &mut self.tray {
            // Before any early return: the menu window must be declared in every pass.
            tray.declare_menu(&ctx);
            if s.close_to_tray && ctx.input(|i| i.viewport().close_requested()) && !tray.quitting() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                tray.hide_window();
            }
            if !tray.window_visible() {
                // Nothing to draw; tracking and OSC go on without the interface.
                self.shared.ui_awake.store(false, Ordering::Relaxed);
                return;
            }
        }

        if self.font_language != s.language {
            load_fonts(&ctx, s.language);
            self.font_language = s.language;
        }

        let style_key = (s.scheme, s.custom_colors, s.interface_scale);
        if self.applied_style != Some(style_key) {
            self.pal = Palette::of(s.scheme, &s.custom_colors);
            apply_style(&ctx, &self.pal);
            ctx.set_zoom_factor(s.interface_scale);
            self.applied_style = Some(style_key);
        }

        if self.sleeping(&s) {
            self.shared.ui_awake.store(false, Ordering::Relaxed);
            egui::CentralPanel::default().frame(Frame::new().fill(self.pal.bg)).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.4);
                    ui.label(RichText::new(self.t(&s, "sleep_title")).heading());
                    ui.label(RichText::new(self.t(&s, "sleep_body")).color(self.pal.dim));
                });
            });
            ctx.request_repaint_after(Duration::from_secs(1));
            self.persist();
            return;
        }
        self.shared.ui_awake.store(true, Ordering::Relaxed);

        let copy_started = Instant::now();
        let snapshot = self.shared.snapshot.lock().clone();
        self.shared.perf.lock().record(Stage::Snapshot, ms_since(copy_started));
        // A frame drawn for the first time adds to the Tables and Preview latencies.
        let new_frame = (snapshot.frame_seq != self.seen_frame).then_some(snapshot.times).flatten();
        self.seen_frame = snapshot.frame_seq;
        let new_preview = self.update_texture(&ctx, &snapshot);
        self.hover = self.hover_now.take();
        self.prev_rows = std::mem::take(&mut self.rows);
        if self.dwell.0 != self.hover {
            self.dwell = (self.hover, Instant::now());
            self.revealed = [false; 3];
        }
        if self.hover.is_some() && self.dwell.1.elapsed() < REVEAL_AFTER {
            ctx.request_repaint_after(REVEAL_AFTER);
        }

        // Too narrow for a sidebar: the camera and calibration move to the top of the content.
        let compact = ui.available_width() < 760.0;
        let bar = Frame::new().fill(self.pal.bg).inner_margin(Margin::symmetric(16, 10));
        let top = egui::Panel::top("top").frame(bar).show(ui, |ui| self.top_bar(ui, &mut s, &snapshot));
        let edge = top.response.rect;
        ui.painter().hline(edge.x_range(), edge.bottom() - 0.5, Stroke::new(1.0, self.pal.line));
        self.window_edges(&ctx);
        let advanced = s.advanced;
        let body = Frame::new().fill(self.pal.bg).inner_margin(Margin::symmetric(16, 14));
        egui::CentralPanel::default().frame(body).show(ui, |ui| {
            if !advanced {
                self.simple_view(ui, &mut s, &snapshot, compact);
                if let Some(times) = new_frame
                    && new_preview
                {
                    let breakdown = perf::interface_breakdown(&times, draw_started, Instant::now());
                    self.shared.perf.lock().record_path(Path::Preview, breakdown);
                }
                return;
            }
            if !compact {
                self.camera_strip(ui, &mut s, &snapshot);
                ui.add_space(8.0);
                if let Some(times) = new_frame
                    && new_preview
                {
                    let breakdown = perf::interface_breakdown(&times, draw_started, Instant::now());
                    self.shared.perf.lock().record_path(Path::Preview, breakdown);
                }
            } else {
                let title = format!("{} · {}", self.t(&s, "camera"), self.t(&s, "calibrate"));
                let title = widgets::title(&self.pal, &title);
                egui::CollapsingHeader::new(title).default_open(false).show(ui, |ui| {
                    // The two cards side by side, as wide as the sidebar would make them.
                    let width = ((ui.available_width() - 12.0) / 2.0).floor();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let column = Layout::top_down(Align::Min);
                        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), column, |ui| {
                            self.camera_card(ui, &mut s, &snapshot, false);
                        });
                        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), column, |ui| {
                            self.calibration_card(ui, &mut s, &snapshot, false);
                        });
                    });
                });
                ui.add_space(8.0);
            }
            self.tools_row(ui, &mut s, &snapshot);
            ui.add_space(12.0);

            let height = (ui.available_height() - 4.0).max(120.0);
            let width = ui.available_width();
            // Wide enough for the links between the panels to be readable.
            let gap = 26.0;
            // Below this width three columns get too narrow to read; the Unified Expressions
            // and Avatar panels then share the right column, one above the other.
            self.narrow = width < 900.0;
            let mp = if self.narrow { (width - gap) * 0.5 } else { (width - gap * 2.0) * 0.48 };
            let column = |ui: &mut egui::Ui, size: Vec2, add: &mut dyn FnMut(&mut egui::Ui)| {
                ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(size);
                    add(ui);
                });
            };
            let tables_started = Instant::now();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                column(ui, Vec2::new(mp, height), &mut |ui| self.mediapipe_panel(ui, &mut s, &snapshot, false));
                if self.narrow {
                    let side = width - gap - mp;
                    let half = ((height - 10.0) / 2.0).floor();
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        column(ui, Vec2::new(side, half), &mut |ui| self.unified_panel(ui, &mut s, &snapshot));
                        column(ui, Vec2::new(side, half), &mut |ui| self.avatar_panel(ui, &mut s, &snapshot));
                    });
                } else {
                    let side = (width - gap * 2.0 - mp) / 2.0;
                    column(ui, Vec2::new(side, height), &mut |ui| self.unified_panel(ui, &mut s, &snapshot));
                    column(ui, Vec2::new(side, height), &mut |ui| self.avatar_panel(ui, &mut s, &snapshot));
                }
            });
            self.draw_links(ui);
            let mut perf = self.shared.perf.lock();
            perf.record(Stage::Tables, ms_since(tables_started));
            if let Some(times) = new_frame {
                perf.record_path(Path::Tables, perf::interface_breakdown(&times, draw_started, Instant::now()));
            }
        });
        if self.settings_open {
            let frame = Frame::new()
                .fill(self.pal.panel)
                .stroke(Stroke::new(1.0, self.pal.line))
                .corner_radius(CornerRadius::same(widgets::RADIUS + 6))
                .inner_margin(Margin::same(20));
            let modal = egui::Modal::new(ui.make_persistent_id("settings_modal"))
                .frame(frame)
                .backdrop_color(Color32::from_black_alpha(150))
                .show(&ctx, |ui| {
                    ui.set_width(460.0);
                    self.settings_panel(ui, &mut s, &snapshot);
                });
            if modal.should_close() {
                self.settings_open = false;
            }
        }
        if self.hover_now != self.hover {
            // Highlights follow the previous frame's hover; draw once more to catch up.
            ctx.request_repaint();
        }
        if s.debug {
            self.performance_window(&ctx, &mut s);
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        if s != before {
            // The tracker may have updated `last_avatar_id` while we drew; keep its value.
            let mut shared = self.shared.settings.write();
            let last_avatar = shared.last_avatar_id.clone();
            *shared = s.clone();
            shared.last_avatar_id = last_avatar;
            self.shared.receive_port.store(s.receive_port, Ordering::Relaxed);
        }
        self.shared.perf.lock().record(Stage::Draw, ms_since(draw_started));
        self.persist();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let settings = self.shared.settings.read().clone();
        if let Err(e) = settings.save() {
            log::warn!("saving settings: {e:#}");
        }
        let store = self.shared.calibrations.lock().clone();
        if let Err(e) = config::save_calibrations(&store) {
            log::warn!("saving calibrations: {e:#}");
        }
    }
}
