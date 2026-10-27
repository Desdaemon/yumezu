//! The thumbnail atlas: one image carrying a picture of every world, packed by `tools/atlas`.
//!
//! One image rather than a texture per world, because the nodes are drawn as a single instanced
//! mesh: fifteen hundred textures would be fifteen hundred draw calls.
//!
//! Fetched from the same host as the dump rather than shipped beside the binary, so a package does
//! not freeze the pictures at release time. See [`read`].
//!
//! A world's cell is the one the dump gives it, which the server hands out once and never moves,
//! so an atlas stays right however far the dump has moved on. A world past the end of the atlas
//! was packed after it and draws the placeholder, which is itself the grid's last cell. Both sides
//! have to agree on [`CELL`], which [`grid`] checks rather than trusts.

use three_d::renderer::*;

use super::fetch;
use super::prelude::*;

// Published by the `copy-file` links in `index.html`.
const PATH: &str = "static/thumbnails.jpg";
const UNKNOWN: &str = "static/unknown_location.png";
/// Size of one thumbnail in texels. Must match `tools/atlas`, which writes the atlas.
pub const CELL: [u32; 2] = [64, 48];
pub const ASPECT: f32 = CELL[0] as f32 / CELL[1] as f32;
/// Mipmapped because nodes are only a few pixels wide until the view comes in, and minifying that
/// far unmipmapped makes the thumbnails crawl as the layout moves.
///
/// Capped because a mip texel averages a square of the atlas without knowing where one world's
/// picture stops. At this level a texel spans 16, which still divides both sides of a cell.
const MIP_LEVELS: u32 = 5;

/// `None` is not fatal: the graph draws without pictures until one resolves, and the caller tries
/// again after a wait -- see [`super::Atlas`].
pub fn load() -> fetch::Pending<Option<CpuTexture>> {
    picture(PATH, Some(MIP_LEVELS))
}

/// Its own file as well as its own cell of the atlas: the graph draws it at every size, and the
/// atlas is a jpeg, the worst thing to put hard white edges on black through. The catalog draws
/// the cell, at about the size it is packed at.
///
/// `None` is not fatal: an unvisited world keeps its bare node.
pub fn placeholder() -> fetch::Pending<Option<CpuTexture>> {
    // Mipmapped, unlike the atlas: one picture has no neighbour to bleed in from.
    picture(UNKNOWN, None)
}

fn picture(path: &'static str, mip_levels: Option<u32>) -> fetch::Pending<Option<CpuTexture>> {
    fetch::spawn(async move {
        match read(path).await {
            Ok(bytes) => match three_d_asset::io::deserialize::<CpuTexture>(path, bytes) {
                Ok(picture) => Some(CpuTexture {
                    mipmap: Some(Mipmap {
                        max_levels: mip_levels.unwrap_or(u32::MAX),
                        ..Default::default()
                    }),
                    // Cells reach the atlas edge, so an off-edge sample must not wrap.
                    wrap_s: Wrapping::ClampToEdge,
                    wrap_t: Wrapping::ClampToEdge,
                    ..picture
                }),
                Err(error) => {
                    log::warn!("{path} is not an image: {error}");
                    None
                }
            },
            Err(error) => {
                log::warn!("no {path}: {error}");
                None
            }
        }
    })
}

/// Off the host the dump comes from, so the pictures stay as current as the worlds they belong to
/// and no package carries a copy that ages.
///
/// One implementation for every target: [`super::world::server`] is the page's own origin on wasm,
/// so the same relative path is the same URL there.
async fn read(path: &str) -> Result<Vec<u8>, String> {
    let url = format!("{}/{path}", super::world::server());
    fetch::client()
        .get(&url)
        .fetch_bytes()
        .await
        .map_err(|error| error.to_string())
}

/// Per world, the uv transform landing its quad on its cell. `of` holds `None` for a world the
/// player has not been to, which draws the placeholder.
///
/// `None` for an atlas that is not whole cells of [`CELL`], which is one packed against a
/// different cell size: sampling it would give every world a picture of somewhere else.
pub fn cells(of: &[Option<usize>], atlas: &CpuTexture) -> Option<Vec<Mat3>> {
    let (columns, rows) = grid(atlas)?;
    Some(
        of.iter()
            .map(|cell| uv(held(*cell, columns, rows), columns, rows))
            .collect(),
    )
}

fn uv(cell: usize, columns: u32, rows: u32) -> Mat3 {
    let size = vec2(1.0 / columns as f32, 1.0 / rows as f32);
    let (column, row) = (cell as u32 % columns, cell as u32 / columns);
    // three-d builds uv as `1 - v`, so rows count from the bottom; within a cell v is right.
    let row = rows - 1 - row;
    Mat3::from_translation(vec2(column as f32 * size.x, row as f32 * size.y))
        * Mat3::from_nonuniform_scale(size.x, size.y)
}

/// `cell` where this atlas holds one, and the placeholder where it does not -- a world packed
/// into no atlas this old, or one the player has not been to.
///
/// The placeholder is the last cell of the grid rather than one counted off the worlds, so neither
/// side carries a number the other could get wrong.
fn held(cell: Option<usize>, columns: u32, rows: u32) -> usize {
    let unknown = (columns * rows) as usize - 1;
    cell.filter(|cell| *cell < unknown).unwrap_or(unknown)
}

/// `None` unless the atlas is whole cells of [`CELL`].
fn grid(atlas: &CpuTexture) -> Option<(u32, u32)> {
    let (columns, rows) = (atlas.width / CELL[0], atlas.height / CELL[1]);
    if columns * CELL[0] != atlas.width || rows * CELL[1] != atlas.height || columns * rows < 2 {
        log::warn!(
            "{PATH} is {}x{}, which is not whole cells of {}x{} with one to spare for the \
             placeholder",
            atlas.width,
            atlas.height,
            CELL[0],
            CELL[1],
        );
        return None;
    }
    Some((columns, rows))
}

/// `None` for a picture stored in a format egui has no pixel for.
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

/// A second upload of the same image: egui hands out its own ids and there is no seam between it
/// and the renderer's [`Texture2DRef`] to share one.
pub struct Sheet {
    texture: egui::TextureHandle,
    columns: u32,
    rows: u32,
}

impl Sheet {
    /// `None` is survivable either way: the catalog lists worlds without pictures.
    pub fn new(egui: &egui::Context, atlas: &CpuTexture) -> Option<Self> {
        let (columns, rows) = grid(atlas)?;
        let Some(image) = color_image(atlas) else {
            log::warn!("{PATH} is not stored as bytes egui can show");
            return None;
        };
        Some(Self {
            // No mipmaps: cells are drawn near their own size.
            texture: egui.load_texture(PATH, image, egui::TextureOptions::LINEAR),
            columns,
            rows,
        })
    }

    /// `cell` reads as in [`cells`] but counts from the top left: egui's images are the right way
    /// up.
    pub fn picture(&self, cell: Option<usize>, height: f32) -> egui::Image<'static> {
        let cell = held(cell, self.columns, self.rows);
        let (column, row) = (cell as u32 % self.columns, cell as u32 / self.columns);
        let cell = egui::vec2(1.0 / self.columns as f32, 1.0 / self.rows as f32);
        let at = egui::pos2(column as f32 * cell.x, row as f32 * cell.y);
        egui::Image::new((self.texture.id(), self.texture.size_vec2()))
            .uv(egui::Rect::from_min_size(at, cell))
            .fit_to_exact_size(egui::vec2(height * ASPECT, height))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_cell_the_atlas_is_too_small_for_falls_back_to_the_placeholder() {
        let (columns, rows) = (4, 4);
        assert_eq!(super::held(Some(0), columns, rows), 0);
        assert_eq!(super::held(Some(14), columns, rows), 14);
        assert_eq!(super::held(Some(15), columns, rows), 15);
        assert_eq!(super::held(Some(9000), columns, rows), 15);
        assert_eq!(super::held(None, columns, rows), 15);
    }
}
