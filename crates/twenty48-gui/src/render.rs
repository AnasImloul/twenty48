//! Drawing the board.

use eframe::egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Response, Stroke, Ui, Vec2, vec2,
};

use twenty48::{Board, Direction, Spawn};

const BACKGROUND: Color32 = Color32::from_rgb(0xbb, 0xad, 0xa0);
const EMPTY: Color32 = Color32::from_rgb(0xcd, 0xc1, 0xb4);
const DARK_TEXT: Color32 = Color32::from_rgb(0x77, 0x6e, 0x65);
const LIGHT_TEXT: Color32 = Color32::from_rgb(0xf9, 0xf6, 0xf2);

/// The canonical 2048 colours up to 2048, then one colour for everything
/// above, which the agent spends most of its game in.
fn tile_color(value: u16) -> Color32 {
    match value {
        2 => Color32::from_rgb(0xee, 0xe4, 0xda),
        4 => Color32::from_rgb(0xed, 0xe0, 0xc8),
        8 => Color32::from_rgb(0xf2, 0xb1, 0x79),
        16 => Color32::from_rgb(0xf5, 0x95, 0x63),
        32 => Color32::from_rgb(0xf6, 0x7c, 0x5f),
        64 => Color32::from_rgb(0xf6, 0x5e, 0x3b),
        128 => Color32::from_rgb(0xed, 0xcf, 0x72),
        256 => Color32::from_rgb(0xed, 0xcc, 0x61),
        512 => Color32::from_rgb(0xed, 0xc8, 0x50),
        1024 => Color32::from_rgb(0xed, 0xc5, 0x3f),
        2048 => Color32::from_rgb(0xed, 0xc2, 0x2e),
        _ => Color32::from_rgb(0x3c, 0x3a, 0x32),
    }
}

fn text_color(value: u16) -> Color32 {
    if value <= 4 { DARK_TEXT } else { LIGHT_TEXT }
}

/// Shrinks the label so five digits fit in the same cell two digits do.
fn font_size(value: u16, cell: f32) -> f32 {
    let digits = value.to_string().len();
    let scale = match digits {
        1 | 2 => 0.45,
        3 => 0.38,
        4 => 0.30,
        _ => 0.24,
    };
    cell * scale
}

pub struct Highlight {
    /// The tile the engine just placed, drawn growing into place.
    pub spawn: Option<Spawn>,
    /// How far through that animation we are, 0 to 1.
    pub age: f32,
    /// A move the agent suggests, drawn as an arrow over the board.
    pub hint: Option<Direction>,
}

pub fn board(ui: &mut Ui, size: f32, board: Board, highlight: &Highlight) -> Response {
    let (response, painter) = ui.allocate_painter(Vec2::splat(size), eframe::egui::Sense::hover());
    let area = response.rect;

    let gap = size * 0.025;
    let cell = (size - gap * 5.0) / 4.0;
    painter.rect_filled(area, size * 0.02, BACKGROUND);

    let tiles = board.tiles();
    for (r, row) in tiles.iter().enumerate() {
        for (c, &value) in row.iter().enumerate() {
            let origin =
                area.min + vec2(gap + c as f32 * (cell + gap), gap + r as f32 * (cell + gap));
            let full = Rect::from_min_size(origin, Vec2::splat(cell));

            if value == 0 {
                painter.rect_filled(full, cell * 0.06, EMPTY);
                continue;
            }

            // A freshly spawned tile grows from 30% to full size, which is
            // enough to catch the eye without the board feeling sluggish.
            let fresh = highlight
                .spawn
                .is_some_and(|spawn| spawn.row == r && spawn.col == c);
            let scale = if fresh {
                0.3 + 0.7 * highlight.age.clamp(0.0, 1.0)
            } else {
                1.0
            };
            let rect = Rect::from_center_size(full.center(), Vec2::splat(cell * scale));

            painter.rect_filled(rect, cell * 0.06, tile_color(value));
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                value,
                FontId::proportional(font_size(value, cell * scale)),
                text_color(value),
            );
        }
    }

    if let Some(dir) = highlight.hint {
        arrow(&painter, area.center(), size * 0.16, dir);
    }

    response
}

/// The agent's suggestion, as a translucent arrow over the middle of the
/// board. Translucent because it sits on top of tiles the player still has to
/// be able to read.
fn arrow(painter: &Painter, center: Pos2, radius: f32, dir: Direction) {
    let step = match dir {
        Direction::Left => vec2(-1.0, 0.0),
        Direction::Right => vec2(1.0, 0.0),
        Direction::Up => vec2(0.0, -1.0),
        Direction::Down => vec2(0.0, 1.0),
    };
    let side = vec2(-step.y, step.x);

    let tip = center + step * radius;
    let back = center - step * radius;
    let ink = Color32::from_rgba_unmultiplied(0x25, 0x22, 0x1e, 200);

    painter.line_segment([back, tip], Stroke::new(radius * 0.26, ink));
    painter.add(eframe::egui::Shape::convex_polygon(
        vec![
            tip + step * radius * 0.45,
            tip + side * radius * 0.42,
            tip - side * radius * 0.42,
        ],
        ink,
        Stroke::NONE,
    ));
}
