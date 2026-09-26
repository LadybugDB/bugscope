//! Theme — the same theme disktree uses.
//!
//! disktree builds on `gpui-omarchy`, which resolves to Omarchy's system
//! theme when present and otherwise follows the OS light/dark setting with
//! two built-in palettes: **Tokyo Night** (dark) and **Flexoki Light**
//! (light). `gpui-omarchy` tracks a much newer GPUI than this crate, so the
//! two palettes are vendored here verbatim (`Theme::tokyo_night`,
//! `Theme::flexoki_light` in gpui-omarchy 0.1.3 `src/theme.rs`), selected by
//! `window.appearance()` exactly like disktree's `appearance.rs`.
//!
//! Node colours follow disktree's `palette.rs` idea: one muted hue per
//! category at a single saturation/lightness level, with amber (`warning`)
//! kept apart for selection, the primary action, and focus.

use gpui::{Hsla, Window, WindowAppearance, rgb};

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub background: Hsla,
    pub surface: Hsla,
    pub inset: Hsla,
    pub foreground: Hsla,
    pub secondary: Hsla,
    pub bright: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub selection: Hsla,
    pub border: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    pub success: Hsla,
}

impl Theme {
    pub fn tokyo_night() -> Self {
        Self {
            dark: true,
            background: rgb(0x1a1b26).into(),
            surface: rgb(0x24283b).into(),
            inset: rgb(0x13141c).into(),
            foreground: rgb(0xa9b1d6).into(),
            secondary: rgb(0xa9b1d6).into(),
            bright: rgb(0xc0caf5).into(),
            accent: rgb(0x7aa2f7).into(),
            on_accent: rgb(0x13141c).into(),
            selection: rgb(0x292e42).into(),
            border: rgb(0x414868).into(),
            danger: rgb(0xf7768e).into(),
            warning: rgb(0xe0af68).into(),
            success: rgb(0x9ece6a).into(),
        }
    }

    pub fn flexoki_light() -> Self {
        Self {
            dark: false,
            background: rgb(0xfffcf0).into(),
            surface: rgb(0xf2f0e5).into(),
            inset: rgb(0xe6e4d9).into(),
            foreground: rgb(0x100f0f).into(),
            secondary: rgb(0x575653).into(),
            bright: rgb(0x100f0f).into(),
            accent: rgb(0x205ea6).into(),
            on_accent: rgb(0xfffcf0).into(),
            selection: rgb(0xdad8ce).into(),
            border: rgb(0xb7b5ac).into(),
            danger: rgb(0xaf3029).into(),
            warning: rgb(0x855b00).into(),
            success: rgb(0x526600).into(),
        }
    }

    /// disktree `appearance.rs`: follow the system appearance.
    pub fn current(window: &Window) -> Self {
        match window.appearance() {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::flexoki_light(),
            _ => Self::tokyo_night(),
        }
    }
}

/// disktree `palette.rs` category hues, shared by both appearances.
const CATEGORY_HUES: [(f32, f32); 9] = [
    (0.605, 1.0), // code — blue
    (0.065, 1.0), // agent scratch — orange
    (0.415, 1.0), // toolchain — teal
    (0.535, 1.0), // synced — cyan
    (0.955, 1.0), // git — red
    (0.745, 1.0), // media — violet
    (0.125, 0.95), // cache — amber
    (0.6, 0.18),  // documents — near-neutral
    (0.6, 0.08),  // other — neutral
];

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.fract() * 6.0 + 6.0) % 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

/// Node-disc colour: the saturated per-hue accent (the strip over a
/// top-level directory in disktree), so small discs stay readable.
pub fn node_color(theme: &Theme, slot: usize) -> Hsla {
    let (h, chroma) = CATEGORY_HUES[slot % CATEGORY_HUES.len()];
    let (s, l) = if theme.dark {
        (0.42 * chroma, 0.52)
    } else {
        (0.45 * chroma, 0.46)
    };
    let (r, g, b) = hsl_to_rgb(h, s, l);
    let hex = ((r.clamp(0.0, 1.0) * 255.0) as u32) << 16
        | ((g.clamp(0.0, 1.0) * 255.0) as u32) << 8
        | (b.clamp(0.0, 1.0) * 255.0) as u32;
    rgb(hex).into()
}

/// Edge colour: the theme border, quiet like disktree's nesting fills.
pub fn edge_color(theme: &Theme) -> Hsla {
    theme.border
}

/// Selection / hover / primary action — disktree's kept-apart amber.
pub fn highlight(theme: &Theme) -> Hsla {
    theme.warning
}
