//! The release package for this browser's platform, offered in a bar; web build only.

use egui_material_icons::MaterialIcon;
use egui_material_icons::icons::{
    ICON_ANDROID, ICON_CLOSE, ICON_COMPUTER, ICON_DESKTOP_MAC, ICON_DESKTOP_WINDOWS,
};

use super::prelude::*;

/// The releases page: `.github/workflows/release.yml` stamps each asset with a version, so no
/// asset URL stays the newest.
pub(super) const RELEASES: &str = "https://github.com/Desdaemon/yumezu/releases/latest";

// The value is always empty.
const DISMISSED: &str = "download-dismissed";

/// A platform `.github/workflows/release.yml` builds a package for; see [`Platform::detected`].
#[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
#[derive(Clone, Copy)]
pub(super) enum Platform {
    Windows,
    MacOs,
    Linux,
    Android,
}

impl Platform {
    /// Spelled the same in every language, so kept out of `locales/`.
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

    /// `None` for an iPhone, a Chromebook, an unrecognised user agent, and off the web build. A
    /// wrong guess only offers a package the user can ignore.
    pub(super) fn detected() -> Option<Self> {
        #[cfg(target_family = "wasm")]
        {
            let navigator = web_sys::window()?.navigator();
            let agent = navigator.user_agent().ok()?;
            // Android before Linux, since Android user agents also name Linux.
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
                // An iPad on the desktop site says `Macintosh`; no Mac reports touch points.
                return (navigator.max_touch_points() <= 1).then_some(Self::MacOs);
            }
            agent.contains("Linux").then_some(Self::Linux)
        }
        #[cfg(not(target_family = "wasm"))]
        None
    }
}

pub(super) struct Offer {
    /// The offered platform; `None` without a bar: nothing to offer, or the offer answered.
    platform: Option<Platform>,
}

impl Offer {
    pub(super) fn new() -> Self {
        Self {
            platform: Platform::detected().filter(|_| !super::store::read_flag(DISMISSED)),
        }
    }

    /// The bar sits at the top of the safe area, below a phone's status bar; the bottom belongs to
    /// the message box.
    pub(super) fn show(&mut self, ctx: &egui::Context, insets: super::ui::Insets) {
        let Some(platform) = self.platform else {
            return;
        };
        let anchor = egui::Align2::CENTER_TOP;
        super::anchored_popup(ctx, "download offer", anchor, insets, |ui| {
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
    }

    fn dismiss(&mut self) {
        self.platform = None;
        super::store::write_flag(DISMISSED, true);
    }
}
