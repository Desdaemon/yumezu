//! Replacing this build with a newer one, for the desktop packages `cargo packager` makes.
//!
//! Only those: the page reloads and the apk goes through the store, so neither compiles this
//! module -- see the stand-in in `app.rs`.
//!
//! The manifest is a file on the release rather than a service, `latest/download` always resolving
//! to the newest one. So `.github/workflows/release.yml` uploading `latest.json` beside the
//! packages is the whole of the server side.

use cargo_packager_updater::{Config, Update, semver};

use super::i18n::t;

const MANIFEST: &str = "https://github.com/Desdaemon/yumezu/releases/latest/download/latest.json";

/// The key `.github/workflows/release.yml` signs the packages with. Passed at build time rather
/// than written here, so a build that was never given one -- a local `cargo run` -- cannot be
/// talked into installing anything, and draws no update controls at all.
const PUBKEY: Option<&str> = option_env!("YUMEZU_UPDATE_PUBKEY");

enum State {
    /// Nobody has asked this run.
    Never,
    Checking,
    /// Asked, and this build is the newest there is.
    Current,
    Ready(Box<Update>),
    Installing,
    /// Written. The running process is still the old one -- `download_and_install` replaces the
    /// package rather than the memory it was loaded into.
    Installed,
    /// The complaint is logged rather than shown: a person can retry, and cannot act on a TLS
    /// error or a malformed manifest.
    Failed,
}

static STATE: std::sync::Mutex<State> = std::sync::Mutex::new(State::Never);

/// The settings tab's update section, drawn only where there is a package to replace.
pub(super) fn controls(ui: &mut egui::Ui) {
    let Some(pubkey) = PUBKEY else {
        return;
    };
    ui.separator();
    let mut state = STATE.lock().unwrap();
    match &*state {
        State::Never | State::Current | State::Failed => {
            if ui
                .button(t!("update-check"))
                .on_hover_text(t!("update-check-hint"))
                .clicked()
            {
                *state = State::Checking;
                check(pubkey);
            }
            // Under the button rather than in it, like `fetch::clear`'s: the button says what it
            // does, and this says what came of the last press.
            match &*state {
                State::Current => {
                    ui.label(t!("update-current"));
                }
                State::Failed => {
                    ui.label(t!("update-failed"));
                }
                _ => {}
            }
        }
        State::Checking => {
            ui.label(t!("update-checking"));
        }
        State::Ready(update) => {
            ui.label(t!("update-ready", version = update.version.clone()));
            if ui.button(t!("update-install")).clicked() {
                let State::Ready(update) = std::mem::replace(&mut *state, State::Installing) else {
                    unreachable!("the arm this is in matched it");
                };
                install(*update);
            }
        }
        State::Installing => {
            ui.label(t!("update-installing"));
        }
        State::Installed => {
            ui.label(t!("update-installed"));
        }
    }
}

/// Its own thread rather than `fetch::spawn`: the updater's requests are blocking ones, which
/// panic on a tokio runtime's thread.
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
            Ok(Some(update)) => State::Ready(Box::new(update)),
            Ok(None) => State::Current,
            Err(error) => {
                log::warn!("cannot check for an update: {error}");
                State::Failed
            }
        };
    });
}

/// Downloads it, checks it against [`PUBKEY`] and puts it where this build is. Its own thread for
/// the reason in [`check`], and because it is a whole package over the network.
fn install(update: Update) {
    std::thread::spawn(move || {
        *STATE.lock().unwrap() = match update.download_and_install() {
            Ok(()) => State::Installed,
            Err(error) => {
                log::warn!("cannot install the update: {error}");
                State::Failed
            }
        };
    });
}
