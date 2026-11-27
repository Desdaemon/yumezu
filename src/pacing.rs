//! When the next frame is wanted, and how that request reaches the event loop. See [`Wanted`].

use super::*;

/// How long the window keeps drawing at the display's rate after the last motion, so uneven drag
/// events do not drop the rate to [`IDLE_REDRAW_HZ`] between them.
pub(super) const IDLE_AFTER_SECONDS: f32 = 2.0;
/// Frames a second once only the dashes move. The dashes march by wall clock, so a lower rate makes
/// the marching coarser at the same speed. See [`App::wanted`].
pub(super) const IDLE_REDRAW_HZ: f32 = 30.0;
/// Frames a second while polling a network fetch, with nothing on screen moving.
const POLL_REDRAW_HZ: f32 = 10.0;

/// When the next frame is wanted, decided at the end of each frame. See [`App::wanted`], and
/// [`profile::eager`] for always-on drawing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Wanted {
    /// As soon as the display will take one: something is moving.
    Now,
    /// Not before this long has passed. Nothing is moving fast enough to need the display's rate.
    After(std::time::Duration),
    /// Not until an event arrives: nothing on screen would change.
    Never,
}

impl Wanted {
    /// The sooner of two demands.
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
    /// When the frame just drawn wants the next one, given what is still moving. See [`Wanted`].
    pub(super) fn wanted(&self, data: &AppEntities) -> Wanted {
        if profile::eager() {
            return Wanted::Now;
        }
        [
            (self.still_seconds < IDLE_AFTER_SECONDS).then_some(Wanted::Now),
            // A fade, a blinking caret, a tooltip about to show.
            Some(Wanted::after(
                self.overlay.as_ref().unwrap().gui.repaint_after(),
            )),
            // The dashes keep moving over a settled layout. See [`AppPersistent::focused`].
            (!data.dash_instances.transformations.is_empty()
                && (self.persistent.focused || cfg!(target_family = "wasm")))
            .then(|| Wanted::after_hz(IDLE_REDRAW_HZ)),
            // Only a frame collects resolved work. See [`App::draw`].
            (self.yno.working()
                || matches!(data.atlas, Atlas::Loading(_))
                || data.lod.has_pending_fetch())
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
            // The redraw request below wakes the loop.
            Wanted::Now | Wanted::Never => ControlFlow::Wait,
            Wanted::After(delay) => ControlFlow::wait_duration(delay),
        };
        event_loop.set_control_flow(flow);
        if self.wanted == Wanted::Now {
            self.request_a_frame();
        }
    }

    /// Requests a frame unless one is already pending. On the web build winit serves a request by
    /// cancelling the previous one, so a drag's events would keep cancelling the frame.
    pub(super) fn request_a_frame(&mut self) {
        if self.frame_requested {
            return;
        }
        if let Some(window) = self.ctx.window.as_ref() {
            self.frame_requested = true;
            window.request_redraw();
        }
    }
}
