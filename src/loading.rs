//! The loading screen, drawn whenever the window has no graph: at launch and while one is rebuilt.
//! See [`Dump`] and [`App::draw_loading`].

use super::*;

/// How long the loading screen takes to fade out over the graph. See [`App::veil`].
pub(super) const LOADING_FADE_SECONDS: f32 = 0.5;

/// How often the server is polled for build progress while there is no graph. A stage lasts tens
/// of seconds, so this only bounds how late a stage change shows.
const BUILDING_POLLED_EVERY_SECONDS: f32 = 1.0;
/// How long to wait before fetching the dump again while the server builds one; longer than the
/// progress poll, which already reports the wait.
const DUMP_POLLED_EVERY_SECONDS: f32 = 3.0;
/// How long to wait before retrying after the dump fetch failed outright: longer than the waits
/// above, short enough to see a server come back.
const DUMP_RETRIED_AFTER_SECONDS: f32 = 10.0;

/// The dump through each stage of arrival; every failure is retried. Kept once built, since a
/// phone rebuilds the entities on every return to the foreground; see [`App::release`].
pub(super) enum Dump {
    Loading(fetch::Pending<Result<Option<world::Dump>, String>>),
    /// Seconds until the next try. `why` is `None` while the server builds a dump (see
    /// [`DUMP_POLLED_EVERY_SECONDS`]), or else the failure (see [`DUMP_RETRIED_AFTER_SECONDS`]).
    Waiting {
        why: Option<String>,
        until: f32,
    },
    Ready(world::Dump),
}

/// The server's build stage for the loading screen, since the dump fetch only answers
/// `needs update`. Polled on a separate clock, one request at a time. See [`world::building`].
#[derive(Default)]
pub(super) struct Building {
    /// The message name from the last answer.
    task: Option<&'static str>,
    pending: Option<fetch::Pending<Option<&'static str>>>,
    /// Seconds until the next poll. Starts at zero, so the first frame of a wait polls.
    until: f32,
}

impl Building {
    fn update(&mut self, seconds: f32) {
        if let Some(pending) = &self.pending {
            if let Some(said) = pending.take() {
                let moved_on = said.is_some() && said != self.task;
                self.task = said;
                // Logged as the on-screen line, so the log and the screen read alike.
                if moved_on {
                    log::info!("the server is building the dump: {}", self.says());
                }
                self.pending = None;
            }
            return;
        }
        self.until -= seconds;
        if self.until <= 0.0 {
            self.until = BUILDING_POLLED_EVERY_SECONDS;
            self.pending = Some(fetch::spawn(world::building()));
        }
    }

    /// The stage the server named, or the generic wait message.
    fn says(&self) -> String {
        match self.task {
            Some(said) => super::i18n::format(said, None),
            None => t!("dump-loading"),
        }
    }
}

impl App {
    /// Builds the graph from an arrived dump, or schedules the next try. Does nothing until the
    /// pending fetch answers.
    pub(super) fn receive_dump(&mut self) {
        let Some(loaded) = (match &self.dump {
            Dump::Loading(pending) => pending.take(),
            _ => None,
        }) else {
            return;
        };
        self.dump = match loaded {
            Ok(Some(dump)) => Dump::Ready(dump),
            // The server is building a dump.
            Ok(None) => Dump::Waiting {
                why: None,
                until: DUMP_POLLED_EVERY_SECONDS,
            },
            // Every failure here is transient, so the frame keeps retrying.
            Err(error) => {
                log::warn!("{error}");
                Dump::Waiting {
                    why: Some(error),
                    until: DUMP_RETRIED_AFTER_SECONDS,
                }
            }
        };
        self.build();
    }
    /// The loading screen, the whole app until the dump arrives: one message over the graph's
    /// background, so the window colour does not change.
    pub(super) fn draw_loading(&mut self) {
        self.veil = 1.0;
        let ctx = self.ctx.wctx.as_ref().unwrap();
        let frame_input = self.ctx.fig.as_mut().unwrap().generate(ctx);
        let window = self.ctx.window.as_ref().unwrap();
        let seconds = frame::clamped_frame_seconds(&frame_input);
        // An unreachable host is building nothing, and a delivered dump has nothing left to say.
        if matches!(
            self.dump,
            Dump::Loading(_) | Dump::Waiting { why: None, .. }
        ) {
            self.building.update(seconds);
        }
        if let Dump::Waiting { until, .. } = &mut self.dump {
            *until -= seconds;
            if *until <= 0.0 {
                self.dump = Dump::Loading(fetch::spawn(world::load(self.revealed)));
            }
        }
        let says = match &self.dump {
            // The dump is here; only the account is pending.
            Dump::Ready(_) => t!("yno-loading"),
            Dump::Waiting { why: Some(_), .. } => t!("dump-failed"),
            _ => self.building.says(),
        };
        // The spinner keeps turning on failure, since a retry follows.
        let failed = match &self.dump {
            Dump::Waiting { why, .. } => why.as_deref(),
            _ => None,
        };
        // Kept for the fade-out once the graph arrives.
        self.said = says.clone();
        let overlay = self.overlay.as_mut().unwrap();
        overlay.gui.context().set_zoom_factor(overlay.ui_scale);
        // As in `Overlay::update`: the Japanese face is not compiled in.
        let context = overlay.gui.context().clone();
        overlay.japanese.serve(&context);
        overlay
            .gui
            .update(window, |ui| loading_frame(ui.ctx(), &says, failed, 1.0));
        let screen = frame_input.screen();
        screen.clear(ClearState::color_and_depth(
            BACKGROUND_COLOR[0],
            BACKGROUND_COLOR[1],
            BACKGROUND_COLOR[2],
            1.0,
            1.0,
        ));
        frame::write_to_screen(&screen, || {
            overlay.gui.paint(window);
        });
    }
}
