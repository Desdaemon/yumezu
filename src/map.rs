//! A world's maps, in a window that can be dragged out of the way.
//!
//! The wiki draws a floor plan of most worlds and publishes it beside the screenshots — see
//! [`super::world::World::maps`]. It is a picture to be read next to the graph rather than in
//! place of it, so it opens in a window that can be moved and resized, and it stays open across
//! selections until it is closed: holding a map up against where the route goes is the whole use
//! of it.
//!
//! A map is also a drawing at a scale nothing on this screen was sized for — some run to a few
//! thousand pixels of corridors — so it is shown in an [`egui::Scene`], which is the container
//! egui has for a surface that is panned and zoomed rather than laid out. The rest is the wiki's
//! own: several maps to a world where it has drawn the floors or the outskirts separately, each
//! under the caption it publishes them with.

use egui_material_icons::icons::{ICON_CLOSE_FULLSCREEN, ICON_FIT_SCREEN, ICON_OPEN_IN_FULL};
use three_d::renderer::CpuTexture;

use super::{detail, fetch, i18n::t, thumbnails, world};

/// The window's own id, which is fixed rather than taken from the title.
///
/// egui remembers a window's position under its id, so a title-derived one would put the window
/// back in the middle of the screen every time it was opened on a different world. This way it
/// stays where it was last dragged, and only what is drawn in it changes.
const ID: &str = "world map";
/// The size the window opens at, in egui's points. Enough of a map to find one's way around at a
/// glance, and small enough that it does not cover the graph it was opened beside.
const SIZE: [f32; 2] = [380.0, 320.0];
/// How far a map can be taken in and out, as the scale it is drawn at.
///
/// Far enough out that the largest of them fits a small window whole, since that is the view every
/// map opens at, and well past 1:1 the other way: these are drawings of tile floors, and a corner
/// worth looking at is often a few tiles across.
const ZOOM: std::ops::RangeInclusive<f32> = 0.02..=8.0;
/// The wiki's captions are sentences rather than names, and every one of them opens with some of
/// this. Cut off the front of a tab, where the world is already named by the window around it and
/// what is wanted is the words telling one map from another.
const CAPTION_PREFIXES: [&str; 2] = ["Map of the ", "Map of "];

/// The window, and whatever it is holding.
pub(super) struct Maps {
    /// The world it is showing, or `None` while it is closed.
    open: Option<Open>,
    /// How big it is being drawn. See [`Sizing`].
    sizing: Sizing,
}

/// Whether the window is at the size it was dragged to or filling the screen.
///
/// egui keeps a window's position and size under its id and nowhere else, so maximizing overwrites
/// the only record of where it was: what it goes back to has to be held here instead.
#[derive(Clone, Copy)]
enum Sizing {
    /// Moved and resized by hand, which is where it opens and where it spends most of its life.
    Free,
    /// Filling the screen, holding the rect it filled it from.
    Full(egui::Rect),
    /// The one frame it takes to put it back. egui's memory of the window is the maximized rect
    /// by now, so the old one is forced on it once before it is let go of again.
    Restoring(egui::Rect),
}

/// One world's maps, and what it is called.
struct Open {
    world: usize,
    /// The world's own title, which is the window's, rather than a map's caption: a world with
    /// several maps has several captions, and they are what the tabs are named by.
    title: String,
    sheets: Vec<Sheet>,
    /// Which of them is on screen. Always a sheet that exists: only the tabs move it.
    at: usize,
}

/// One map: the wiki's caption, the picture under it, and where the reader has got to in it.
struct Sheet {
    label: String,
    picture: Picture,
    /// What part of the map the window is looking at, in the picture's own pixels, which is what
    /// [`egui::Scene`] pans and zooms by moving. Held per map rather than per window, so stepping
    /// through the tabs and back leaves each one where it was left.
    ///
    /// Empty until the picture arrives and there is a size to fit, which is also what
    /// [`egui::Scene`] reads as "no view yet" and fits from.
    at: egui::Rect,
}

/// How far one map has got.
enum Picture {
    /// On its way in. See [`fetch`].
    Loading(fetch::Pending<Option<CpuTexture>>),
    Ready(egui::TextureHandle),
    /// The wiki has nothing at that address, or nothing egui can show. Kept rather than dropped,
    /// because a window left open would otherwise ask again on every frame of it.
    Missing,
}

impl Maps {
    /// Closed, which is how every run starts: the map is asked for a world at a time.
    pub(super) fn new() -> Self {
        Self {
            open: None,
            sizing: Sizing::Free,
        }
    }

    /// Opens it on a world, or closes it if that is the world it was already showing — the button
    /// that opens it is the button that puts it away again.
    ///
    /// Opening starts every one of the world's maps loading at once. There are seldom more than
    /// two, never more than seven, and they are asked for together because they are tabs of one
    /// window rather than a thing each.
    pub(super) fn toggle(&mut self, world: usize, title: &str, maps: &[world::Map]) {
        if self.open.as_ref().is_some_and(|open| open.world == world) {
            self.open = None;
            return;
        }
        self.open = Some(Open {
            world,
            title: title.to_owned(),
            sheets: maps
                .iter()
                .map(|map| Sheet {
                    label: map.label.clone(),
                    picture: Picture::Loading(detail::load(map.url.clone())),
                    at: egui::Rect::ZERO,
                })
                .collect(),
            at: 0,
        });
    }

    /// Draws it, and takes in whatever arrived since the last frame.
    ///
    /// `insets` is what the system's own furniture covers, which only matters to the maximized
    /// window: that is the one size this picks rather than the reader.
    pub(super) fn show(&mut self, ctx: &egui::Context, insets: egui::Margin) {
        let Some(open) = &mut self.open else {
            return;
        };
        for sheet in &mut open.sheets {
            sheet.arrive(ctx);
        }
        // Copied out and back for the same reason the guide's tick is: [`egui::Window::open`]
        // holds its flag for as long as the closure that reads the rest runs.
        let mut showing = true;
        let window = egui::Window::new(&open.title)
            .id(egui::Id::new(ID))
            .open(&mut showing)
            .constrain(true);
        let window = match self.sizing {
            // Resizable and not scrolling: the map inside does its own panning, so the window's
            // edge is for saying how much of the screen to give it rather than how far down the
            // page to go.
            Sizing::Free => window.default_size(SIZE).resizable(true),
            // The content rect is the whole window here: three-d tells egui nothing about the
            // system's furniture, so the insets the app works out for itself are what keeps a
            // maximized window out from under a status bar.
            Sizing::Full(_) => window.fixed_rect(ctx.content_rect() - insets),
            Sizing::Restoring(rect) => window.fixed_rect(rect),
        };
        let full = self.sizing.is_full();
        let shown = window.show(ctx, |ui| open.show(ui, full));
        if !showing {
            self.open = None;
            return;
        }
        // `inner` is `None` on a frame the window is rolled up into its title bar, which is the
        // other way of getting it out of the way and is egui's own.
        let Some(shown) = shown else {
            return;
        };
        self.sizing = match (shown.inner, self.sizing) {
            (Some(true), Sizing::Full(rect)) => Sizing::Restoring(rect),
            // The rect it is being taken out of, read off the frame that drew it there.
            (Some(true), _) => Sizing::Full(shown.response.rect),
            (_, Sizing::Restoring(_)) => Sizing::Free,
            (_, sizing) => sizing,
        };
    }
}

impl Sizing {
    /// Whether the window is filling the screen, which is what the button in it offers to undo.
    fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }
}

impl Open {
    /// The tabs, if there is more than one, and the map under them. Returns whether the window was
    /// asked to be taken to or out of the whole screen.
    fn show(&mut self, ui: &mut egui::Ui, full: bool) -> bool {
        if self.sheets.is_empty() {
            ui.label(t!("map-none"));
            return false;
        }
        // Bound out of `self` so the row borrows the sheets and the choice separately: one is read
        // to name the tabs and the other is what clicking one writes.
        let (sheets, at) = (&self.sheets, &mut self.at);
        let (mut refit, mut resize) = (false, false);
        ui.horizontal_wrapped(|ui| {
            match sheets.len() {
                // Nothing to choose between, so the caption is read out instead of being made
                // into a tab that does nothing.
                1 => {
                    ui.label(&sheets[0].label);
                }
                _ => {
                    for (which, sheet) in sheets.iter().enumerate() {
                        ui.selectable_value(at, which, caption(&sheet.label))
                            .on_hover_text(&sheet.label);
                    }
                }
            }
            refit = ui
                .button(ICON_FIT_SCREEN)
                .on_hover_text(t!("map-fit"))
                .clicked();
            let (icon, hint) = match full {
                true => (ICON_CLOSE_FULLSCREEN, t!("map-restore")),
                false => (ICON_OPEN_IN_FULL, t!("map-maximize")),
            };
            resize = ui.button(icon).on_hover_text(hint).clicked();
        });
        ui.separator();
        self.sheets[self.at].show(ui, refit);
        resize
    }
}

impl Sheet {
    /// Uploads the picture the frame it turns up, and once only.
    fn arrive(&mut self, ctx: &egui::Context) {
        let Picture::Loading(pending) = &self.picture else {
            return;
        };
        let Some(loaded) = pending.take() else {
            return;
        };
        self.picture = match loaded.as_ref().and_then(thumbnails::color_image) {
            // Linear, and no mipmaps: what a map is drawn at is the reader's to say from one
            // moment to the next, so there is no one size to have filtered for.
            Some(image) => {
                let texture =
                    ctx.load_texture(self.label.clone(), image, egui::TextureOptions::LINEAR);
                self.at = fits(&texture);
                Picture::Ready(texture)
            }
            None => Picture::Missing,
        };
    }

    /// The map itself, on a surface that is dragged and zoomed rather than scrolled.
    fn show(&mut self, ui: &mut egui::Ui, refit: bool) {
        match &self.picture {
            Picture::Loading(_) => {
                ui.spinner();
            }
            Picture::Missing => {
                ui.weak(t!("map-missing"));
            }
            Picture::Ready(texture) => {
                let size = texture.size_vec2();
                let scene = egui::Scene::new()
                    .zoom_range(ZOOM)
                    // The picture is the whole of the contents, so there is nothing to lay out
                    // beyond it and nothing wanted past its edge.
                    .max_inner_size(size);
                let shown = scene.show(ui, &mut self.at, |ui| {
                    // At its own size, in the scene's own coordinates: fitting it to the window
                    // is the scene's transform to do, and doing it here as well would mean
                    // zooming a picture that had already been shrunk to fit.
                    ui.add(egui::Image::new((texture.id(), size)).fit_to_original_size(1.0));
                });
                // The way back, by the button or by the gesture egui's own scenes are reset with.
                if refit || shown.response.double_clicked() {
                    self.at = fits(texture);
                }
            }
        }
    }
}

/// The view that holds the whole of a picture: its own bounds, which [`egui::Scene`] letterboxes
/// into however wide or tall the window happens to be.
///
/// The picture is added at the origin of the scene's coordinates, so this is where it lands.
fn fits(texture: &egui::TextureHandle) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, texture.size_vec2())
}

/// What a tab is named by: the wiki's caption with the words every one of them opens with taken
/// off, and the full stop most of them end on.
///
/// The whole caption is still there to be read, on hovering the tab. This is only what fits on it.
fn caption(label: &str) -> &str {
    let named = CAPTION_PREFIXES
        .iter()
        .find_map(|prefix| label.strip_prefix(prefix))
        .unwrap_or(label);
    named.strip_suffix('.').unwrap_or(named)
}
