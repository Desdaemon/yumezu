//! The thumbnail atlas: one image carrying a picture of every world, packed by `tools/atlas`.
//!
//! One image rather than one texture per world, because the nodes are drawn as a single instanced
//! mesh: fifteen hundred textures would be fifteen hundred draw calls, whereas an atlas is one,
//! with each instance carrying the corner of it that its own world sits in.
//!
//! A world's cell is its index in the dump, which is also its node index, so nothing has to be
//! carried alongside the atlas to say which picture belongs to which world. What the two sides do
//! have to agree on is [`CELL`], and [`cells`] checks that they still do rather than trusting it.

use three_d::{egui, renderer::*};

use super::fetch;

/// Where the atlas is served from: alongside the page on the web, under the working directory
/// natively, and inside the apk on Android, which the same relative path reaches on all three.
const PATH: &str = "static/thumbnails.jpg";
/// Size of one thumbnail in the atlas, in texels. Must match `tools/atlas`, which writes it.
pub const CELL: [u32; 2] = [64, 48];
/// How much wider a thumbnail is than it is tall, which is the shape a node is drawn in.
pub const ASPECT: f32 = CELL[0] as f32 / CELL[1] as f32;
/// How many mipmaps the atlas keeps.
///
/// Nodes are drawn only a few pixels wide until the view comes in on them, and minifying an
/// unmipmapped texture that far makes the thumbnails crawl as the layout moves. Capped, because a
/// mip texel averages a square of the atlas without knowing where one world's picture stops: at
/// this level a texel spans 16 of the atlas's own, which still divides both sides of a cell, so
/// no world ends up sampling its neighbour's picture. A full chain would end up averaging the
/// whole atlas into one texel.
const MIP_LEVELS: u32 = 5;

/// Starts loading the atlas. See [`fetch`].
///
/// `None` if it cannot be had, which is not fatal: the app draws the graph without pictures. That
/// is also the state a fresh checkout is in until `just thumbnails` has been run.
pub fn load() -> fetch::Pending<Option<CpuTexture>> {
    fetch::spawn(async {
        match read().await {
            Ok(bytes) => match three_d_asset::io::deserialize::<CpuTexture>(PATH, bytes) {
                Ok(atlas) => Some(CpuTexture {
                    mipmap: Some(Mipmap {
                        max_levels: MIP_LEVELS,
                        ..Default::default()
                    }),
                    // A cell reaches the edge of the atlas, so a sample that falls off it has to
                    // be pinned to the edge rather than wrapped around to the far side.
                    wrap_s: Wrapping::ClampToEdge,
                    wrap_t: Wrapping::ClampToEdge,
                    ..atlas
                }),
                Err(error) => {
                    log::warn!("{PATH} is not an image: {error}");
                    None
                }
            },
            Err(error) => {
                log::warn!("no world thumbnails: {error}");
                None
            }
        }
    })
}

/// The atlas's bytes, however this platform stores them.
///
/// A relative path off the working directory natively and a request beside the page on the web,
/// both of which the asset loader already knows how to tell apart on its own.
#[cfg(not(target_os = "android"))]
async fn read() -> Result<Vec<u8>, String> {
    let mut assets = three_d_asset::io::load_async(&[PATH])
        .await
        .map_err(|error| error.to_string())?;
    assets.remove(PATH).map_err(|error| error.to_string())
}

/// An apk holds its assets compressed inside itself rather than as files, so there is no path to
/// hand the loader and nothing to await: the framework unpacks the whole of one on demand, and
/// only the manager it is asked through has to be reached for. See `app`'s `ANDROID`.
#[cfg(target_os = "android")]
async fn read() -> Result<Vec<u8>, String> {
    use std::io::Read as _;

    let manager = super::ANDROID
        .get()
        .ok_or("the framework's handle was never passed on")?
        .asset_manager();
    let name = std::ffi::CString::new(PATH).unwrap();
    let mut asset = manager
        .open(&name)
        .ok_or_else(|| format!("{PATH} is not in the apk"))?;
    let mut bytes = Vec::new();
    asset
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Per world, the uv transform that lands its quad on its own cell of the atlas.
///
/// `None` if the atlas cannot hold the worlds asked of it, which means it was packed against a
/// different [`CELL`] or a different dump: sampling it anyway would give every world a picture of
/// somewhere else, which is worse than giving it none.
pub fn cells(worlds: usize, atlas: &CpuTexture) -> Option<Vec<Mat3>> {
    let (columns, rows) = grid(worlds, atlas)?;
    let cell = vec2(1.0 / columns as f32, 1.0 / rows as f32);
    Some(
        (0..worlds)
            .map(|world| {
                let (column, row) = (world as u32 % columns, world as u32 / columns);
                // Rows are counted from the bottom of the atlas, because the uv coordinates this
                // transform is composed with are already flipped in v: three-d builds a mesh's uv
                // buffer as `1 - v` (see its `renderer::geometry` docs), so a quad's top edge
                // arrives here as v = 1. Only the offset turns over; the direction within a cell
                // is right as it stands, which is why the pictures are not upside down.
                let row = rows - 1 - row;
                Mat3::from_translation(vec2(column as f32 * cell.x, row as f32 * cell.y))
                    * Mat3::from_nonuniform_scale(cell.x, cell.y)
            })
            .collect(),
    )
}

/// How the atlas divides into cells, if it divides into enough of them for `worlds`.
///
/// `None` if it does not, which means it was packed against a different [`CELL`] or a different
/// dump: sampling it anyway would give every world a picture of somewhere else.
fn grid(worlds: usize, atlas: &CpuTexture) -> Option<(u32, u32)> {
    let (columns, rows) = (atlas.width / CELL[0], atlas.height / CELL[1]);
    if columns * CELL[0] != atlas.width
        || rows * CELL[1] != atlas.height
        || ((columns * rows) as usize) < worlds
    {
        log::warn!(
            "{PATH} is {}x{}, which is not {worlds} cells of {}x{}",
            atlas.width,
            atlas.height,
            CELL[0],
            CELL[1],
        );
        return None;
    }
    Some((columns, rows))
}

/// A decoded picture as egui holds them, or `None` for one stored in a format it has no pixel
/// for.
///
/// The one place a [`CpuTexture`] is turned into something egui can show, which the atlas needs
/// for its [`Sheet`] and the map window needs for every map it opens. See [`super::map`].
pub fn color_image(picture: &CpuTexture) -> Option<egui::ColorImage> {
    let size = [picture.width as usize, picture.height as usize];
    match &picture.data {
        TextureData::RgbU8(pixels) => Some(egui::ColorImage::from_rgb(size, pixels.as_flattened())),
        TextureData::RgbaU8(pixels) => Some(egui::ColorImage::from_rgba_unmultiplied(
            size,
            pixels.as_flattened(),
        )),
        _ => None,
    }
}

/// The atlas again, as egui holds it: what the sidebar's catalog draws its pictures out of.
///
/// A second upload of the same image, because the two sides own their textures separately — egui
/// hands out its own ids and manages its own uploads, and there is no seam between it and the
/// renderer's [`Texture2DRef`] to share one across. One extra copy of a single image, held for the
/// life of the app, against a catalog that can show what a release actually looks like.
pub struct Sheet {
    texture: egui::TextureHandle,
    columns: u32,
    rows: u32,
}

impl Sheet {
    /// `None` if the atlas cannot be read as this many cells, or is stored in a format egui has
    /// no pixel for — the same failure the renderer's own side takes, and just as survivable: the
    /// catalog lists releases without pictures.
    pub fn new(egui: &egui::Context, worlds: usize, atlas: &CpuTexture) -> Option<Self> {
        let (columns, rows) = grid(worlds, atlas)?;
        let Some(image) = color_image(atlas) else {
            log::warn!("{PATH} is not stored as bytes egui can show");
            return None;
        };
        Some(Self {
            // Linear, and no mipmaps: the cells are drawn at about their own size, so there is
            // nothing to filter between.
            texture: egui.load_texture(PATH, image, egui::TextureOptions::LINEAR),
            columns,
            rows,
        })
    }

    /// One world's picture, `height` points tall and in the shape the atlas crops to.
    ///
    /// Counted from the top left, unlike [`cells`]: egui's images are the right way up, so only
    /// the renderer has a flip to undo.
    pub fn picture(&self, world: usize, height: f32) -> egui::Image<'static> {
        let (column, row) = (world as u32 % self.columns, world as u32 / self.columns);
        let cell = egui::vec2(1.0 / self.columns as f32, 1.0 / self.rows as f32);
        let at = egui::pos2(column as f32 * cell.x, row as f32 * cell.y);
        egui::Image::new((self.texture.id(), self.texture.size_vec2()))
            .uv(egui::Rect::from_min_size(at, cell))
            .fit_to_exact_size(egui::vec2(height * ASPECT, height))
    }
}
