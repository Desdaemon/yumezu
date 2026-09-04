//! The controls, said once at the start, and the choice not to be told again.
//!
//! Two of the ways around the graph are the ones nothing on screen can announce for itself: the
//! keys, which are invisible, and the rocker, which is two unlabelled arrows in a corner. So they
//! are named here, on the first run, in a panel that can be dismissed for good.

use super::i18n::t;

/// What the dismissal is kept under. The presence of the value *is* the answer, so there is
/// nothing to parse and nothing that can be half-written. See [`super::store`].
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
        egui::Window::new(t!("guide-title"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(WIDTH)
            .show(ctx, |ui| {
                ui.set_max_width(WIDTH);
                ui.strong(t!("guide-inputs"));
                egui::Grid::new("input guide").show(ui, |ui| {
                    // Both halves are said rather than only the second: the keys are legends a
                    // keyboard is printed with and stay as they are, but "left mouse" and "one
                    // finger" are as much prose as what they do.
                    for row in [
                        "fly",
                        "strafe",
                        "orbit-mouse",
                        "orbit-touch",
                        "options",
                        "pan",
                        "pinch",
                        "scroll",
                    ] {
                        ui.monospace(super::i18n::format(&format!("guide-{row}-input"), None));
                        ui.label(super::i18n::format(&format!("guide-{row}-action"), None));
                        ui.end_row();
                    }
                });
                ui.add_space(ui.spacing().item_spacing.y);

                ui.strong(t!("guide-rocker"));
                ui.label(t!("guide-rocker-body"));
                ui.add_space(ui.spacing().item_spacing.y);

                if ui.checkbox(&mut dismissed, t!("dont-show-again")).changed() {
                    remember(dismissed);
                }
                taken = ui.button(t!("guide-got-it")).clicked();
            });
        self.open = open && !taken;
        self.dismissed = dismissed;
    }
}

/// Whether this app was told, on some earlier run, to stop opening the panel.
fn remembered() -> bool {
    super::store::read(DISMISSED).is_some()
}

/// Writes that choice down, or takes it back.
fn remember(dismissed: bool) {
    super::store::write(DISMISSED, dismissed.then_some(""));
}
