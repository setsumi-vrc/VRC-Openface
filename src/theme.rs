use eframe::egui::Color32;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scheme {
    Black,
    Light,
    Violet,
    Pink,
    Blue,
    Green,
    /// The first-run default, starting from `CustomColors::default()`.
    #[default]
    Custom,
}

impl Scheme {
    pub const ALL: [Scheme; 7] =
        [Scheme::Black, Scheme::Light, Scheme::Violet, Scheme::Pink, Scheme::Blue, Scheme::Green, Scheme::Custom];

    pub fn text_key(self) -> &'static str {
        match self {
            Scheme::Black => "theme_black",
            Scheme::Light => "theme_light",
            Scheme::Violet => "theme_violet",
            Scheme::Pink => "theme_pink",
            Scheme::Blue => "theme_blue",
            Scheme::Green => "theme_green",
            Scheme::Custom => "theme_custom",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomColors {
    pub background: [u8; 3],
    pub panel: [u8; 3],
    pub text: [u8; 3],
    pub accent: [u8; 3],
}

impl Default for CustomColors {
    fn default() -> Self {
        Self { background: [8, 8, 12], panel: [16, 16, 24], text: [235, 235, 235], accent: [255, 170, 60] }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    /// Controls resting on a panel: buttons, fields, chips.
    pub raised: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub line: Color32,
    pub accent: Color32,
    /// Status colors: tracking works, something to look at, something is wrong.
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    /// Text drawn on an accent-filled button.
    pub on_accent: Color32,
    pub dark: bool,
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn luminance(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

fn build(bg: Color32, panel: Color32, text: Color32, accent: Color32) -> Palette {
    let dark = luminance(bg) < 0.5;
    // Status colors keep their meaning in every scheme; on light backgrounds they are darkened to stay readable.
    let status = |c: Color32| if dark { c } else { mix(c, Color32::BLACK, 0.35) };
    Palette {
        bg,
        panel,
        raised: mix(panel, text, 0.07),
        text,
        dim: mix(text, bg, 0.55),
        line: mix(text, bg, 0.78),
        accent,
        ok: status(Color32::from_rgb(98, 204, 140)),
        warn: status(Color32::from_rgb(240, 186, 72)),
        err: status(Color32::from_rgb(240, 104, 104)),
        on_accent: if luminance(accent) > 0.55 { Color32::BLACK } else { Color32::WHITE },
        dark,
    }
}

impl Palette {
    /// Fill of a control under the pointer: the resting fill moved toward the text color.
    pub fn hover(&self) -> Color32 {
        mix(self.raised, self.text, if self.dark { 0.12 } else { 0.10 })
    }

    pub fn of(scheme: Scheme, custom: &CustomColors) -> Palette {
        let c = Color32::from_rgb;
        match scheme {
            Scheme::Black => build(c(0, 0, 0), c(0, 0, 0), c(235, 235, 235), c(235, 235, 235)),
            Scheme::Light => build(c(246, 246, 244), c(255, 255, 255), c(24, 24, 28), c(24, 24, 28)),
            Scheme::Violet => build(c(16, 10, 28), c(24, 16, 40), c(232, 224, 255), c(178, 140, 255)),
            Scheme::Pink => build(c(28, 10, 20), c(40, 16, 30), c(255, 228, 240), c(255, 128, 180)),
            Scheme::Blue => build(c(6, 14, 28), c(10, 22, 42), c(222, 236, 255), c(90, 170, 255)),
            Scheme::Green => build(c(6, 20, 14), c(10, 30, 20), c(222, 255, 236), c(100, 220, 150)),
            Scheme::Custom => build(rgb(custom.background), rgb(custom.panel), rgb(custom.text), rgb(custom.accent)),
        }
    }
}
