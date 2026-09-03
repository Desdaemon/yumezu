//! The controls, said once at the start, and the choice not to be told again.
//!
//! Two of the ways around the graph are the ones nothing on screen can announce for itself: the
//! keys, which are invisible, and the rocker, which is two unlabelled arrows in a corner. So they
//! are named here, on the first run, in a panel that can be dismissed for good.

use three_d::egui;

/// Where the dismissal is kept: the localStorage key on the page, the file name everywhere else.
/// The presence of the value *is* the answer, so there is nothing to parse and nothing that can
/// be half-written.
const DISMISSED: &str = "guide-dismissed";

/// Width the panel is laid out to, in egui's points. Wide enough for the longest line below to
/// read without wrapping mid-phrase, and narrow enough to sit inside a phone's window.
const WIDTH: f32 = 320.0;

/// The panel, and what it remembers.
pub(super) struct Guide {
    /// Whether it is on screen this frame.
    open: bool,
    /// Whether the box is ticked. Kept apart from `open`, because ticking the box does not close
    /// the panel and closing the panel does not tick the box: the tick is about every later run.
    dismissed: bool,
}

impl Guide {
    /// Opens on a run that was never told to stop opening it.
    pub(super) fn new() -> Self {
        let dismissed = remembered();
        Self {
            open: !dismissed,
            dismissed,
        }
    }

    /// Puts it back on screen, for the person who ticked the box and then wanted it again. See
    /// the settings tab.
    pub(super) fn reopen(&mut self) {
        self.open = true;
    }

    /// Draws it, and writes the tick through the moment it changes.
    ///
    /// The fields are copied out and back rather than borrowed: [`egui::Window::open`] holds one
    /// of them for as long as the closure that reads the other runs.
    pub(super) fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let (mut open, mut dismissed, mut taken) = (true, self.dismissed, false);
        egui::Window::new("Controls")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(WIDTH)
            .show(ctx, |ui| {
                ui.set_max_width(WIDTH);
                ui.strong("Inputs");
                egui::Grid::new("input guide").show(ui, |ui| {
                    for (input, label) in [
                        ("W/S", "Fly forward/backward"),
                        ("A/D", "Strafe"),
                        ("Left mouse", "Orbit"),
                        ("One finger", "Orbit"),
                        ("Right mouse", "Options"),
                        ("Right mouse (hold)", "Pan"),
                        ("Two fingers", "Zoom/Pan"),
                        ("Scroll wheel", "Zoom")
                    ] {
                        ui.monospace(input);
                        ui.label(label);
                        ui.end_row();
                    }
                });
                ui.add_space(ui.spacing().item_spacing.y);

                ui.strong("The rocker");
                ui.label(
                    "The two arrows in the bottom-right corner select an entire layer of the graph at once."
                );
                ui.add_space(ui.spacing().item_spacing.y);

                if ui
                    .checkbox(&mut dismissed, "Don't show this again")
                    .changed()
                {
                    remember(dismissed);
                }
                taken = ui.button("Got it").clicked();
            });
        self.open = open && !taken;
        self.dismissed = dismissed;
    }
}

/// Whether this app was told, on some earlier run, to stop opening the panel.
fn remembered() -> bool {
    #[cfg(target_family = "wasm")]
    {
        storage().is_some_and(|storage| storage.get_item(DISMISSED).ok().flatten().is_some())
    }
    #[cfg(not(target_family = "wasm"))]
    {
        marker().is_some_and(|marker| marker.exists())
    }
}

/// Writes that choice down, or takes it back.
///
/// A failure is dropped rather than reported: there is nowhere on screen to report it to, and
/// nothing but this panel's next appearance depends on it.
fn remember(dismissed: bool) {
    #[cfg(target_family = "wasm")]
    {
        let Some(storage) = storage() else { return };
        let _ = match dismissed {
            true => storage.set_item(DISMISSED, "1"),
            false => storage.remove_item(DISMISSED),
        };
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let Some(marker) = marker() else { return };
        match dismissed {
            true => {
                if let Some(directory) = marker.parent() {
                    let _ = std::fs::create_dir_all(directory);
                }
                let _ = std::fs::write(&marker, []);
            }
            false => {
                let _ = std::fs::remove_file(&marker);
            }
        }
    }
}

/// The page's own store, which a page served from a file or with storage turned off has none of.
#[cfg(target_family = "wasm")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The file whose presence is the dismissal, in the one directory this app is allowed to keep
/// anything in. `None` where there is no such directory to be found, which leaves the choice
/// lasting only as long as the run.
#[cfg(not(target_family = "wasm"))]
fn marker() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "android")]
    // The app's own private directory, which the framework hands out and nothing else can read.
    let directory = super::ANDROID.get()?.internal_data_path()?;
    #[cfg(not(target_os = "android"))]
    // Where a desktop keeps what an app is configured with. Read off the environment rather than
    // through a crate: it is two variables and one fallback between them.
    let directory = std::path::PathBuf::from(match std::env::var_os("XDG_CONFIG_HOME") {
        Some(config) => config,
        None => {
            let mut home = std::env::var_os("HOME")?;
            home.push("/.config");
            home
        }
    })
    .join(env!("CARGO_PKG_NAME"));
    Some(directory.join(DISMISSED))
}
