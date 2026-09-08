//! The wiki's floor plans for a world -- see [`super::world::World::maps`].
//!
//! Read next to the graph rather than in place of it, so they get a window that can be moved and
//! resized and stays open across selections. Some run to a few thousand pixels of corridors, hence
//! [`egui::Scene`] -- panned and zoomed rather than laid out. Worlds whose floors the wiki drew
//! separately get a tab per map.

use egui_material_icons::icons::{ICON_CLOSE_FULLSCREEN, ICON_FIT_SCREEN, ICON_OPEN_IN_FULL};
use three_d::renderer::CpuTexture;

use super::{detail, fetch, i18n::t, thumbnails, world};

// Fixed rather than taken from the title: egui files a window's position under its id, so a
// title-derived one would recentre the window every time a different world opened it.
const ID: &str = "world map";
// Enough map to get one's bearings without covering the graph it was opened beside.
const SIZE: [f32; 2] = [380.0, 320.0];
// Far enough out that the largest map fits a small window whole, and well past 1:1 the other way
// -- a corner worth looking at is often only a few tiles across.
const ZOOM: std::ops::RangeInclusive<f32> = 0.02..=8.0;
// The wiki's captions are sentences, not names, and all start with one of these. Cut off a tab,
// where the window already names the world.
const CAPTION_PREFIXES: [&str; 2] = ["Map of the ", "Map of "];

pub(super) struct Maps {
    open: Option<Open>,
    sizing: Sizing,
}

/// egui keeps a window's position and size under its id and nowhere else, so maximizing overwrites
/// the only record of where it was.
#[derive(Clone, Copy)]
enum Sizing {
    /// Moved and resized by hand, which is where the window opens.
    Free,
    Full(egui::Rect),
    /// The one frame it takes to put the window back: egui's memory of it is the maximized rect by
    /// now, so the old one is forced on it once.
    Restoring(egui::Rect),
}

struct Open {
    world: usize,
    /// The window's title. A map's caption names its tab instead.
    title: String,
    sheets: Vec<Sheet>,
    /// Always a sheet that exists: only the tabs move it.
    at: usize,
}

struct Sheet {
    label: String,
    picture: Picture,
    /// In the picture's own pixels, per map, so stepping through the tabs and back leaves each one
    /// where it was left. Empty until the picture arrives and there is a size to fit, which is also
    /// what [`egui::Scene`] reads as "no view yet".
    at: egui::Rect,
}

enum Picture {
    Loading(fetch::Pending<Option<CpuTexture>>),
    Ready(egui::TextureHandle),
    /// Kept rather than dropped: a window left open would otherwise ask again every frame.
    Missing,
}

impl Maps {
    pub(super) fn new() -> Self {
        Self {
            open: None,
            sizing: Sizing::Free,
        }
    }

    /// Every one of the world's maps starts loading at once: there are never more than seven.
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

    /// `insets` is what the system's own furniture covers, which matters only to the maximized
    /// window -- the one size this picks rather than the reader.
    pub(super) fn show(&mut self, ctx: &egui::Context, insets: egui::Margin) {
        let Some(open) = &mut self.open else {
            return;
        };
        for sheet in &mut open.sheets {
            sheet.arrive(ctx);
        }
        // Copied out and back: `Window::open` holds its flag for as long as the closure reading
        // the rest runs.
        let mut showing = true;
        let window = egui::Window::new(&open.title)
            .id(egui::Id::new(ID))
            .open(&mut showing)
            .constrain(true);
        let window = match self.sizing {
            // Not scrolling: the map does its own panning, so the window's edge only says how much
            // of the screen to give it.
            Sizing::Free => window.default_size(SIZE).resizable(true),
            // three-d tells egui nothing about the system's furniture, so egui's content rect is
            // the whole window; the app's own insets keep a maximized window out from under a
            // status bar.
            Sizing::Full(_) => window.fixed_rect(ctx.content_rect() - insets),
            Sizing::Restoring(rect) => window.fixed_rect(rect),
        };
        let full = self.sizing.is_full();
        let shown = window.show(ctx, |ui| open.show(ui, full));
        if !showing {
            self.open = None;
            return;
        }
        // `inner` is `None` on a frame the window is rolled up into its title bar.
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
    fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }
}

impl Open {
    /// Returns whether the window was asked to be taken to or out of the whole screen.
    fn show(&mut self, ui: &mut egui::Ui, full: bool) -> bool {
        if self.sheets.is_empty() {
            ui.label(t!("map-none"));
            return false;
        }
        // Bound out of `self` so the row can borrow the sheets and the choice separately.
        let (sheets, at) = (&self.sheets, &mut self.at);
        let (mut refit, mut resize) = (false, false);
        ui.horizontal_wrapped(|ui| {
            match sheets.len() {
                // Nothing to choose between, so the caption is a label, not a dead tab.
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
    fn arrive(&mut self, ctx: &egui::Context) {
        let Picture::Loading(pending) = &self.picture else {
            return;
        };
        let Some(loaded) = pending.take() else {
            return;
        };
        self.picture = match loaded.as_ref().and_then(thumbnails::color_image) {
            // No mipmaps: the reader picks the scale moment to moment, so there is no one size to
            // have filtered for.
            Some(image) => {
                let texture =
                    ctx.load_texture(self.label.clone(), image, egui::TextureOptions::LINEAR);
                self.at = fits(&texture);
                Picture::Ready(texture)
            }
            None => Picture::Missing,
        };
    }

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
                    // The picture is the whole of the contents; nothing is wanted past its edge.
                    .max_inner_size(size);
                let shown = scene.show(ui, &mut self.at, |ui| {
                    // At its own size, in the scene's own coordinates: the scene's transform does
                    // the fitting, and fitting here too would zoom an already-shrunk picture.
                    ui.add(egui::Image::new((texture.id(), size)).fit_to_original_size(1.0));
                });
                if refit || shown.response.double_clicked() {
                    self.at = fits(texture);
                }
            }
        }
    }
}

/// The view holding the whole picture, which [`egui::Scene`] letterboxes into however wide or tall
/// the window happens to be. The picture is added at the origin, so this is where it lands.
fn fits(texture: &egui::TextureHandle) -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, texture.size_vec2())
}

fn caption(label: &str) -> &str {
    let named = CAPTION_PREFIXES
        .iter()
        .find_map(|prefix| label.strip_prefix(prefix))
        .unwrap_or(label);
    named.strip_suffix('.').unwrap_or(named)
}
