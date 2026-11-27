//! The wiki's floor plans for a world, in a movable window with a tab per map and
//! [`egui::Scene`] to pan large ones.

use egui_material_icons::icons::{ICON_CLOSE_FULLSCREEN, ICON_FIT_SCREEN, ICON_OPEN_IN_FULL};
use three_d::renderer::CpuTexture;

use super::{fetch, i18n::t, lod, thumbnails, world};

// Fixed: egui keys window position by id, so a title-derived id would recentre per world.
const ID: &str = "world map";
const SIZE: [f32; 2] = [380.0, 320.0];
// Past 1:1, since a corner worth seeing is often a few tiles across.
const ZOOM: std::ops::RangeInclusive<f32> = 0.02..=8.0;
// Cut off a tab, where the window already names the world.
const CAPTION_PREFIXES: [&str; 2] = ["Map of the ", "Map of "];

pub(super) struct Maps {
    open: Option<Open>,
    sizing: Sizing,
}

/// egui stores a window's rect only under the window's id, so maximizing overwrites the old rect.
#[derive(Clone, Copy)]
enum Sizing {
    /// Moved and resized by hand, which is where the window opens.
    Free,
    Full(egui::Rect),
    /// One frame forcing the old rect back over the maximized rect in egui's memory.
    Restoring(egui::Rect),
}

struct Open {
    world: usize,
    /// The window's title. Tabs show map captions instead.
    title: String,
    sheets: Vec<Sheet>,
    /// Always a valid map index: only the tabs change the index.
    at: usize,
}

struct Sheet {
    label: String,
    picture: Picture,
    /// The view in picture pixels, per map, so switching tabs keeps each view. Empty until the
    /// picture resolves, which [`egui::Scene`] reads as "no view yet".
    at: egui::Rect,
}

enum Picture {
    Loading(fetch::Pending<Option<CpuTexture>>),
    Ready(egui::TextureHandle),
    /// Kept, so an open window does not refetch every frame.
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
                    picture: Picture::Loading(lod::fetch_picture(map.url.clone())),
                    at: egui::Rect::ZERO,
                })
                .collect(),
            at: 0,
        });
    }

    /// `insets` is the area the system bars cover, which only the maximized window avoids.
    pub(super) fn show(&mut self, ctx: &egui::Context, insets: super::ui::Insets) {
        let Some(open) = &mut self.open else {
            return;
        };
        for sheet in &mut open.sheets {
            sheet.resolve(ctx);
        }
        // Copied: `Window::open` borrows the flag while the closure runs.
        let mut showing = true;
        let window = egui::Window::new(&open.title)
            .id(egui::Id::new(ID))
            .open(&mut showing)
            .constrain(true);
        let window = match self.sizing {
            // The map pans itself, so the window does not scroll.
            Sizing::Free => window.default_size(SIZE).resizable(true),
            // three-d reports no system insets, so the app's own keep this clear of a status bar.
            Sizing::Full(_) => window.fixed_rect(insets.shrink_rect(ctx.content_rect())),
            Sizing::Restoring(rect) => window.fixed_rect(rect),
        };
        let full = self.sizing.is_full();
        let shown = window.show(ctx, |ui| open.show(ui, full));
        if !showing {
            self.open = None;
            return;
        }
        // `None` while the window is rolled up.
        let Some(shown) = shown else {
            return;
        };
        self.sizing = match (shown.inner, self.sizing) {
            (Some(true), Sizing::Full(rect)) => Sizing::Restoring(rect),
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
    /// Returns whether maximizing was toggled.
    fn show(&mut self, ui: &mut egui::Ui, full: bool) -> bool {
        if self.sheets.is_empty() {
            ui.label(t!("map-none"));
            return false;
        }
        let (sheets, at) = (&self.sheets, &mut self.at);
        let (mut refit, mut resize) = (false, false);
        ui.horizontal_wrapped(|ui| {
            match sheets.len() {
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
    fn resolve(&mut self, ctx: &egui::Context) {
        let Picture::Loading(pending) = &self.picture else {
            return;
        };
        let Some(loaded) = pending.take() else {
            return;
        };
        self.picture = match loaded.as_ref().and_then(thumbnails::color_image) {
            // No mipmaps: the zoom changes freely.
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
                let scene = egui::Scene::new().zoom_range(ZOOM).max_inner_size(size);
                let shown = scene.show(ui, &mut self.at, |ui| {
                    // The scene's transform already fits the picture.
                    ui.add(egui::Image::new((texture.id(), size)).fit_to_original_size(1.0));
                });
                if refit || shown.response.double_clicked() {
                    self.at = fits(texture);
                }
            }
        }
    }
}

/// The view holding the whole picture at the origin, which [`egui::Scene`] letterboxes into the
/// window.
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
