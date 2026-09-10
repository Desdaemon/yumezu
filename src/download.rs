//! Which package this browser has a platform for, and the bar offering it.
//!
//! Only on the page: a run that is already the app has nothing to offer, and the platforms with no
//! package here are offered nothing rather than something they cannot run.

use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ANDROID, ICON_CLOSE, ICON_COMPUTER, ICON_DESKTOP_MAC, ICON_DESKTOP_WINDOWS,
};

use super::i18n::t;

/// The releases page rather than a package: `.github/workflows/release.yml` stamps every asset
/// with the version that built it, so no one address stays pointed at the newest, and the page
/// carries the install notes each of them needs anyway.
pub(super) const RELEASES: &str = "https://github.com/Desdaemon/yumezu/releases/latest";

// Presence is the whole answer; the value is always empty.
const DISMISSED: &str = "download-dismissed";

/// A platform `.github/workflows/release.yml` builds a package for. See [`Platform::detected`],
/// which is how anything gets one.
// Off the page nothing is ever offered, so nothing there constructs one either.
#[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
#[derive(Clone, Copy)]
pub(super) enum Platform {
    Windows,
    MacOs,
    Linux,
    Android,
}

impl Platform {
    /// Spelled the same in every language, which is why these are here rather than in `locales/`.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOs => "macOS",
            Self::Linux => "Linux",
            Self::Android => "Android",
        }
    }

    pub(super) fn icon(self) -> MaterialIcon {
        match self {
            Self::Windows => ICON_DESKTOP_WINDOWS,
            Self::MacOs => ICON_DESKTOP_MAC,
            Self::Linux => ICON_COMPUTER,
            Self::Android => ICON_ANDROID,
        }
    }

    /// `None` for an iPhone, a Chromebook, an agent saying nothing recognisable, and off the page
    /// entirely -- a run that is already a package has nothing to be offered. Nothing rests on the
    /// answer: an agent that lies is offered a package it can ignore.
    pub(super) fn detected() -> Option<Self> {
        #[cfg(target_family = "wasm")]
        {
            let navigator = web_sys::window()?.navigator();
            let agent = navigator.user_agent().ok()?;
            // Android before Linux, whose name it also carries.
            if agent.contains("Android") {
                return Some(Self::Android);
            }
            if agent.contains("iPhone") || agent.contains("iPad") {
                return None;
            }
            if agent.contains("Windows") {
                return Some(Self::Windows);
            }
            if agent.contains("Macintosh") {
                // An iPad asked for the desktop site says `Macintosh` and drops `iPad`. No Mac
                // reports touch points, so this is what still tells the two apart.
                return (navigator.max_touch_points() <= 1).then_some(Self::MacOs);
            }
            agent.contains("Linux").then_some(Self::Linux)
        }
        #[cfg(not(target_family = "wasm"))]
        None
    }
}

pub(super) struct Offer {
    /// What the bar is offering, and `None` where there is no bar: nothing to offer, or an offer
    /// already answered.
    platform: Option<Platform>,
}

impl Offer {
    pub(super) fn new() -> Self {
        Self {
            platform: Platform::detected().filter(|_| super::store::read(DISMISSED).is_none()),
        }
    }

    /// The bar stands off the bottom of the safe area, not of the window, which on a phone sits
    /// behind the navigation bar.
    pub(super) fn show(&mut self, ctx: &egui::Context, insets: egui::Margin) {
        let Some(platform) = self.platform else {
            return;
        };
        egui::Area::new(egui::Id::new("download offer"))
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
                                platform.icon().codepoint,
                                t!("download-app", platform = platform.name())
                            ))
                            .clicked()
                        {
                            super::open_in_browser(RELEASES);
                            // Taking the offer answers it: there is nothing further to hand out.
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
        self.platform = None;
        super::store::write(DISMISSED, Some(""));
    }
}
