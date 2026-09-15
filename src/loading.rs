//! The stretch at the start of a run with a window on screen and nothing to draw in it. See
//! [`Dump`] and [`App::draw_loading`].

use super::*;

/// How long the loading frame takes to fade off the graph behind it. See [`App::veil`].
pub(super) const LOADING_FADE_SECONDS: f32 = 0.5;

/// How often the server is polled for what it is building, while there is nothing to draw. A stage
/// lasts tens of seconds, so this is how soon a change is said, not how closely it is followed.
const BUILDING_POLLED_EVERY_SECONDS: f32 = 1.0;
/// How long a run waits before fetching the dump again, after the server said it is building one.
///
/// Longer than the poll above because this one is not for the screen: nothing about the wait looks
/// different for having tried, and the server is being polled about its progress anyway.
const DUMP_POLLED_EVERY_SECONDS: f32 = 3.0;
/// How long a run waits before trying again, after the dump could not be had at all.
///
/// Much longer than the wait above, which waits on a server that answered, where this is a host
/// that is not there. Still short enough to pick up a server coming back while someone is
/// watching.
const DUMP_RETRIED_AFTER_SECONDS: f32 = 10.0;

/// The dump, at whatever stage of arriving it has reached.
///
/// It comes off the network rather than out of the binary, so there is a stretch at the start of a
/// run with a window on screen and nothing to draw in it: see [`App::draw_loading`].
///
/// There is no state for having given up: every reason a dump fails to arrive is one that passes,
/// and none is worth a window the person has to close and open again.
///
/// Kept once it arrives rather than dropped into the entities built from it, a phone rebuilding
/// those every time the app comes back to the screen. See [`App::release`].
pub(super) enum Dump {
    Loading(fetch::Pending<Result<Option<world::Dump>, String>>),
    /// Not this time, and this many seconds until the run tries again.
    ///
    /// Two things put a run here: the server building a dump rather than serving one it is about
    /// to replace, which is `why: None`, or the fetch coming to nothing, where `why` is what went
    /// wrong. Hence the two waits, [`DUMP_POLLED_EVERY_SECONDS`] and
    /// [`DUMP_RETRIED_AFTER_SECONDS`].
    Waiting {
        why: Option<String>,
        until: f32,
    },
    Ready(world::Dump),
}

/// What the server says it is building, for the loading frame to say instead of the plain wait.
///
/// A server with no dump yet answers the fetch with `needs update` rather than a document, so the
/// fetch cannot say how the wait is going. Polled on its own clock, one request at a time. See
/// [`world::building`].
#[derive(Default)]
pub(super) struct Building {
    /// The name of the message the last answer named.
    task: Option<&'static str>,
    pending: Option<fetch::Pending<Option<&'static str>>>,
    /// Seconds until the next poll. Starts at zero, so the first frame of a wait polls.
    until: f32,
}

impl Building {
    fn tick(&mut self, seconds: f32) {
        if let Some(pending) = &self.pending {
            if let Some(said) = pending.take() {
                let moved_on = said.is_some() && said != self.task;
                self.task = said;
                // The only account a run leaves of a wait that can last a minute. Logged as the
                // line on screen rather than the message name behind it, so the two read alike.
                if moved_on {
                    log::info!("the server is building the dump: {}", self.says());
                }
                self.pending = None;
            }
            // One at a time: a request still in flight is the answer to how long to wait for it.
            return;
        }
        self.until -= seconds;
        if self.until <= 0.0 {
            self.until = BUILDING_POLLED_EVERY_SECONDS;
            self.pending = Some(fetch::spawn(world::building()));
        }
    }

    /// The stage the server named, or the plain wait.
    fn says(&self) -> String {
        match self.task {
            Some(said) => super::i18n::format(said, None),
            None => t!("dump-loading"),
        }
    }
}

impl App {
    /// Takes in a dump that has arrived and builds the graph out of it, or settles how long this
    /// run waits before trying again. Does nothing until the fetch in flight answers.
    pub(super) fn receive_dump(&mut self) {
        let Some(loaded) = (match &self.dump {
            Dump::Loading(pending) => pending.take(),
            _ => None,
        }) else {
            return;
        };
        self.dump = match loaded {
            Ok(Some(dump)) => Dump::Ready(dump),
            // Not an error and not a dump: the server is building one.
            Ok(None) => Dump::Waiting {
                why: None,
                until: DUMP_POLLED_EVERY_SECONDS,
            },
            // Not the end of the run: everything that stops a dump arriving is something that
            // passes, so the loading frame goes on saying so and trying again.
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
    /// The frame drawn while there is no graph: the background, and a word about why. The whole of
    /// the app until the dump lands, however many tries that takes.
    ///
    /// Everything the overlay usually reads is built out of the dump, so only the one message is
    /// laid out, over the background the graph is drawn on so the window does not change colour.
    pub(super) fn draw_loading(&mut self) {
        // Whole for as long as this is the frame: what fades is what is left once the graph draws.
        self.veil = 1.0;
        let ctx = self.ctx.wctx.as_ref().unwrap();
        let frame_input = self.ctx.fig.as_mut().unwrap().generate(ctx);
        let window = self.ctx.window.as_ref().unwrap();
        let seconds = (frame_input.elapsed_time as f32 * 1e-3).min(0.05);
        // Only worth polling a server that is answering: an unreachable host is building nothing,
        // and one whose dump arrived has nothing left to say.
        if matches!(
            self.dump,
            Dump::Loading(_) | Dump::Waiting { why: None, .. }
        ) {
            self.building.tick(seconds);
        }
        // The clock every wait ends on: however the last try turned out, the next one is what this
        // run does about it.
        if let Dump::Waiting { until, .. } = &mut self.dump {
            *until -= seconds;
            if *until <= 0.0 {
                self.dump = Dump::Loading(fetch::spawn(world::load(self.revealed)));
            }
        }
        let says = match &self.dump {
            // The dump is here and the graph is not, which leaves one thing being waited for.
            Dump::Ready(_) => t!("yno-loading"),
            Dump::Waiting { why: Some(_), .. } => t!("dump-failed"),
            _ => self.building.says(),
        };
        // What went wrong, for the reader who goes looking. The frame keeps spinning either way,
        // because either way it will try again.
        let failed = match &self.dump {
            Dump::Waiting { why, .. } => why.as_deref(),
            _ => None,
        };
        // Kept for the fade that follows this frame, which carries out whichever line was on it.
        self.said = says.clone();
        let overlay = self.overlay.as_mut().unwrap();
        // The same scale the panel is laid out at, so the message is the size the rest of the
        // interface will be.
        overlay.gui.context().set_zoom_factor(overlay.ui_scale);
        // As in `Overlay::run`: the face Japanese needs is not compiled in, and this frame is the
        // first that may want it.
        let context = overlay.gui.context().clone();
        overlay.japanese.serve(&context);
        overlay
            .gui
            .run(window, |ui| loading_frame(ui.ctx(), &says, failed, 1.0));
        frame_input
            .screen()
            .clear(ClearState::color_and_depth(
                BACKGROUND_COLOR[0],
                BACKGROUND_COLOR[1],
                BACKGROUND_COLOR[2],
                1.0,
                1.0,
            ))
            .write::<std::convert::Infallible>(|| {
                overlay.gui.paint(window);
                Ok(())
            })
            .unwrap();
    }
}
