//! The bar offering the Android apk, which draws the same graph faster than the page does.
//!
//! Shown only on the page and only on Android: a desktop has nothing to install, an iPhone
//! cannot install this, and the apk itself is already what is being offered.

use egui_material_icons::icons::{ICON_ANDROID, ICON_CLOSE};

use super::i18n::t;

const URL: &str = "/android";

// Presence is the whole answer; the value is always empty.
const DISMISSED: &str = "download-dismissed";

pub(super) struct Offer {
    open: bool,
}

impl Offer {
    pub(super) fn new() -> Self {
        Self {
            open: on_android_browser() && super::store::read(DISMISSED).is_none(),
        }
    }

    /// `insets` is what the system's own furniture covers: the bar stands off the bottom of the
    /// safe area, not of the window, which on a phone sits behind the navigation bar.
    pub(super) fn show(&mut self, ctx: &egui::Context, insets: egui::Margin) {
        if !self.open {
            return;
        }
        egui::Area::new(egui::Id::new("android offer"))
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                [0.0, -((insets.bottom + super::PANEL_MARGIN) as f32)],
            )
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .button(format!(
                                "{}  {}",
                                ICON_ANDROID.codepoint,
                                t!("download-android")
                            ))
                            .clicked()
                        {
                            super::open_in_browser(URL);
                            // Taking the offer ends it too: there is no second apk to hand out.
                            self.dismiss();
                        }
                        if ui
                            .button(ICON_CLOSE)
                            .on_hover_text(t!("dont-show-again"))
                            .clicked()
                        {
                            self.dismiss();
                        }
                    });
                });
            });
    }

    fn dismiss(&mut self) {
        self.open = false;
        super::store::write(DISMISSED, Some(""));
    }
}

/// The user agent is all a page is told about the device, and anything can claim anything in it.
/// Nothing rests on it: a browser that lies is offered a download it can ignore.
fn on_android_browser() -> bool {
    #[cfg(target_family = "wasm")]
    {
        web_sys::window()
            .and_then(|window| window.navigator().user_agent().ok())
            .is_some_and(|agent| agent.contains("Android"))
    }
    #[cfg(not(target_family = "wasm"))]
    false
}
