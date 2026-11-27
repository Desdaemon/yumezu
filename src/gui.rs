//! egui input through `egui_winit` and drawing through `egui_glow`, paired by hand: `three_d`'s
//! `GUI` drops [`egui::PlatformOutput`], which `egui_winit` and the web build's `text_agent` read.

use winit::{event::WindowEvent, event_loop::ActiveEventLoop, window::Window};

pub(crate) struct Gui {
    ctx: egui::Context,
    state: egui_winit::State,
    painter: egui_glow::Painter,
    /// Carried from [`Gui::update`] to [`Gui::paint`]: the paint must happen inside the render
    /// target the 3D scene was written to.
    shapes: Vec<egui::epaint::ClippedShape>,
    textures: egui::TexturesDelta,
    pixels_per_point: f32,
    /// egui's repaint delay: zero while the panel animates, [`Duration::MAX`] while the overlay is
    /// unchanged.
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

        // Same as `three_d`'s `GUI`: no shader prefix, sniffed version, dither.
        let painter = egui_glow::Painter::new(context.deref().clone(), "", None, true)
            .expect("egui's painter could not be built on the window's context");
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            // Pixels-per-point, which `take_egui_input` reads every frame.
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

    pub(crate) fn repaint_after(&self) -> std::time::Duration {
        self.repaint_after
    }

    /// The caller also passes the event to the 3D scene: whether the overlay consumed the event is
    /// known after layout, from [`egui::Context::wants_pointer_input`].
    pub(crate) fn on_window_event(&mut self, window: &Window, event: &WindowEvent) {
        let _ = self.state.on_window_event(window, event);
    }

    pub(crate) fn update(&mut self, window: &Window, build_ui: impl FnMut(&mut egui::Ui)) {
        #[cfg(target_family = "wasm")]
        self.agent.lend_focus(&mut self.state);

        let input = self.state.take_egui_input(window);
        // This app opens only the root viewport.
        let output = self.ctx.run_ui(input, build_ui);
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

    /// Call inside the write callback of the scene's render target, so the overlay lands over the
    /// scene. Returns the draw call count, one per clipped primitive.
    pub(crate) fn paint(&mut self, window: &Window) -> usize {
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
        primitives.len()
    }

    /// An explicit call, since on a phone the GL context holding the buffers is lost first. See
    /// `App::suspended`.
    pub(crate) fn destroy(&mut self) {
        self.painter.destroy();
    }
}
