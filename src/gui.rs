//! The overlay's own half of the window: the input egui is given, and the painter that draws what
//! it returns.
//!
//! `three_d` ships a `GUI` that does both, and this replaces it. Its pairing builds egui's input
//! out of `three_d`'s own events and then drops egui's [`egui::PlatformOutput`] on the floor,
//! which leaves an input method nowhere to reach: nothing ever allows one, nothing places the
//! candidate window under the caret, and a word being built up before it is committed is never
//! reported at all. What is here instead is the pairing `eframe` is built out of --
//! [`egui_winit::State`] for the input, [`egui_glow::Painter`] for the drawing -- with only the
//! handful of lines that stand between them written here, so that the winit loop and the
//! `three_d` renderer this app already has are left exactly as they were.
//!
//! `eframe` itself is not used because it wants to own the window, the context and the loop, and
//! all three are already owned here by a 3D renderer that has to draw before the overlay does.
//!
//! The page is the one platform winit has no input method for -- its backend's `set_ime_allowed`
//! is an empty function and it never sends [`winit::event::WindowEvent::Ime`]. See
//! [`super::text_agent`], which is the piece of `eframe` that cannot be borrowed rather than
//! reimplemented.

use winit::{event::WindowEvent, event_loop::ActiveEventLoop, window::Window};

pub(crate) struct Gui {
    ctx: egui::Context,
    state: egui_winit::State,
    painter: egui_glow::Painter,
    /// Carried between [`Gui::run`] and [`Gui::paint`], which are two calls because the second
    /// has to happen inside the render target the 3D scene was written to.
    shapes: Vec<egui::epaint::ClippedShape>,
    textures: egui::TexturesDelta,
    pixels_per_point: f32,
    /// See [`super::text_agent`]. The page only.
    #[cfg(target_family = "wasm")]
    agent: super::text_agent::TextAgent,
}

impl Gui {
    /// Builds the overlay against the window's own GL context, which the 3D renderer already
    /// holds: the painter draws into the same surface the scene does, one after the other.
    pub(crate) fn new(
        event_loop: &ActiveEventLoop,
        window: &Window,
        context: &three_d::Context,
    ) -> Self {
        use std::ops::Deref as _;

        // The same arguments `three_d`'s own `GUI` passes: no shader prefix, the version sniffed
        // rather than named, dithering on.
        let painter = egui_glow::Painter::new(context.deref().clone(), "", None, true)
            .expect("egui's painter could not be built on the window's context");
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            // Read off the window every frame by `take_egui_input`, so there is nothing to say
            // here that would not immediately be said again.
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
        }
    }

    pub(crate) fn context(&self) -> &egui::Context {
        &self.ctx
    }

    /// Offers the event to egui. The caller hands it on to the 3D scene regardless: what the
    /// overlay took is settled after the frame by [`egui::Context::wants_pointer_input`], not
    /// here, because a press only becomes the panel's once the panel has been laid out under it.
    pub(crate) fn on_window_event(&mut self, window: &Window, event: &WindowEvent) {
        let _ = self.state.on_window_event(window, event);
    }

    /// Lays the overlay out for this frame. Draw it with [`Gui::paint`].
    pub(crate) fn run(&mut self, window: &Window, run_ui: impl FnMut(&mut egui::Ui)) {
        #[cfg(target_family = "wasm")]
        self.agent.lend_focus(&mut self.state);

        let input = self.state.take_egui_input(window);
        let output = self.ctx.run_ui(input, run_ui);

        // No viewport commands are followed: this app opens one window and never asks for
        // another, so there is nothing for egui to command about the rest.
        #[cfg(target_family = "wasm")]
        self.agent.follow(&self.ctx, output.platform_output.ime);
        // What allows the input method, and what puts the candidate window under the caret rather
        // than in the corner of the screen. Also the cursor icon and the clipboard, neither of
        // which reached the window before.
        self.state
            .handle_platform_output(window, output.platform_output);

        self.shapes = output.shapes;
        self.pixels_per_point = output.pixels_per_point;
        self.textures.append(output.textures_delta);
    }

    /// Draws what the last [`Gui::run`] laid out. Must be called inside the write callback of the
    /// render target the scene was drawn to, which is what puts the overlay over it.
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

    /// Gives the painter's buffers back. Has to be called while the context that holds them is
    /// still there, which is why it is a call rather than a `Drop`: on a phone the context goes
    /// away first unless something says otherwise. See `App::suspended`.
    pub(crate) fn destroy(&mut self) {
        self.painter.destroy();
    }
}
