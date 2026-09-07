//! First-run panel naming the controls: the keys are invisible and the rocker is two unlabelled
//! arrows, so nothing on screen can announce itself.

use super::i18n::t;

// Presence is the whole answer; the value is always empty.
const DISMISSED: &str = "guide-dismissed";

// Widest unwrapped line, still inside a phone's window.
const WIDTH: f32 = 320.0;

pub(super) struct Guide {
    open: bool,
    /// Separate from `open`: the tick governs later runs, not this one.
    dismissed: bool,
}

impl Guide {
    pub(super) fn new() -> Self {
        let dismissed = remembered();
        Self {
            open: !dismissed,
            dismissed,
        }
    }

    pub(super) fn reopen(&mut self) {
        self.open = true;
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        // Copied out and back rather than borrowed: `Window::open` holds one field for as long
        // as the closure reading the other runs.
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
                    // The input column is localised too: "left mouse" is prose, not a key legend.
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

fn remembered() -> bool {
    super::store::read(DISMISSED).is_some()
}

fn remember(dismissed: bool) {
    super::store::write(DISMISSED, dismissed.then_some(""));
}
