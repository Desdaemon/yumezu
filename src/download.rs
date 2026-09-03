//! The Android app, offered to the phone that could be running it instead.
//!
//! The page is the whole of this app on a desktop, but on a phone it is the slower half of one:
//! the apk served at [`URL`] draws the same graph with nothing between it and the device. So a
//! phone that arrives at the page is told the apk exists, in a bar it can put away for good.
//!
//! Only on the page, and only on Android. A desktop has nothing here to install, an iPhone cannot
//! install this, and the phone build is already the thing being offered -- so on all three there
//! is nothing to say, and [`on_android_browser`] is what tells them apart.

use egui_material_icons::icons::{ICON_ANDROID, ICON_CLOSE};
use three_d::egui;

/// Where the apk is served from. Relative to the page, so it is whichever host served the page.
const URL: &str = "/android";

/// What the dismissal is kept under. The presence of the value *is* the answer, so there is
/// nothing to parse and nothing that can be half-written. See [`super::store`].
const DISMISSED: &str = "download-dismissed";

/// The bar, and whether it is still being shown.
pub(super) struct Offer {
    /// Whether it is on screen this frame. False for good once it is put away, and false from the
    /// start on every platform and every later run that has no offer to make.
    open: bool,
}

impl Offer {
    /// Opens on a phone reading the page that was never told to stop offering.
    pub(super) fn new() -> Self {
        Self {
            open: on_android_browser() && super::store::read(DISMISSED).is_none(),
        }
    }

    /// Draws it, if there is anything to offer.
    ///
    /// `insets` is what the system's own furniture covers: the bar stands off the bottom of the
    /// safe area rather than the bottom of the window, which on a phone is behind the navigation.
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
                            .button(format!("{}  Get the Android app", ICON_ANDROID.codepoint))
                            .clicked()
                        {
                            super::open_in_browser(URL);
                            // Taking the offer ends it as surely as refusing it does: whatever
                            // comes of the download, there is no second apk to hand out.
                            self.dismiss();
                        }
                        if ui
                            .button(ICON_CLOSE)
                            .on_hover_text("Don't show this again")
                            .clicked()
                        {
                            self.dismiss();
                        }
                    });
                });
            });
    }

    /// Takes the bar off the screen and writes that it is not to come back.
    fn dismiss(&mut self) {
        self.open = false;
        super::store::write(DISMISSED, Some(""));
    }
}

/// Whether this is a device that could run the apk: the page, on Android.
///
/// The user agent is the only thing a page is told about the device reading it. It is a string
/// anything can claim anything in, but nothing here rests on it: a browser that lies about being
/// Android is offered a download it can ignore, and one that lies the other way is left with the
/// page it already has.
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
