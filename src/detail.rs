//! Full-size world pictures, for the few worlds the view has come close enough to.
//!
//! Past [`SWITCH_PIXELS`] the screen asks for more texels than the atlas cell has, so the wiki's
//! own copy is fetched and drawn over the world's atlas quad. Only [`HELD`] at once, each being a
//! texture and a draw call of its own where the atlas is one of each for the whole graph.

use std::collections::{HashMap, HashSet};

use three_d::renderer::*;

use super::{fetch, thumbnails};

/// Node width on screen, in physical pixels, at which the full picture becomes worth fetching.
///
/// Well past the [`super::thumbnails::CELL`] width where magnification begins: a thumbnail
/// stretched a little is not visibly soft, and switching at the first magnified texel would spend a
/// download on a node nobody is looking at yet.
pub const SWITCH_PIXELS: f32 = 160.0;
/// Node width at which a world gives its own picture back. Under [`SWITCH_PIXELS`] so a node
/// sitting on the switch does not trade picture for atlas cell every other frame.
pub const LEAVE_PIXELS: f32 = SWITCH_PIXELS * 0.75;
/// A ceiling on cost, not on the view: the nearest nodes are served first, so coming in on a crowd
/// sharpens the ones the view has come in on and leaves the rest on the atlas.
const HELD: usize = 8;
/// The wiki's edge answers a request with no `Origin` with a challenge page rather than the
/// picture. Only the native build sets it: the browser sets its own and will not be overridden.
const ORIGIN: &str = "https://explorer.yume.wiki";
/// How far in front of its own atlas quad a full picture is drawn, as a fraction of the node's
/// radius: the two are otherwise coplanar and the depth test would pick between them per pixel.
/// Small enough that the picture neither grows visibly nor pulls out of a crowded node.
pub const LIFT: f32 = 0.02;

enum Held {
    Loading(fetch::Pending<Option<CpuTexture>>),
    /// Boxed because it dwarfs [`Held::Loading`], which every entry passes through.
    Ready(Box<Gm<Mesh, ColorMaterial>>),
}

pub struct Detail {
    /// Per world, where the wiki serves its picture from. Empty for a world the player has not
    /// been to, which has no picture of its own to fetch.
    images: Vec<String>,
    held: HashMap<usize, Held>,
    /// Kept so the wiki is not asked again every time the view comes back.
    missing: HashSet<usize>,
    /// Nearest the camera first, which is the order the budget is spent in.
    wanted: Vec<usize>,
    unvisited: Unvisited,
}

/// The picture every world the player has not been to is drawn as. One picture however many worlds
/// wear it, so it is held once and drawn as a single instanced mesh -- a frontier can put hundreds
/// on screen at once.
///
/// Not a level of detail, unlike everything else here: it is the world's only picture, so it is
/// drawn at every size and spends none of the [`HELD`] budget.
#[derive(Default)]
struct Unvisited {
    loading: Option<fetch::Pending<Option<CpuTexture>>>,
    /// `None` until the picture arrives, and forever if it cannot be had, which leaves these worlds
    /// their bare nodes.
    quads: Option<Gm<InstancedMesh, ColorMaterial>>,
}

pub struct Magnified {
    pub world: usize,
    /// How wide the node comes out on screen, in physical pixels. What admits it, not what ranks
    /// it -- see [`Detail::wanted`].
    pub width: f32,
    /// Taken from the world's own atlas quad, so the switch changes the detail and nothing else.
    pub transformation: Mat4,
    /// The atlas quad's tint, so a picture dims along with the graph around it.
    pub color: Srgba,
}

impl Detail {
    /// `unvisited` is whether the placeholder is worth fetching at all.
    pub fn new(images: Vec<String>, unvisited: bool) -> Self {
        Self {
            images,
            held: HashMap::new(),
            missing: HashSet::new(),
            wanted: Vec::new(),
            unvisited: Unvisited {
                loading: unvisited.then(thumbnails::placeholder),
                quads: None,
            },
        }
    }

    /// Stands the placeholder on the quads it was handed. Every frame, because the layout moves the
    /// nodes, the camera turns them, and a selection dims them.
    /// What keeps the window drawing while a world sharpens.
    pub fn pending(&self) -> bool {
        self.unvisited.loading.is_some()
            || self
                .held
                .values()
                .any(|held| matches!(held, Held::Loading(_)))
    }

    pub fn place_unvisited(&mut self, context: &Context, quads: &Instances) {
        if let Some(loading) = &self.unvisited.loading
            && let Some(loaded) = loading.take()
        {
            self.unvisited.loading = None;
            // Failures are logged where they are found, and leave these worlds their bare nodes.
            if let Some(picture) = loaded {
                self.unvisited.quads = Some(Gm::new(
                    InstancedMesh::new(context, quads, &CpuMesh::square()),
                    quad_material(context, &picture),
                ));
            }
        }
        if let Some(drawn) = &mut self.unvisited.quads {
            drawn.set_instances(quads);
        }
    }

    /// `magnified` is every world drawn wider than [`LEAVE_PIXELS`], nearest first. Anything past
    /// [`HELD`] is left on the atlas, and anything held but no longer asked for is dropped.
    pub fn track(&mut self, context: &Context, magnified: &[Magnified]) {
        // Filtered before the budget is counted: a world with no picture to be had already has the
        // atlas cell or the placeholder, and a slot spent on it would push out a world that has
        // one.
        let magnified: Vec<&Magnified> = magnified
            .iter()
            .filter(|it| !self.images[it.world].is_empty() && !self.missing.contains(&it.world))
            // The upper edge of the band the caller admitted: a world wide enough starts, one
            // already held carries on down to [`LEAVE_PIXELS`]. Loading counts, dropping it
            // stranding the fetch.
            .filter(|it| it.width >= SWITCH_PIXELS || self.held.contains_key(&it.world))
            .take(HELD)
            .collect();
        let wanted: Vec<usize> = magnified.iter().map(|it| it.world).collect();
        // Before the new ones start, so a picture on its way out frees its slot in the same frame.
        self.held.retain(|world, _| wanted.contains(world));
        self.wanted = wanted;

        for it in magnified {
            // Removed rather than borrowed, so whatever it turns into goes back without a second
            // borrow.
            let mut held = match self.held.remove(&it.world) {
                None => Held::Loading(load(self.images[it.world].clone())),
                Some(Held::Loading(pending)) => match pending.take() {
                    Some(Some(picture)) => Held::Ready(Box::new(quad(context, &picture))),
                    // Out of the running above rather than occupying a slot it can never draw
                    // from.
                    Some(None) => {
                        self.missing.insert(it.world);
                        continue;
                    }
                    None => Held::Loading(pending),
                },
                Some(held) => held,
            };
            // Every frame, not just on arrival: the quad has to keep up with the atlas quad it
            // stands over as the layout and the camera move.
            if let Held::Ready(quad) = &mut held {
                quad.set_transformation(it.transformation);
                quad.material.color = it.color;
            }
            self.held.insert(it.world, held);
        }
    }

    pub fn drawn(&self) -> impl Iterator<Item = &dyn Object> {
        // A world turning over wears both for a moment, and the placeholder wins on depth:
        // [`LIFT`] lifts it against the node's whole radius where the picture is lifted against
        // the shrinking radius it is drawn at.
        self.unvisited
            .quads
            .iter()
            .map(|quads| quads as &dyn Object)
            .chain(self.pictures().map(|(_, quad)| quad))
    }

    /// The pictures of the worlds a selection lights.
    ///
    /// The overlay clears the depth [`Detail::drawn`] wrote, so a lit world it does not draw again
    /// falls back to the atlas quad underneath. Only the lit ones -- a magnified world outside the
    /// selection belongs under the overlay, not in it.
    pub fn drawn_lit<'a>(&'a self, lit: &'a [usize]) -> impl Iterator<Item = &'a dyn Object> {
        self.pictures()
            .filter_map(|(world, quad)| lit.contains(&world).then_some(quad))
    }

    fn pictures(&self) -> impl Iterator<Item = (usize, &dyn Object)> {
        self.wanted
            .iter()
            .filter_map(|&world| match self.held.get(&world) {
                Some(Held::Ready(quad)) => Some((world, quad.as_ref() as &dyn Object)),
                _ => None,
            })
    }
}

/// Cropping is what makes the switch invisible: `tools/atlas` centre-crops every picture to
/// [`thumbnails::ASPECT`] before packing it, and the node's quad has that aspect ratio, so a full
/// picture shown whole would jump to a different framing of the same screenshot.
fn quad(context: &Context, picture: &CpuTexture) -> Gm<Mesh, ColorMaterial> {
    Gm::new(
        Mesh::new(context, &CpuMesh::square()),
        quad_material(context, picture),
    )
}

/// Apart from [`quad`] because the placeholder wears the same paint on an instanced mesh.
fn quad_material(context: &Context, picture: &CpuTexture) -> ColorMaterial {
    let (width, height) = (picture.width as f32, picture.height as f32);
    // The long side gives, centred, and the other is kept whole. Symmetric, so it does not matter
    // which end the uv coordinates count from -- not the same end as in the atlas.
    let visible = if width > height * thumbnails::ASPECT {
        vec2(height * thumbnails::ASPECT / width, 1.0)
    } else {
        vec2(1.0, width / thumbnails::ASPECT / height)
    };
    let mut texture = Texture2DRef::from_cpu_texture(
        context,
        &CpuTexture {
            // The switch happens where the atlas runs out rather than where this picture reaches
            // its own size, so it is minified onto the node at every size below that.
            mipmap: Some(Mipmap::default()),
            wrap_s: Wrapping::ClampToEdge,
            wrap_t: Wrapping::ClampToEdge,
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

/// `None` for anything that cannot be had or read, which is fatal nowhere this is called from: a
/// world keeps its slightly soft atlas cell, and a map says it has no picture. [`ORIGIN`] and the
/// decoder are what separate a picture from a challenge page.
pub fn load(url: String) -> fetch::Pending<Option<CpuTexture>> {
    fetch::spawn(async move {
        let bytes = match download(&url).await {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("no full picture from {url}: {error}");
                return None;
            }
        };
        // The same decoder the atlas goes through, which picks the format off the path.
        let mut assets = three_d_asset::io::RawAssets::new();
        assets.insert(&url, bytes);
        match assets.deserialize::<CpuTexture>(&url) {
            Ok(picture) => Some(picture),
            Err(error) => {
                log::warn!("{url} is not an image: {error}");
                None
            }
        }
    })
}

async fn download(url: &str) -> Result<Vec<u8>, fetch::Error> {
    Ok(fetch::client()
        .get(url)
        // Dropped by the browser, which sets its own.
        .header("origin", ORIGIN)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec())
}
