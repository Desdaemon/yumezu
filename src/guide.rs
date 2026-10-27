//! First-run panel naming the controls: the keys are invisible and the rocker is two unlabelled
//! arrows, so nothing on screen can announce itself.

use super::prelude::*;

const DISMISSED: &str = "guide-dismissed";

// Widest unwrapped line within a phone's window.
const WIDTH: f32 = 320.0;

pub(super) struct Guide {
    open: bool,
    /// Separate from `open`: the tick governs later runs, not this one.
    dismissed: bool,
}

impl Guide {
    pub(super) fn new() -> Self {
        let dismissed = super::store::read_flag(DISMISSED);
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
        // Copied: `Window::open` borrows one field while the closure reads the other.
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
                    // "left mouse" is prose, so the input column is localised too.
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
                    super::store::write_flag(DISMISSED, dismissed);
                }
                taken = ui.button(t!("guide-got-it")).clicked();
            });
        self.open = open && !taken;
        self.dismissed = dismissed;
    }
}
