//! The look of RPG Maker 2000's system graphics: the default one painted in code, or one of Yume
//! 2kki's own skins sampled out of an atlas. A palette for egui's widgets, and the window every
//! panel sits in.

use super::*;

const PAINTED_TEXT: egui::Color32 = egui::Color32::WHITE;
const PAINTED_TEXT_QUIET: egui::Color32 = egui::Color32::from_rgb(0x9a, 0xa4, 0xd8);
const PAINTED_ACCENT: egui::Color32 = egui::Color32::from_rgb(0xff, 0xe0, 0x8a);
const PAINTED_TOP: egui::Color32 = egui::Color32::from_rgb(0x2b, 0x3f, 0x9e);
const PAINTED_MIDDLE: egui::Color32 = egui::Color32::from_rgb(0x16, 0x22, 0x6a);
const PAINTED_BOTTOM: egui::Color32 = egui::Color32::from_rgb(0x0c, 0x14, 0x46);
/// How far down the window its gradient reaches the middle colour.
const PAINTED_MIDDLE_AT: f32 = 0.55;
const PAINTED_EDGE: egui::Color32 = egui::Color32::from_rgb(0xd8, 0xdc, 0xf4);
/// The two lines inside the edge, light then dark, that give the border its bevel.
const PAINTED_BEVEL_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x4a, 0x5a, 0xa8);
const PAINTED_BEVEL_DARK: egui::Color32 = egui::Color32::from_rgb(0x0a, 0x0f, 0x36);
const PAINTED_WELL: egui::Color32 = egui::Color32::from_rgb(0x06, 0x0a, 0x2a);
const PAINTED_EDGE_WIDTH: f32 = 2.0;
const PAINTED_CORNER: u8 = 3;

pub(super) const WINDOW_PADDING: i8 = 10;
/// How much of the graph a window's background lets through; its border and text stay solid.
const WINDOW_SEE_THROUGH: f32 = 0.2;
const WIDGET_CORNER: u8 = 3;
/// How opaque the cursor is over a selected row, and over a hovered one.
const CURSOR_ALPHA: u8 = 72;
const CURSOR_FAINT_ALPHA: u8 = 26;
/// How far a skin's quiet text and rules lean from its text towards its background.
const QUIET_SHARE: f32 = 0.35;
const WELL_DARKENING: f32 = 0.5;

/// Packed by `tools/windowskins.py` out of forest-orb's cuts; keep the layout below in sync with it.
const ATLAS: &[u8] = include_bytes!("../../assets/windowskins.png");
const ATLAS_NAMES: &str = include_str!("../../assets/windowskins.txt");
const ATLAS_COLUMNS: usize = 11;
const ATLAS_CELL: egui::Vec2 = egui::vec2(64.0, 48.0);
const BACKGROUND_AT: egui::Vec2 = egui::vec2(0.0, 0.0);
const BORDER_AT: egui::Vec2 = egui::vec2(32.0, 0.0);
const TEXT_GRADIENT_AT: egui::Vec2 = egui::vec2(0.0, 32.0);
const ACCENT_GRADIENT_AT: egui::Vec2 = egui::vec2(48.0, 32.0);
const ARROW_UP_AT: egui::Vec2 = egui::vec2(32.0, 32.0);
const ARROW_DOWN_AT: egui::Vec2 = egui::vec2(32.0, 40.0);
const TILE_SIZE: egui::Vec2 = egui::vec2(32.0, 32.0);
const ARROW_SIZE: egui::Vec2 = egui::vec2(16.0, 8.0);
const ARROW_SCALE: f32 = 1.5;
/// Mobile GPUs interpolate UVs at half precision, enough to sample past a piece's edge into its neighbour.
const TEXEL_INSET: f32 = 0.25;
/// The row of a text gradient read as the text's one colour.
const GRADIENT_MIDDLE: f32 = 8.0;
/// How deep each side of a border is, which `border-image` calls its slice.
const BORDER_SLICE: f32 = 8.0;
const ATLAS_TEXTURE_ID: &str = "window skins";

const SKIN_ID: &str = "window skin";
const SKIN_STORED: &str = "window-skin";

/// DotGothic16 cut down to Latin; Japanese falls through to the face [`super::japanese`] serves.
const FONT: &[u8] = include_bytes!("../../assets/DotGothic16-Latin.ttf");
const FONT_NAME: &str = "DotGothic16";

/// Urotsuki walking, the four frames side by side, as the explorer shows while it loads.
const WALK: &[u8] = include_bytes!("../../assets/urowalk.png");
const WALK_FRAMES: usize = 4;
const WALK_FRAME_SECONDS: f64 = 0.15;
/// Whole pixels, so the sprite stays sharp.
const WALK_SCALE: f32 = 2.0;
const WALK_TEXTURE_ID: &str = "walking sprite";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Skin {
    /// The default system graphic's blue, painted rather than sampled.
    Painted,
    /// One of Yume 2kki's skins, by its place in the atlas.
    Atlas(usize),
}

impl Skin {
    pub(super) fn remembered() -> Self {
        store::read(SKIN_STORED)
            .and_then(|name| ATLAS_NAMES.lines().position(|line| line == name))
            .map_or(Self::Painted, Self::Atlas)
    }

    pub(super) fn remember(self) {
        store::write(SKIN_STORED, self.atlas_name());
    }

    pub(super) fn every() -> impl Iterator<Item = Self> {
        std::iter::once(Self::Painted).chain((0..ATLAS_NAMES.lines().count()).map(Self::Atlas))
    }

    fn atlas_name(self) -> Option<&'static str> {
        match self {
            Self::Painted => None,
            Self::Atlas(index) => ATLAS_NAMES.lines().nth(index),
        }
    }
}

/// The colours egui's widgets are drawn in, to match a skin.
struct Palette {
    text: egui::Color32,
    quiet: egui::Color32,
    accent: egui::Color32,
    base: egui::Color32,
    well: egui::Color32,
    edge: egui::Color32,
    rule: egui::Color32,
}

impl Palette {
    fn of(skin: Skin) -> Self {
        match skin {
            Skin::Painted => Self {
                text: PAINTED_TEXT,
                quiet: PAINTED_TEXT_QUIET,
                accent: PAINTED_ACCENT,
                base: PAINTED_MIDDLE,
                well: PAINTED_WELL,
                edge: PAINTED_EDGE,
                rule: PAINTED_BEVEL_LIGHT,
            },
            Skin::Atlas(index) => Self::sampled(index),
        }
    }

    /// Read off the skin's pixels the way forest-orb reads them: the text from the middle of its
    /// gradient, the background from three texels averaged.
    fn sampled(index: usize) -> Self {
        let pixels = atlas_pixels();
        let cell = cell_origin(index);
        let at = |offset: egui::Vec2| {
            let texel = cell + offset;
            let [r, g, b, _] = pixels.get_pixel(texel.x as u32, texel.y as u32).0;
            egui::Color32::from_rgb(r, g, b)
        };
        let text = at(TEXT_GRADIENT_AT + egui::vec2(0.0, GRADIENT_MIDDLE));
        let accent = at(ACCENT_GRADIENT_AT + egui::vec2(0.0, GRADIENT_MIDDLE));
        let samples = [0.0, 4.0, 8.0].map(|step| at(BACKGROUND_AT + egui::Vec2::splat(step)));
        let channel = |pick: fn(egui::Color32) -> u8| {
            (samples
                .iter()
                .map(|&color| u32::from(pick(color)))
                .sum::<u32>()
                / 3) as u8
        };
        let base =
            egui::Color32::from_rgb(channel(|c| c.r()), channel(|c| c.g()), channel(|c| c.b()));
        let toward_base = text.lerp_to_gamma(base, QUIET_SHARE);
        Self {
            text,
            quiet: toward_base,
            accent,
            base,
            well: base.lerp_to_gamma(egui::Color32::BLACK, WELL_DARKENING),
            edge: text,
            rule: toward_base,
        }
    }

    fn visuals(&self) -> egui::Visuals {
        let mut visuals = egui::Visuals::dark();
        visuals.weak_text_color = Some(self.quiet);
        visuals.hyperlink_color = self.accent;
        visuals.extreme_bg_color = self.well;
        visuals.faint_bg_color = self.base;
        visuals.window_fill = self.base;
        visuals.panel_fill = self.base;
        visuals.window_stroke = egui::Stroke::new(PAINTED_EDGE_WIDTH, self.edge);
        visuals.window_corner_radius = egui::CornerRadius::same(PAINTED_CORNER);
        visuals.menu_corner_radius = egui::CornerRadius::same(PAINTED_CORNER);
        visuals.window_shadow = egui::Shadow::NONE;
        visuals.popup_shadow = egui::Shadow::NONE;
        let [r, g, b, _] = self.text.to_array();
        let cursor = |alpha: u8| egui::Color32::from_rgba_unmultiplied(r, g, b, alpha);
        visuals.selection = egui::style::Selection {
            bg_fill: cursor(CURSOR_ALPHA),
            stroke: egui::Stroke::new(1.0, self.text),
        };
        let widget = |fill: egui::Color32, edge: egui::Stroke| egui::style::WidgetVisuals {
            bg_fill: fill,
            weak_bg_fill: fill,
            bg_stroke: edge,
            corner_radius: egui::CornerRadius::same(WIDGET_CORNER),
            fg_stroke: egui::Stroke::new(1.0, self.text),
            // egui takes the stroke out of a button's padding even where it draws no frame, so a
            // hovered row would grow by it; expanding by the stroke keeps every state one size.
            expansion: edge.width,
        };
        let rule = egui::Stroke::new(1.0, self.rule);
        visuals.widgets = egui::style::Widgets {
            noninteractive: widget(egui::Color32::TRANSPARENT, rule),
            inactive: widget(self.well, rule),
            hovered: widget(
                cursor(CURSOR_FAINT_ALPHA),
                egui::Stroke::new(1.0, self.text),
            ),
            active: widget(cursor(CURSOR_ALPHA), egui::Stroke::new(2.0, self.text)),
            open: widget(
                cursor(CURSOR_FAINT_ALPHA),
                egui::Stroke::new(1.0, self.text),
            ),
        };
        visuals
    }
}

pub(super) fn install_font(ctx: &egui::Context) {
    ctx.add_font(egui::epaint::text::FontInsert::new(
        FONT_NAME,
        egui::FontData::from_static(FONT),
        vec![egui::epaint::text::InsertFontFamily {
            family: egui::FontFamily::Proportional,
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
}

/// Dark whatever the system prefers: every skin picks its own colours.
pub(super) fn apply_skin(ctx: &egui::Context, skin: Skin) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, Palette::of(skin).visuals());
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(SKIN_ID), skin));
}

fn applied_skin(ctx: &egui::Context) -> Skin {
    ctx.data(|data| data.get_temp(egui::Id::new(SKIN_ID)))
        .unwrap_or(Skin::Painted)
}

/// Lays `add_contents` out inside a window, painted behind it once its size is known.
pub(super) fn window<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let inner = egui::Frame::NONE
        .inner_margin(WINDOW_PADDING)
        .show(ui, add_contents);
    let skin = applied_skin(ui.ctx());
    let background = egui::Color32::WHITE.gamma_multiply(1.0 - WINDOW_SEE_THROUGH);
    ui.painter().set(
        backdrop,
        window_shape(ui.ctx(), skin, inner.response.rect, background),
    );
    inner
}

#[derive(Clone, Copy)]
pub(super) enum Arrow {
    Up,
    Down,
}

impl Arrow {
    fn atlas_at(self) -> egui::Vec2 {
        match self {
            Arrow::Up => ARROW_UP_AT,
            Arrow::Down => ARROW_DOWN_AT,
        }
    }
}

/// The applied skin's own arrow, `width` across; the painted skin has none.
pub(super) fn skin_arrow(
    ctx: &egui::Context,
    arrow: Arrow,
    width: f32,
) -> Option<egui::Image<'static>> {
    let index = match applied_skin(ctx) {
        Skin::Painted => return None,
        Skin::Atlas(index) => index,
    };
    let texture = atlas_texture(ctx);
    let uv = texel_uv(
        cell_origin(index) + arrow.atlas_at(),
        ARROW_SIZE,
        texture.size_vec2(),
    );
    let size = ARROW_SIZE * (width / ARROW_SIZE.x);
    Some(egui::Image::new((texture.id(), size)).uv(uv))
}

/// A skin drawn small enough to pick from, with its arrows in it as forest-orb shows it.
pub(super) fn skin_button(
    ui: &mut egui::Ui,
    skin: Skin,
    chosen: bool,
    size: egui::Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    painter.add(window_shape(ui.ctx(), skin, rect, egui::Color32::WHITE));
    if let Skin::Atlas(index) = skin {
        let texture = atlas_texture(ui.ctx());
        let cell = cell_origin(index);
        let mut arrows = egui::Mesh::with_texture(texture.id());
        let arrow = ARROW_SIZE * ARROW_SCALE;
        let up = egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, arrow.y), arrow);
        let down = egui::Rect::from_center_size(rect.center() + egui::vec2(0.0, arrow.y), arrow);
        let atlas = texture.size_vec2();
        arrows.add_rect_with_uv(
            up,
            texel_uv(cell + Arrow::Up.atlas_at(), ARROW_SIZE, atlas),
            egui::Color32::WHITE,
        );
        arrows.add_rect_with_uv(
            down,
            texel_uv(cell + Arrow::Down.atlas_at(), ARROW_SIZE, atlas),
            egui::Color32::WHITE,
        );
        painter.add(arrows);
    }
    let edge = match (chosen, response.hovered()) {
        (true, _) => Some(2.0),
        (false, true) => Some(1.0),
        (false, false) => None,
    };
    if let Some(width) = edge {
        let stroke = egui::Stroke::new(width, ui.visuals().selection.stroke.color);
        painter.rect_stroke(
            rect.expand(2.0),
            WIDGET_CORNER,
            stroke,
            egui::StrokeKind::Outside,
        );
    }
    let named = skin
        .atlas_name()
        .map_or_else(|| t!("system-theme-painted"), str::to_owned);
    response.on_hover_text(named)
}

/// `background` tints the fill alone, never the border.
fn window_shape(
    ctx: &egui::Context,
    skin: Skin,
    rect: egui::Rect,
    background: egui::Color32,
) -> egui::Shape {
    match skin {
        Skin::Painted => painted_window_shape(rect, background),
        Skin::Atlas(index) => atlas_window_shape(ctx, index, rect, background),
    }
}

fn painted_window_shape(rect: egui::Rect, background: egui::Color32) -> egui::Shape {
    let fill = rect.shrink(1.0);
    let middle = egui::lerp(fill.top()..=fill.bottom(), PAINTED_MIDDLE_AT);
    let upper = egui::Rect::from_x_y_ranges(fill.x_range(), fill.top()..=middle);
    let lower = egui::Rect::from_x_y_ranges(fill.x_range(), middle..=fill.bottom());
    let mut gradient = egui::Mesh::default();
    let (top, middle, bottom) = (
        PAINTED_TOP * background,
        PAINTED_MIDDLE * background,
        PAINTED_BOTTOM * background,
    );
    vertical_gradient(&mut gradient, upper, top, middle);
    vertical_gradient(&mut gradient, lower, middle, bottom);
    let edge = |inset: f32, width: f32, color: egui::Color32| {
        egui::Shape::rect_stroke(
            rect.shrink(inset),
            egui::CornerRadius::same(PAINTED_CORNER),
            egui::Stroke::new(width, color),
            egui::StrokeKind::Inside,
        )
    };
    egui::Shape::Vec(vec![
        egui::Shape::mesh(gradient),
        edge(0.0, PAINTED_EDGE_WIDTH, PAINTED_EDGE),
        edge(PAINTED_EDGE_WIDTH, 1.0, PAINTED_BEVEL_LIGHT),
        edge(PAINTED_EDGE_WIDTH + 1.0, 1.0, PAINTED_BEVEL_DARK),
    ])
}

fn vertical_gradient(
    mesh: &mut egui::Mesh,
    rect: egui::Rect,
    top: egui::Color32,
    bottom: egui::Color32,
) {
    let first = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(first, first + 1, first + 2);
    mesh.add_triangle(first + 1, first + 2, first + 3);
}

/// The background tiled under the border, the border cut in nine as CSS's `border-image` cuts it:
/// corners whole, edges repeated.
fn atlas_window_shape(
    ctx: &egui::Context,
    index: usize,
    rect: egui::Rect,
    background: egui::Color32,
) -> egui::Shape {
    let texture = atlas_texture(ctx);
    let atlas = texture.size_vec2();
    let cell = cell_origin(index);
    let mut mesh = egui::Mesh::with_texture(texture.id());
    let background_tile = egui::Rect::from_min_size((cell + BACKGROUND_AT).to_pos2(), TILE_SIZE);
    tile(
        &mut mesh,
        rect.shrink(BORDER_SLICE / 2.0),
        background_tile,
        atlas,
        background,
    );
    let border = egui::Rect::from_min_size((cell + BORDER_AT).to_pos2(), TILE_SIZE);
    let cuts = |from: f32, to: f32| [from, from + BORDER_SLICE, to - BORDER_SLICE, to];
    let (across, down) = (
        cuts(rect.left(), rect.right()),
        cuts(rect.top(), rect.bottom()),
    );
    let (source_across, source_down) = (
        cuts(border.left(), border.right()),
        cuts(border.top(), border.bottom()),
    );
    for row in 0..3 {
        for column in 0..3 {
            if (row, column) == (1, 1) {
                continue;
            }
            let piece = |xs: [f32; 4], ys: [f32; 4]| {
                egui::Rect::from_min_max(
                    egui::pos2(xs[column], ys[row]),
                    egui::pos2(xs[column + 1], ys[row + 1]),
                )
            };
            tile(
                &mut mesh,
                piece(across, down),
                piece(source_across, source_down),
                atlas,
                egui::Color32::WHITE,
            );
        }
    }
    egui::Shape::mesh(mesh)
}

/// Repeats `source`, in texels, across `area` at a texel a point, cutting the last row and column
/// short rather than squeezing them.
fn tile(
    mesh: &mut egui::Mesh,
    area: egui::Rect,
    source: egui::Rect,
    atlas: egui::Vec2,
    tint: egui::Color32,
) {
    let mut y = area.top();
    while y < area.bottom() {
        let height = (area.bottom() - y).min(source.height());
        let mut x = area.left();
        while x < area.right() {
            let width = (area.right() - x).min(source.width());
            let size = egui::vec2(width, height);
            let place = egui::Rect::from_min_size(egui::pos2(x, y), size);
            let uv = texel_uv(source.min.to_vec2(), size, atlas);
            mesh.add_rect_with_uv(place, uv, tint);
            x += source.width();
        }
        y += source.height();
    }
}

fn texel_uv(at: egui::Vec2, size: egui::Vec2, atlas: egui::Vec2) -> egui::Rect {
    egui::Rect::from_min_max(
        ((at + egui::Vec2::splat(TEXEL_INSET)) / atlas).to_pos2(),
        ((at + size - egui::Vec2::splat(TEXEL_INSET)) / atlas).to_pos2(),
    )
}

fn cell_origin(index: usize) -> egui::Vec2 {
    let (column, row) = (index % ATLAS_COLUMNS, index / ATLAS_COLUMNS);
    egui::vec2(column as f32, row as f32) * ATLAS_CELL
}

fn atlas_pixels() -> &'static image::RgbaImage {
    static PIXELS: std::sync::OnceLock<image::RgbaImage> = std::sync::OnceLock::new();
    PIXELS.get_or_init(|| {
        image::load_from_memory_with_format(ATLAS, image::ImageFormat::Png)
            .expect("the window skin atlas compiled in is a PNG")
            .to_rgba8()
    })
}

fn atlas_texture(ctx: &egui::Context) -> egui::TextureHandle {
    texture_once(ctx, ATLAS_TEXTURE_ID, || {
        let pixels = atlas_pixels();
        let size = [pixels.width() as usize, pixels.height() as usize];
        egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_raw())
    })
}

/// Uploaded the first time it is asked for and kept in egui's memory for every frame after.
fn texture_once(
    ctx: &egui::Context,
    name: &str,
    decode: impl FnOnce() -> egui::ColorImage,
) -> egui::TextureHandle {
    let id = egui::Id::new(name);
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return texture;
    }
    let texture = ctx.load_texture(name, decode(), egui::TextureOptions::NEAREST);
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

/// One frame of the walk, chosen by the clock, and a repaint booked for the next.
pub(super) fn walking_sprite(ui: &mut egui::Ui) -> egui::Response {
    let texture = texture_once(ui.ctx(), WALK_TEXTURE_ID, || {
        let strip = image::load_from_memory_with_format(WALK, image::ImageFormat::Png)
            .expect("the walking sprite compiled in is a PNG")
            .to_rgba8();
        let size = [strip.width() as usize, strip.height() as usize];
        egui::ColorImage::from_rgba_unmultiplied(size, strip.as_raw())
    });
    let size = texture.size_vec2();
    let frame_width = 1.0 / WALK_FRAMES as f32;
    let frame = (ui.input(|input| input.time) / WALK_FRAME_SECONDS) as usize % WALK_FRAMES;
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f64(WALK_FRAME_SECONDS));
    let uv = egui::Rect::from_min_size(
        egui::pos2(frame as f32 * frame_width, 0.0),
        egui::vec2(frame_width, 1.0),
    );
    let shown = egui::vec2(size.x * frame_width, size.y) * WALK_SCALE;
    ui.add(egui::Image::new((texture.id(), shown)).uv(uv))
}
