//! Level of detail for world pictures: the atlas cell is the low LoD, and up to [`MAX_HI_LOD`]
//! worlds drawn wider than [`HI_LOD_ENTER_PIXELS`] get the full-size wiki picture as the high LoD.

use std::collections::{HashMap, HashSet};

use image::DynamicImage;
use three_d::renderer::*;

use super::fetch;
use super::instanced::RefilledMesh;
use super::prelude::*;
use super::thumbnails;

/// Node width on screen, in physical pixels, at which the high LoD is fetched. Well past the
/// [`super::thumbnails::CELL`] width, since a slightly stretched thumbnail is not visibly soft.
pub const HI_LOD_ENTER_PIXELS: f32 = 160.0;
/// Node width below which a world drops to the low LoD. Under [`HI_LOD_ENTER_PIXELS`] so a node
/// at the threshold does not flicker between LoDs.
pub const HI_LOD_EXIT_PIXELS: f32 = HI_LOD_ENTER_PIXELS * 0.75;
/// The most worlds at the high LoD at once, given to the nearest nodes first.
const MAX_HI_LOD: usize = 8;
/// The wiki's edge answers a request with no `Origin` with a challenge page. The web build sends
/// the browser's own `Origin` instead.
const ORIGIN: &str = "https://explorer.yume.wiki";
/// How far toward the camera a high-LoD quad is drawn in front of the atlas quad, as a fraction of
/// the distance, to avoid z-fighting.
pub const LIFT: f32 = 0.01;
/// The art's native size. Four in five wiki uploads are exactly 2x, 3x, 4x or 6x this size, which
/// [`sieve`] scales back down; other sizes are kept as uploaded.
const NATIVE: [u32; 2] = [320, 240];

/// One world's high LoD, from fetch to upload.
enum HiLod {
    Loading(fetch::Pending<Option<CpuTexture>>),
    /// Boxed: far larger than [`HiLod::Loading`], which every entry passes through.
    Ready(Box<Gm<Mesh, ColorMaterial>>),
}

pub struct LodPictures {
    /// Per world, the wiki URL of the picture; empty for an unvisited world, which has none.
    images: Vec<String>,
    hi_lod: HashMap<usize, HiLod>,
    /// Worlds whose picture failed to load, so the wiki is not asked again.
    missing: HashSet<usize>,
    /// The worlds within the [`MAX_HI_LOD`] budget, nearest the camera first.
    hi_lod_worlds: Vec<usize>,
    unvisited: Unvisited,
}

/// The placeholder picture of every unvisited world, drawn as one instanced mesh. The only picture
/// those worlds have, so drawn at every size and outside the [`MAX_HI_LOD`] budget.
#[derive(Default)]
struct Unvisited {
    loading: Option<fetch::Pending<Option<CpuTexture>>>,
    /// `None` until the picture loads, and for good if the load fails, leaving bare nodes.
    quads: Option<Gm<RefilledMesh, ColorMaterial>>,
    /// The same picture on the highlighted subset, for the overlay; see
    /// [`LodPictures::drawn_highlighted`].
    lit: Option<Gm<RefilledMesh, ColorMaterial>>,
}

/// A world drawn wider than [`HI_LOD_EXIT_PIXELS`], up for the high LoD.
pub struct LodCandidate {
    pub world: usize,
    /// Width of the node on screen, in physical pixels. Admits the world; distance ranks it.
    pub width: f32,
    /// Copied from the atlas quad, so switching LoD changes only the texture.
    pub transformation: Mat4,
    /// The atlas quad's tint, so a picture dims along with the graph around it.
    pub color: Srgba,
}

impl LodPictures {
    /// `unvisited` is whether the placeholder is worth fetching at all.
    pub fn new(images: Vec<String>, unvisited: bool) -> Self {
        Self {
            images,
            hi_lod: HashMap::new(),
            missing: HashSet::new(),
            hi_lod_worlds: Vec::new(),
            unvisited: Unvisited {
                loading: unvisited.then(thumbnails::placeholder),
                quads: None,
                lit: None,
            },
        }
    }

    /// Whether a picture is still loading, which keeps the window redrawing.
    pub fn has_pending_fetch(&self) -> bool {
        self.unvisited.loading.is_some()
            || self
                .hi_lod
                .values()
                .any(|hi_lod| matches!(hi_lod, HiLod::Loading(_)))
    }

    /// Places the placeholder on `quads` and `lit`, every frame: the layout moves the nodes, the
    /// camera turns them, and a selection dims them.
    pub fn place_unvisited(&mut self, context: &Context, quads: &Instances, lit: &Instances) {
        if let Some(loading) = &self.unvisited.loading
            && let Some(loaded) = loading.take()
        {
            self.unvisited.loading = None;
            if let Some(picture) = loaded {
                // One upload: a [`Texture2DRef`] clone is a Mat3 and an atomic.
                let material = quad_material(context, &picture);
                self.unvisited.lit = Some(Gm::new(
                    RefilledMesh::new(context, lit, &CpuMesh::square()),
                    material.clone(),
                ));
                self.unvisited.quads = Some(Gm::new(
                    RefilledMesh::new(context, quads, &CpuMesh::square()),
                    material,
                ));
            }
        }
        if let Some(drawn) = &mut self.unvisited.quads {
            drawn.set_instances(quads);
        }
        if let Some(drawn) = &mut self.unvisited.lit {
            drawn.set_instances(lit);
        }
    }

    /// Spends the [`MAX_HI_LOD`] budget on `candidates`, nearest first, and drops every other
    /// world to the low LoD.
    pub fn select_hi_lod(&mut self, context: &Context, candidates: &[LodCandidate]) {
        // Filtered before the budget, so a world with no picture takes no slot.
        let selected: Vec<&LodCandidate> = candidates
            .iter()
            .filter(|it| !self.images[it.world].is_empty() && !self.missing.contains(&it.world))
            // A high-LoD world stays down to [`HI_LOD_EXIT_PIXELS`]; dropping a loading world
            // wastes the fetch.
            .filter(|it| it.width >= HI_LOD_ENTER_PIXELS || self.hi_lod.contains_key(&it.world))
            .take(MAX_HI_LOD)
            .collect();
        let hi_lod_worlds: Vec<usize> = selected.iter().map(|it| it.world).collect();
        // Frees slots in the same frame.
        self.hi_lod.retain(|world, _| hi_lod_worlds.contains(world));
        self.hi_lod_worlds = hi_lod_worlds;

        for it in selected {
            let mut hi_lod = match self.hi_lod.remove(&it.world) {
                None => HiLod::Loading(fetch_picture(self.images[it.world].clone())),
                Some(HiLod::Loading(pending)) => match pending.take() {
                    Some(Some(picture)) => HiLod::Ready(Box::new(hi_lod_quad(context, &picture))),
                    Some(None) => {
                        self.missing.insert(it.world);
                        continue;
                    }
                    None => HiLod::Loading(pending),
                },
                Some(hi_lod) => hi_lod,
            };
            // Every frame: the quad follows the atlas quad as the layout and camera move.
            if let HiLod::Ready(quad) = &mut hi_lod {
                quad.set_transformation(it.transformation);
                quad.material.color = it.color;
            }
            self.hi_lod.insert(it.world, hi_lod);
        }
    }

    pub fn drawn(&self) -> impl Iterator<Item = &dyn Object> {
        // The placeholder wins on depth, sitting two [`LIFT`]s in front to the picture's one.
        self.unvisited
            .quads
            .iter()
            .map(|quads| quads as &dyn Object)
            .chain(self.ready_hi_lod().map(|(_, quad)| quad))
    }

    /// The pictures of the highlighted worlds, drawn again because the overlay clears the depth
    /// [`LodPictures::drawn`] wrote.
    pub fn drawn_highlighted<'a>(
        &'a self,
        lit: &'a [usize],
    ) -> impl Iterator<Item = &'a dyn Object> {
        self.unvisited
            .lit
            .iter()
            // Skip empty meshes, which would still trigger the overlay's depth clear.
            .filter(|quads| quads.instance_count() > 0)
            .map(|quads| quads as &dyn Object)
            .chain(
                self.ready_hi_lod()
                    .filter_map(|(world, quad)| lit.contains(&world).then_some(quad)),
            )
    }

    fn ready_hi_lod(&self) -> impl Iterator<Item = (usize, &dyn Object)> {
        self.hi_lod_worlds
            .iter()
            .filter_map(|&world| match self.hi_lod.get(&world) {
                Some(HiLod::Ready(quad)) => Some((world, quad.as_ref() as &dyn Object)),
                _ => None,
            })
    }
}

/// Centre-cropped to [`thumbnails::ASPECT`], as `dreamweaver` crops every atlas cell, so switching
/// LoD keeps the framing.
fn hi_lod_quad(context: &Context, picture: &CpuTexture) -> Gm<Mesh, ColorMaterial> {
    Gm::new(
        Mesh::new(context, &CpuMesh::square()),
        quad_material(context, picture),
    )
}

/// Separate from [`hi_lod_quad`] because the placeholder uses the same material on an instanced
/// mesh.
fn quad_material(context: &Context, picture: &CpuTexture) -> ColorMaterial {
    let (width, height) = (picture.width as f32, picture.height as f32);
    // A centred crop, so the uv origin does not matter.
    let visible = if width > height * thumbnails::ASPECT {
        vec2(height * thumbnails::ASPECT / width, 1.0)
    } else {
        vec2(1.0, width / thumbnails::ASPECT / height)
    };
    let mut texture = Texture2DRef::from_cpu_texture(
        context,
        &CpuTexture {
            // Minified until the switch to the high LoD, where the atlas runs out.
            mipmap: Some(Mipmap::default()),
            wrap_s: Wrapping::ClampToEdge,
            wrap_t: Wrapping::ClampToEdge,
            // 320x240 pixel art; nearest minification would crawl as the layout moves.
            mag_filter: Interpolation::Nearest,
            ..picture.clone()
        },
    );
    texture.transformation = Mat3::from_translation((vec2(1.0, 1.0) - visible) * 0.5)
        * Mat3::from_nonuniform_scale(visible.x, visible.y);
    ColorMaterial {
        texture: Some(texture),
        ..Default::default()
    }
}

/// A wiki picture, for the high LoD or a map. `None` when the picture cannot be fetched or
/// decoded: the world stays at the low LoD, and a map shows no picture.
pub fn fetch_picture(url: String) -> fetch::Pending<Option<CpuTexture>> {
    fetch::spawn(async move {
        let request = fetch::client().get(&url).header("origin", ORIGIN);
        let bytes = match request.fetch_bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("no picture from {url}: {error}");
                return None;
            }
        };
        // Decoded with `image`, whose raw texels [`sieve`] can decimate.
        match image::load_from_memory(&bytes) {
            Ok(picture) => Some(texture(picture)),
            Err(error) => {
                log::warn!("{url} is not an image: {error}");
                None
            }
        }
    })
}

/// RGB stays RGB, three quarters the size of the RGBA every other format widens to.
fn texture(picture: DynamicImage) -> CpuTexture {
    let (width, height) = (picture.width(), picture.height());
    let by = whole_multiple(width, height);
    let data = match picture {
        DynamicImage::ImageRgb8(picture) => {
            TextureData::RgbU8(sieve(picture.into_raw().as_chunks().0, width, by))
        }
        picture => TextureData::RgbaU8(sieve(
            picture.into_rgba8().into_raw().as_chunks().0,
            width,
            by,
        )),
    };
    CpuTexture {
        data,
        width: width / by,
        height: height / by,
        ..Default::default()
    }
}

/// How many times over [`NATIVE`] the upload is, and 1 for anything that is not a whole multiple.
fn whole_multiple(width: u32, height: u32) -> u32 {
    let [wide, tall] = NATIVE;
    let by = width / wide;
    let whole =
        by > 1 && by == height / tall && width.is_multiple_of(wide) && height.is_multiple_of(tall);
    if whole { by } else { 1 }
}

/// Every `by`th texel of every `by`th row; the whole picture when `by` is 1. Sound only on art
/// upscaled by exactly `by`, where each skipped texel copies a kept one.
fn sieve<T: Copy>(texels: &[T], width: u32, by: u32) -> Vec<T> {
    if by == 1 {
        return texels.to_vec();
    }
    let (width, by) = (width as usize, by as usize);
    texels
        .chunks_exact(width)
        .step_by(by)
        .flat_map(|row| row.iter().step_by(by).copied())
        .collect()
}
