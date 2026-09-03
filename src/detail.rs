//! Full-size world pictures, for the few worlds the view has come close enough to.
//!
//! The atlas holds every world at [`super::thumbnails::CELL`], which is all a node drawn a handful
//! of pixels wide can show. Come close enough and the screen starts asking for more texels than
//! the atlas has, and the picture goes soft. That is what this module answers: past
//! [`SWITCH_PIXELS`] a world's picture is fetched from the wiki at the size the wiki keeps it, and
//! drawn over the world's own atlas quad.
//!
//! Only a handful are ever held at once — see [`HELD`] — because a full picture is a texture and a
//! draw call of its own, where the atlas is one of each for the whole graph. Which handful is the
//! view's to say, frame by frame, so this module holds no opinion about the layout: it is told
//! what to show and where.

use std::collections::HashMap;

use three_d::renderer::*;

use super::{fetch, thumbnails};

/// How wide a node has to come out on screen, in physical pixels, before its picture is worth
/// fetching at full size.
///
/// Rather more than the [`super::thumbnails::CELL`] width the atlas holds, which is the point
/// where magnification technically begins: a thumbnail stretched a little is not visibly soft, and
/// switching at the first texel of magnification would spend a download on a node nobody is
/// looking at yet. This is the width the softness starts to read at.
pub const SWITCH_PIXELS: f32 = 160.0;
/// How many full pictures are held at once.
///
/// A ceiling on the cost rather than on the view: the widest nodes are served first, so coming in
/// on a crowd sharpens the ones nearest the camera and leaves the rest on the atlas. Small,
/// because each one is a texture upload and a draw call, and because a view that has this many
/// nodes above [`SWITCH_PIXELS`] is a view of a wall of pictures rather than of a world.
const HELD: usize = 8;
/// Which site the wiki serves pictures to.
///
/// Its edge answers a request with no `Origin` at all with a challenge page rather than the
/// picture, and answers this one with the picture and a header allowing it. On the page the
/// browser sets this itself, and refuses to let it be set — so this only carries the native build,
/// and a page served from anywhere else is on its own for cross-origin permission.
const ORIGIN: &str = "https://explorer.yume.wiki";
/// How far in front of its own atlas quad a full picture is drawn, as a fraction of the node's
/// radius.
///
/// The two quads are the same size in the same place, so without this they would be coplanar and
/// the depth test would pick between them per pixel. Toward the camera, and small enough that the
/// picture neither grows visibly nor pulls out of a node the layout has crowded.
pub const LIFT: f32 = 0.02;

/// One world's picture, at whatever stage it has reached.
enum Held {
    /// On its way in. See [`fetch`].
    Loading(fetch::Pending<Option<CpuTexture>>),
    /// Ready to draw, cropped and sized to stand exactly over the world's atlas quad. Boxed
    /// because it is far the largest of these, and the other two are what most of them are.
    Ready(Box<Gm<Mesh, ColorMaterial>>),
    /// The wiki has no picture here, or none this can read. Kept so it is not asked for again
    /// every time the view comes back.
    Missing,
}

/// The full-size pictures currently held, and what to draw them on.
pub struct Detail {
    /// Per world, where the wiki serves its picture from.
    images: Vec<String>,
    held: HashMap<usize, Held>,
    /// The worlds [`Detail::track`] was last asked for, widest first, which is the order they are
    /// drawn in and the order the budget is spent in.
    wanted: Vec<usize>,
}

/// A world the view is asking for at full size, and how its quad is drawn.
pub struct Magnified {
    pub world: usize,
    /// Where and how big, taken from the world's own atlas quad so the switch changes the detail
    /// and nothing else.
    pub transformation: Mat4,
    /// What the atlas quad is tinted with, so a picture dims along with the graph around it.
    pub color: Srgba,
}

impl Detail {
    pub fn new(images: Vec<String>) -> Self {
        Self {
            images,
            held: HashMap::new(),
            wanted: Vec::new(),
        }
    }

    /// Brings the held pictures in line with what the view is asking for, and places the ones that
    /// have arrived.
    ///
    /// `magnified` is every world drawn wider than [`SWITCH_PIXELS`], widest first. Anything past
    /// [`HELD`] of it is left on the atlas, and anything held but no longer asked for is dropped —
    /// which is the whole of the eviction policy, because what the view is not looking at costs
    /// nothing to fetch again if it looks back.
    pub fn track(&mut self, context: &Context, magnified: &[Magnified]) {
        let magnified = &magnified[..magnified.len().min(HELD)];
        let wanted: Vec<usize> = magnified.iter().map(|it| it.world).collect();
        // Before the new ones are started, so a picture on its way out frees its budget for one
        // on its way in within the same frame. A world with no picture is kept either way: it
        // holds nothing, and forgetting it would mean asking the wiki again on every visit.
        self.held
            .retain(|world, held| matches!(held, Held::Missing) || wanted.contains(world));
        self.wanted = wanted;

        for it in magnified {
            // Taken out of the store rather than looked at in it, so whatever it has turned into
            // can go back in its place without the store being borrowed twice.
            let mut held = match self.held.remove(&it.world) {
                None => Held::Loading(load(self.images[it.world].clone())),
                Some(Held::Loading(pending)) => match pending.take() {
                    Some(Some(picture)) => Held::Ready(Box::new(quad(context, &picture))),
                    Some(None) => Held::Missing,
                    None => Held::Loading(pending),
                },
                Some(held) => held,
            };
            // Every frame, not just on arrival: the layout moves under the nodes and the camera
            // turns the quads, and this one has to stay over the atlas quad it stands in for.
            if let Held::Ready(quad) = &mut held {
                quad.set_transformation(it.transformation);
                quad.material.color = it.color;
            }
            self.held.insert(it.world, held);
        }
    }

    /// The pictures ready to draw, widest first. See [`Detail::track`], which is what decides both.
    pub fn drawn(&self) -> impl Iterator<Item = &dyn Object> {
        self.wanted
            .iter()
            .filter_map(|world| match self.held.get(world) {
                Some(Held::Ready(quad)) => Some(quad.as_ref() as &dyn Object),
                _ => None,
            })
    }
}

/// A quad carrying `picture`, cropped the way the atlas crops its cells.
///
/// The crop is what makes the switch invisible: `tools/atlas` centre-crops every picture to
/// [`thumbnails::ASPECT`] before packing it, and the node's quad is that shape, so a full picture
/// shown whole would jump to a different framing of the same screenshot.
fn quad(context: &Context, picture: &CpuTexture) -> Gm<Mesh, ColorMaterial> {
    let (width, height) = (picture.width as f32, picture.height as f32);
    // Whichever side is long for the shape wanted is the one that gives, centred: the other is
    // kept whole. A symmetric crop, so it does not matter which end of the picture the uv
    // coordinates count from — which they do differently here than in the atlas.
    let visible = if width > height * thumbnails::ASPECT {
        vec2(height * thumbnails::ASPECT / width, 1.0)
    } else {
        vec2(1.0, width / thumbnails::ASPECT / height)
    };
    let mut texture = Texture2DRef::from_cpu_texture(
        context,
        &CpuTexture {
            // A full picture is minified onto the node at every size below the one it was fetched
            // for, which is most of them: the switch happens where the atlas runs out, not where
            // this picture is finally shown at its own size.
            mipmap: Some(Mipmap::default()),
            wrap_s: Wrapping::ClampToEdge,
            wrap_t: Wrapping::ClampToEdge,
            ..picture.clone()
        },
    );
    texture.transformation = Mat3::from_translation((vec2(1.0, 1.0) - visible) * 0.5)
        * Mat3::from_nonuniform_scale(visible.x, visible.y);
    Gm::new(
        Mesh::new(context, &CpuMesh::square()),
        ColorMaterial {
            texture: Some(texture),
            ..Default::default()
        },
    )
}

/// Starts loading one picture from the wiki. See [`fetch`].
///
/// `None` for anything that cannot be had or read, which is not fatal anywhere it is called from:
/// a world keeps the atlas cell it already had, a little soft, and a map says it has no picture.
/// The map window loads through here too — the wiki serves both from the same edge, and it is
/// [`ORIGIN`] and the decoder that make the difference between a picture and a challenge page.
pub fn load(url: String) -> fetch::Pending<Option<CpuTexture>> {
    fetch::spawn(async move {
        let bytes = match download(&url).await {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("no full picture from {url}: {error}");
                return None;
            }
        };
        // Through the same decoder the atlas goes through, which picks its format off the path —
        // the wiki's own, extension and all.
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

/// Fetches one picture's bytes.
///
/// Its own client rather than a shared one: eight downloads over a session do not repay a
/// connection pool, and building the client inside the request keeps it on the executor
/// [`fetch::spawn`] put this on, which is where the native one needs to be.
async fn download(url: &str) -> Result<Vec<u8>, reqwest::Error> {
    Ok(reqwest::Client::new()
        .get(url)
        // See [`ORIGIN`]. Dropped by the browser, which sets its own.
        .header("origin", ORIGIN)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec())
}
