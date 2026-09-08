//! The overlay's own half of the window: the input egui is given, and the painter that draws what
//! it returns.
//!
//! Hand-paired from [`egui_winit::State`] and [`egui_glow::Painter`] rather than taken from
//! either crate above them. `three_d`'s own `GUI` drops egui's [`egui::PlatformOutput`], which
//! leaves an input method nowhere to reach -- never allowed, never placed under the caret, and an
//! uncommitted word never reported. `eframe` wants the window, the context and the loop, all
//! three of which the 3D renderer that draws first already owns.
//!
//! The page has no winit input method at all: its `set_ime_allowed` is empty and it never sends
//! [`winit::event::WindowEvent::Ime`]. See [`super::text_agent`].

use winit::{event::WindowEvent, event_loop::ActiveEventLoop, window::Window};

pub(crate) struct Gui {
    ctx: egui::Context,
    state: egui_winit::State,
    painter: egui_glow::Painter,
    /// Carried from [`Gui::run`] to [`Gui::paint`]: the paint must happen inside the render
    /// target the 3D scene was written to.
    shapes: Vec<egui::epaint::ClippedShape>,
    textures: egui::TexturesDelta,
    pixels_per_point: f32,
    /// How long egui is content to wait before it is drawn again: zero while something in the
    /// panel is animating, [`Duration::MAX`] when the overlay would come out identical. What
    /// stops [`super::app::App`] idling the window over a fade or a blinking caret.
    repaint_after: std::time::Duration,
    #[cfg(target_family = "wasm")]
    agent: super::text_agent::TextAgent,
}

impl Gui {
    /// Builds the overlay on the window's GL context, the same one the 3D renderer draws to.
    pub(crate) fn new(
        event_loop: &ActiveEventLoop,
        window: &Window,
        context: &three_d::Context,
    ) -> Self {
        use std::ops::Deref as _;

        // The arguments `three_d`'s own `GUI` passes: no shader prefix, sniffed version, dither.
        let painter = egui_glow::Painter::new(context.deref().clone(), "", None, true)
            .expect("egui's painter could not be built on the window's context");
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            // Pixels-per-point, which `take_egui_input` reads off the window every frame anyway.
            None,
            event_loop.system_theme(),
            Some(painter.max_texture_side()),
        );
        Self {
            #[cfg(target_family = "wasm")]
            agent: super::text_agent::TextAgent::attach(window)
                .expect("the page would not take the text element the overlay types into"),
            ctx,
            state,
            painter,
            shapes: Vec::new(),
            textures: Default::default(),
            pixels_per_point: window.scale_factor() as f32,
            repaint_after: std::time::Duration::ZERO,
        }
    }

    pub(crate) fn context(&self) -> &egui::Context {
        &self.ctx
    }

    /// See [`Gui::repaint_after`].
    pub(crate) fn repaint_after(&self) -> std::time::Duration {
        self.repaint_after
    }

    /// The caller passes the event to the 3D scene regardless: whether the overlay took it is
    /// settled after layout by [`egui::Context::wants_pointer_input`], because a press only
    /// becomes the panel's once the panel has been laid out under it.
    pub(crate) fn on_window_event(&mut self, window: &Window, event: &WindowEvent) {
        let _ = self.state.on_window_event(window, event);
    }

    pub(crate) fn run(&mut self, window: &Window, run_ui: impl FnMut(&mut egui::Ui)) {
        #[cfg(target_family = "wasm")]
        self.agent.lend_focus(&mut self.state);

        let input = self.state.take_egui_input(window);
        // Only the root viewport is read: this app opens one window and never asks for another.
        let output = self.ctx.run_ui(input, run_ui);
        self.repaint_after = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |it| it.repaint_delay);

        #[cfg(target_family = "wasm")]
        self.agent.follow(&self.ctx, output.platform_output.ime);
        self.state
            .handle_platform_output(window, output.platform_output);

        self.shapes = output.shapes;
        self.pixels_per_point = output.pixels_per_point;
        self.textures.append(output.textures_delta);
    }

    /// Must be called inside the write callback of the render target the scene was drawn to, so
    /// the overlay lands over it.
    pub(crate) fn paint(&mut self, window: &Window) {
        let shapes = std::mem::take(&mut self.shapes);
        let mut textures = std::mem::take(&mut self.textures);
        for (id, delta) in &textures.set {
            self.painter.set_texture(*id, delta);
        }
        let primitives = self.ctx.tessellate(shapes, self.pixels_per_point);
        self.painter.paint_primitives(
            window.inner_size().into(),
            self.pixels_per_point,
            &primitives,
        );
        for id in textures.free.drain(..) {
            self.painter.free_texture(id);
        }
    }

    /// A call rather than a `Drop` because on a phone the context holding the buffers goes away
    /// first. See `App::suspended`.
    pub(crate) fn destroy(&mut self) {
        self.painter.destroy();
    }
}
