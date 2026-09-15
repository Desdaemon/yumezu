//! When the next frame is wanted, and how that request reaches the event loop. See [`Wanted`].

use super::*;

/// How long after the last thing that moved the window goes on being drawn at the display's rate.
///
/// Without it the pacing would follow the input rather than the view: a drag's events arrive
/// unevenly, so a frame that saw none would drop to [`IDLE_REDRAW_HZ`] and the next go back up.
pub(super) const IDLE_AFTER_SECONDS: f32 = 2.0;
/// Frames a second the window falls back to when the dashes are the only thing still moving.
///
/// A settled layout nobody is touching would otherwise be redrawn at the display's own rate for as
/// long as the app is open. The dashes march by wall clock, so a slower rate makes the marching
/// coarser and not slower. See [`App::wanted`].
pub(super) const IDLE_REDRAW_HZ: f32 = 30.0;
/// Frames a second while the only reason to draw one is to see whether something fetched over
/// the network has landed. Nothing on screen is moving, so this is a poll and not an animation.
const POLL_REDRAW_HZ: f32 = 10.0;

/// When the next frame is wanted, which is what a frame ends by working out.
///
/// The window is redrawn on demand rather than continuously: see [`App::wanted`] for what is
/// wanting one, and [`profile::eager`] for the switch that puts the old always-on behaviour back.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Wanted {
    /// As soon as the display will take one: something is moving.
    Now,
    /// Not before this long has passed. Nothing is moving fast enough to need the display's rate.
    After(std::time::Duration),
    /// Not until an event arrives. Every pixel would come out the way it already is.
    Never,
}

impl Wanted {
    /// The stricter of two demands, a frame being drawn for whichever of them wants one first.
    fn or_sooner(self, other: Self) -> Self {
        self.min(other)
    }

    fn after_hz(hz: f32) -> Self {
        Self::After(std::time::Duration::from_secs_f32(1.0 / hz))
    }

    /// Zero reads as [`Wanted::Now`] and [`std::time::Duration::MAX`] as [`Wanted::Never`], which
    /// is how egui spells both.
    fn after(delay: std::time::Duration) -> Self {
        if delay.is_zero() {
            Self::Now
        } else if delay == std::time::Duration::MAX {
            Self::Never
        } else {
            Self::After(delay)
        }
    }
}

impl App {
    /// When the frame just drawn wants the next one, given what it left moving. See [`Wanted`].
    pub(super) fn wanted(&self, data: &AppEntities) -> Wanted {
        if profile::eager() {
            return Wanted::Now;
        }
        [
            // Moving, or not long enough ago to start pacing the view by anything but the
            // display. See [`IDLE_AFTER_SECONDS`].
            (self.still < IDLE_AFTER_SECONDS).then_some(Wanted::Now),
            // A fade in the panel, a blinking caret, a tooltip about to show itself.
            Some(Wanted::after(
                self.overlay.as_ref().unwrap().gui.repaint_after(),
            )),
            // The one thing that goes on moving over a settled layout, and so the only reason
            // a graph nothing else is happening to is drawn at all. See [`AppStatics::focused`].
            (!data.dash_instances.transformations.is_empty()
                && (self.statics.focused || cfg!(target_family = "wasm")))
            .then(|| Wanted::after_hz(IDLE_REDRAW_HZ)),
            // Nothing is moving; a frame is drawn only because reading what has resolved is
            // something only a frame does. See [`App::draw`].
            (self.yno.working()
                || matches!(data.atlas, Atlas::Loading(_))
                || data.detail.pending())
            .then(|| Wanted::after_hz(POLL_REDRAW_HZ)),
        ]
        .into_iter()
        .flatten()
        .fold(Wanted::Never, Wanted::or_sooner)
    }

    /// Turns what the frame just drawn wanted into how the loop waits. See [`Wanted`].
    pub(super) fn pace(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use winit::event_loop::ControlFlow;
        let flow = match self.wanted {
            // The redraw request below is what wakes the loop; the wait is what it falls back to
            // once the frame has been drawn.
            Wanted::Now | Wanted::Never => ControlFlow::Wait,
            Wanted::After(delay) => ControlFlow::wait_duration(delay),
        };
        event_loop.set_control_flow(flow);
        if self.wanted == Wanted::Now {
            self.request_a_frame();
        }
    }

    /// Requests a frame, unless one has already been requested and not yet drawn.
    ///
    /// On the page a request is an animation frame, and winit serves a second by cancelling the
    /// first, so requesting on every event of a drag cancels the frame that was about to be drawn
    /// over and over.
    pub(super) fn request_a_frame(&mut self) {
        if self.requested {
            return;
        }
        if let Some(window) = self.ctx.window.as_ref() {
            self.requested = true;
            window.request_redraw();
        }
    }
}
