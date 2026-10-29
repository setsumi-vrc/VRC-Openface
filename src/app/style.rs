//! Fonts, the egui style built from the palette, and the images loaded at start.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, ColorImage, CornerRadius, FontFamily, FontId, Stroke, TextStyle, TextureHandle, TextureOptions, Vec2,
};

use crate::i18n::Language;
use crate::ui::theme::{Palette, mix};
use crate::ui::widgets;

/// Flag images by language code (drawn by `tools/make_flags.py`).
pub(super) const FLAGS: [(&str, &[u8]); 10] = [
    ("en", include_bytes!("../../assets/flags/en.png")),
    ("es", include_bytes!("../../assets/flags/es.png")),
    ("pt", include_bytes!("../../assets/flags/pt.png")),
    ("fr", include_bytes!("../../assets/flags/fr.png")),
    ("de", include_bytes!("../../assets/flags/de.png")),
    ("it", include_bytes!("../../assets/flags/it.png")),
    ("ru", include_bytes!("../../assets/flags/ru.png")),
    ("zh", include_bytes!("../../assets/flags/zh.png")),
    ("ja", include_bytes!("../../assets/flags/ja.png")),
    ("ko", include_bytes!("../../assets/flags/ko.png")),
];

pub(super) fn load_png(ctx: &egui::Context, name: &str, png: &[u8]) -> Option<TextureHandle> {
    let icon = eframe::icon_data::from_png_bytes(png).map_err(|e| log::warn!("{name} icon: {e}")).ok()?;
    let image = ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
}

/// Font files by role, the first one found wins, with the face to use in a collection.
#[cfg(windows)]
mod font_files {
    pub const UI: &[(&str, u32)] = &[("segoeui.ttf", 0)];
    pub const UI_SEMIBOLD: &[(&str, u32)] = &[("seguisb.ttf", 0)];
    pub const JAPANESE: &[(&str, u32)] = &[("YuGothM.ttc", 0), ("msgothic.ttc", 0)];
    pub const KOREAN: &[(&str, u32)] = &[("malgun.ttf", 0)];
    pub const CHINESE: &[(&str, u32)] = &[("msyh.ttc", 0)];
}

/// Noto (Sans and Sans CJK) and DejaVu, which most distributions install. The CJK collection
/// holds Japanese, Korean and Simplified Chinese as faces 0, 1 and 2.
#[cfg(not(windows))]
mod font_files {
    pub const UI: &[(&str, u32)] = &[("NotoSans-Regular.ttf", 0), ("DejaVuSans.ttf", 0)];
    pub const UI_SEMIBOLD: &[(&str, u32)] =
        &[("NotoSans-SemiBold.ttf", 0), ("NotoSans-Medium.ttf", 0), ("DejaVuSans-Bold.ttf", 0)];
    pub const JAPANESE: &[(&str, u32)] =
        &[("NotoSansCJK-Regular.ttc", 0), ("NotoSansCJKjp-Regular.otf", 0), ("NotoSansJP-Regular.otf", 0)];
    pub const KOREAN: &[(&str, u32)] =
        &[("NotoSansCJK-Regular.ttc", 1), ("NotoSansCJKkr-Regular.otf", 0), ("NotoSansKR-Regular.otf", 0)];
    pub const CHINESE: &[(&str, u32)] = &[
        ("NotoSansCJK-Regular.ttc", 2),
        ("NotoSansCJKsc-Regular.otf", 0),
        ("NotoSansSC-Regular.otf", 0),
        ("wqy-microhei.ttc", 0),
    ];
}

/// Every font file in the system's font folders by file name. Windows keeps them in one
/// folder; Linux spreads them over package subfolders of several roots.
fn font_paths() -> HashMap<OsString, PathBuf> {
    let mut roots = Vec::new();
    if let Some(windir) = std::env::var_os("WINDIR") {
        roots.push(PathBuf::from(windir).join("Fonts"));
    }
    if cfg!(not(windows)) {
        roots.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
        roots.extend(data_home.map(|d| d.join("fonts")));
        roots.extend(std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".fonts")));
    }
    let mut found = HashMap::new();
    while let Some(dir) = roots.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                roots.push(path);
            } else {
                found.entry(entry.file_name()).or_insert(path);
            }
        }
    }
    found
}

/// Whether `text` has Han, kana or Hangul characters, which need the CJK fonts.
pub(super) fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{1100}'..='\u{11FF}'       // Hangul Jamo
            | '\u{2E80}'..='\u{9FFF}'     // CJK radicals, punctuation, kana, Han
            | '\u{AC00}'..='\u{D7AF}'     // Hangul syllables
            | '\u{F900}'..='\u{FAFF}'     // CJK compatibility ideographs
            | '\u{FF00}'..='\u{FFEF}'     // half- and full-width forms
            | '\u{20000}'..='\u{2FA1F}') // Han extensions
    })
}

/// Loads the interface fonts: Segoe UI (regular and semibold, found on every Windows; Noto Sans
/// or DejaVu elsewhere) for the Latin and Cyrillic text, and with `cjk` the system's CJK fonts as
/// fallbacks. Those are about 47 MB, so they load only when something on screen needs them. For
/// Japanese, Korean and Chinese the CJK font of that language goes first: Han glyphs differ and
/// sit on different baselines between fonts, so mixing them within one line looks broken.
pub(super) fn load_fonts(ctx: &egui::Context, language: Language, cjk: bool) {
    let paths = font_paths();
    let load = |files: &[(&str, u32)]| {
        files.iter().find_map(|&(name, index)| {
            let bytes = std::fs::read(paths.get(OsStr::new(name))?).ok()?;
            Some(egui::FontData { index, ..egui::FontData::from_owned(bytes) })
        })
    };
    let preferred = match language {
        Language::Ja => "japanese",
        Language::Ko => "korean",
        Language::Zh => "chinese",
        _ => "",
    };
    let mut candidates = if cjk {
        [
            ("japanese", load(font_files::JAPANESE)),
            ("korean", load(font_files::KOREAN)),
            ("chinese", load(font_files::CHINESE)),
        ]
    } else {
        [("japanese", None), ("korean", None), ("chinese", None)]
    };
    candidates.sort_by_key(|(name, _)| *name != preferred);
    let mut definitions = egui::FontDefinitions::default();
    let defaults = |family: &FontFamily, definitions: &egui::FontDefinitions| {
        definitions.families.get(family).cloned().unwrap_or_default()
    };
    let (default_proportional, default_monospace) =
        (defaults(&FontFamily::Proportional, &definitions), defaults(&FontFamily::Monospace, &definitions));

    let mut cjk = Vec::new();
    for (name, font) in candidates {
        let Some(font) = font else { continue };
        definitions.font_data.insert(name.into(), Arc::new(font));
        cjk.push(name.to_string());
    }
    let mut ui_fonts = Vec::new();
    for (name, files) in [("ui", font_files::UI), ("ui-semibold", font_files::UI_SEMIBOLD)] {
        if let Some(font) = load(files) {
            definitions.font_data.insert(name.into(), Arc::new(font));
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
pub(super) fn native_font(language: Language) -> FontFamily {
    match language {
        Language::Ja => FontFamily::Name("japanese".into()),
        Language::Ko => FontFamily::Name("korean".into()),
        Language::Zh => FontFamily::Name("chinese".into()),
        _ => FontFamily::Proportional,
    }
}

pub(super) fn apply_style(ctx: &egui::Context, pal: &Palette) {
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
