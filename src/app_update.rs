//! Self-update for the `cargo packager` desktop builds, from `latest.json` on the newest GitHub
//! release; the web build and the apk compile a stand-in in `app.rs`.

use cargo_packager_updater::{Config, Update, semver};

use super::prelude::*;

const MANIFEST: &str = "https://github.com/Desdaemon/yumezu/releases/latest/download/latest.json";

/// The key `.github/workflows/release.yml` signs the packages with, set at build time. A build
/// without the key, such as a local `cargo run`, installs nothing.
const PUBKEY: Option<&str> = option_env!("YUMEZU_UPDATE_PUBKEY");

enum State {
    Never,
    Checking,
    Current,
    Ready(Box<Update>),
    Installing,
    /// Written to disk; the running process stays the old version until restarted.
    Installed,
    /// The error is only logged: the user can retry, and cannot act on a TLS error or a malformed
    /// manifest.
    Failed,
}

static STATE: std::sync::Mutex<State> = std::sync::Mutex::new(State::Never);

/// The settings tab's app update section, drawn only where there is a package to replace.
pub(super) fn controls(ui: &mut egui::Ui) {
    let Some(pubkey) = PUBKEY else {
        return;
    };
    ui.separator();
    let mut state = STATE.lock().unwrap();
    match &*state {
        State::Never | State::Current | State::Failed => {
            if ui
                .button(t!("app-update-check"))
                .on_hover_text(t!("app-update-check-hint"))
                .clicked()
            {
                *state = State::Checking;
                check(pubkey);
            }
            match &*state {
                State::Current => {
                    ui.label(t!("app-update-current"));
                }
                State::Failed => {
                    ui.label(t!("app-update-failed"));
                }
                _ => {}
            }
        }
        State::Checking => {
            ui.label(t!("app-update-checking"));
        }
        State::Ready(app_update) => {
            ui.label(t!("app-update-ready", version = app_update.version.clone()));
            if ui.button(t!("app-update-install")).clicked() {
                let State::Ready(app_update) = std::mem::replace(&mut *state, State::Installing)
                else {
                    unreachable!("the arm this is in matched it");
                };
                install(*app_update);
            }
        }
        State::Installing => {
            ui.label(t!("app-update-installing"));
        }
        State::Installed => {
            ui.label(t!("app-update-installed"));
        }
    }
}

/// A dedicated thread: the updater's blocking requests panic on a tokio runtime thread.
fn check(pubkey: &'static str) {
    let config = Config {
        endpoints: vec![MANIFEST.parse().expect("the manifest URL does not parse")],
        pubkey: pubkey.to_owned(),
        windows: None,
    };
    let current = env!("CARGO_PKG_VERSION");
    std::thread::spawn(move || {
        let found = current
            .parse()
            .map_err(|error: semver::Error| error.to_string())
            .and_then(|current| {
                cargo_packager_updater::check_update(current, config)
                    .map_err(|error| error.to_string())
            });
        *STATE.lock().unwrap() = match found {
            Ok(Some(app_update)) => State::Ready(Box::new(app_update)),
            Ok(None) => State::Current,
            Err(error) => {
                log::warn!("cannot check for an app update: {error}");
                State::Failed
            }
        };
    });
}

/// Downloads the package, verifies the signature against [`PUBKEY`] and installs over this build,
/// on a dedicated thread as in [`check`].
fn install(app_update: Update) {
    std::thread::spawn(move || {
        *STATE.lock().unwrap() = match app_update.download_and_install() {
            Ok(()) => State::Installed,
            Err(error) => {
                log::warn!("cannot install the app update: {error}");
                State::Failed
            }
        };
    });
}
