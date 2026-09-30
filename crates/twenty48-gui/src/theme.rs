//! Window colours and widget styling.
//!
//! The board has a fixed palette it cannot give up without stopping looking
//! like 2048, so the rest of the window is built out of the same colours.
//! Left on egui's defaults the board reads as an image pasted onto a grey
//! dialog.

use eframe::egui::{
    Align2, Color32, Context, CornerRadius, FontId, RichText, Sense, Stroke, Style, Ui, Visuals,
    vec2,
};

/// Page background, the same off-white the original game uses.
pub const CANVAS: Color32 = Color32::from_rgb(0xfa, 0xf8, 0xef);
/// Body text.
pub const INK: Color32 = Color32::from_rgb(0x77, 0x6e, 0x65);
/// Captions and hints.
pub const MUTED: Color32 = Color32::from_rgb(0xa2, 0x98, 0x8c);
/// Text on a dark fill.
pub const LIGHT: Color32 = Color32::from_rgb(0xf9, 0xf6, 0xf2);
/// Buttons and the filled part of a slider.
pub const ACCENT: Color32 = Color32::from_rgb(0x8f, 0x7a, 0x66);
/// Game over.
pub const DANGER: Color32 = Color32::from_rgb(0x9c, 0x42, 0x42);

const ACCENT_HOVER: Color32 = Color32::from_rgb(0xa2, 0x8b, 0x74);
const ACCENT_ACTIVE: Color32 = Color32::from_rgb(0x79, 0x66, 0x55);
const CHIP: Color32 = Color32::from_rgb(0xbb, 0xad, 0xa0);
const CHIP_CAPTION: Color32 = Color32::from_rgb(0xee, 0xe4, 0xda);
const TROUGH: Color32 = Color32::from_rgb(0xe4, 0xdb, 0xcd);
const FIELD: Color32 = Color32::from_rgb(0xf2, 0xed, 0xe1);
const RULE: Color32 = Color32::from_rgb(0xe8, 0xe0, 0xd3);

const RADIUS: CornerRadius = CornerRadius::same(6);

pub fn apply(ctx: &Context) {
    let mut visuals = Visuals::light();
    visuals.panel_fill = CANVAS;
    visuals.window_fill = CANVAS;
    visuals.extreme_bg_color = FIELD;
    visuals.faint_bg_color = FIELD;
    // Draws the part of a slider left of the handle, which is the only cue
    // for where a logarithmic slider actually sits.
    visuals.slider_trailing_fill = true;
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, LIGHT);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, RULE);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, INK);
    widgets.noninteractive.corner_radius = RADIUS;

    for (state, fill) in [
        (&mut widgets.inactive, ACCENT),
        (&mut widgets.hovered, ACCENT_HOVER),
        (&mut widgets.active, ACCENT_ACTIVE),
        (&mut widgets.open, ACCENT_HOVER),
    ] {
        // `weak_bg_fill` paints buttons, `bg_fill` the slider handle and rail.
        state.weak_bg_fill = fill;
        state.bg_fill = TROUGH;
        state.bg_stroke = Stroke::new(2.0, fill);
        state.fg_stroke = Stroke::new(1.0, LIGHT);
        state.corner_radius = RADIUS;
        state.expansion = 0.0;
    }

    let mut style = Style {
        visuals,
        ..Style::default()
    };
    style.spacing.item_spacing = eframe::egui::vec2(8.0, 7.0);
    style.spacing.button_padding = eframe::egui::vec2(12.0, 5.0);
    style.spacing.slider_width = 130.0;
    style.spacing.slider_rail_height = 6.0;

    // Pinned rather than following the system, because the palette is the
    // game's and there is no dark counterpart to it.
    ctx.set_theme(eframe::egui::ThemePreference::Light);
    ctx.set_style_of(eframe::egui::Theme::Light, style.clone());
    ctx.set_style_of(eframe::egui::Theme::Dark, style);
}

/// A caption over a number, on a tile-coloured chip. The score readout in the
/// original game, which is the obvious thing for the header to echo.
/// Outside width of a [`chip`], so a caller can right-align a row of them.
pub const CHIP_WIDTH: f32 = 96.0;
/// Outside height of a [`chip`].
pub const CHIP_HEIGHT: f32 = 46.0;

pub fn chip(ui: &mut Ui, caption: &str, value: &str) {
    // Laid out by hand. Nesting a `Frame` inside the header row inherits that
    // row's centre cross-alignment, and a chip then lands at a different
    // height depending on how much of the row is left, which steps a row of
    // them down the page.
    let (rect, _) = ui.allocate_exact_size(vec2(CHIP_WIDTH, CHIP_HEIGHT), Sense::hover());
    ui.painter().rect_filled(rect, RADIUS, CHIP);

    let font = |size| FontId::proportional(size);
    ui.painter().text(
        rect.center_top() + vec2(0.0, 7.0),
        Align2::CENTER_TOP,
        caption,
        font(10.0),
        CHIP_CAPTION,
    );
    ui.painter().text(
        rect.center_bottom() + vec2(0.0, -8.0),
        Align2::CENTER_BOTTOM,
        value,
        font(19.0),
        LIGHT,
    );
}

/// The heading above a group of controls.
pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).size(11.0).strong().color(MUTED));
    ui.add_space(2.0);
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(MUTED));
}

/// A name on the left, its value against the right edge.
pub fn readout(ui: &mut Ui, name: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).size(12.0).color(MUTED));
        ui.with_layout(
            eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
            |ui| ui.label(RichText::new(value).size(12.0).strong().color(INK)),
        );
    });
}

pub fn rule(ui: &mut Ui) {
    ui.add_space(8.0);
    ui.separator();
}
