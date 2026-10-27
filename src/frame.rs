//! One frame of the graph, from what arrived since the last one through to the passes that draw
//! it. See [`App::draw`].

use super::*;

impl App {
    /// One frame: takes in whatever arrived, moves the camera, steps the layout, and draws it.
    /// Answers whether it got as far as rebuilding the geometry, which [`FrameStats`] counts.
    pub(super) fn draw(&mut self) -> bool {
        // The loading frame below returns through here, and it animates.
        self.wanted = Wanted::Now;
        // A resumed session needs its account before the graph is built.
        self.yno.poll();
        if self.yno.restated() {
            self.retire_graph();
            self.build();
        }
        // `world::hide` dropped these worlds on read, so revealing them takes another dump.
        if !self.revealed && self.code.taken() {
            self.revealed = true;
            self.retire_graph();
            self.dump = Dump::Loading(fetch::spawn(world::load(true)));
        }
        if self.data.is_none() {
            self.receive_dump();
            if self.data.is_none() {
                self.draw_loading();
                return false;
            }
        }
        let ctx = self.ctx.wctx.as_ref().unwrap();
        let mut frame_input = self.ctx.fig.as_mut().unwrap().generate(ctx);
        let window = self.ctx.window.as_ref().unwrap();
        let seconds = clamped_frame_seconds(&frame_input);

        if self.veil > 0.0 {
            self.veil = (self.veil - seconds / LOADING_FADE_SECONDS).max(0.0);
        }
        let fading = (self.veil > 0.0).then(|| (self.said.clone(), self.veil));
        let data = self.data.as_mut().unwrap();
        self.statics.camera.set_viewport(frame_input.viewport);
        let dollied = profile::pan_aside(&mut self.statics, data);
        let account = &mut self.yno;
        // The whole game, which the settings tab measures against; the graph may be the frontier.
        let dump = match &self.dump {
            Dump::Ready(dump) => Some(dump),
            _ => None,
        };
        if self
            .overlay
            .as_mut()
            .unwrap()
            .run(window, &mut frame_input, data, account, dump, fading)
        {
            // A flat layout has no depth for repulsion to reinflate; restarting beats reseeding.
            scatter(data);
            if data.dimensions() == Dimensions::Two {
                // Squared before rotation locks, or an oblique camera could never straighten.
                self.statics.face_plane();
            }
        }
        // Ahead of the pan and orbit, which take the left-button motion this leaves.
        data.track_gesture(
            &self.statics.camera,
            &mut frame_input.events,
            self.statics.touches.pinching,
        );
        if data.dimensions() == Dimensions::Two {
            // Swallowing the orbit drag leaves the zoom and the pan.
            lock_rotation(&mut frame_input.events);
        }
        // three-d reads the finger gap as a wheel, and the midpoint's travel is the pan.
        let dragged = self
            .statics
            .touches
            .take_travel(frame_input.device_pixel_ratio);
        // After the overlay, which settles whether the keys type into the search box.
        let typing = self.overlay.as_ref().unwrap().keyboard;
        self.statics.walk.track(&frame_input.events, typing);
        match typing {
            true => self.code.forget(),
            false => frame_input
                .events
                .iter()
                .filter_map(|event| match event {
                    Event::Text(typed) => Some(typed.chars()),
                    _ => None,
                })
                .flatten()
                .for_each(|key| self.code.typed(key)),
        }
        let (across, into) = self.statics.walk.travel(seconds);
        let walking = across != 0.0 || into != 0.0;
        let panned = self
            .statics
            .pan(&mut frame_input.events, frame_input.device_pixel_ratio)
            | self.statics.pan_by(dragged, frame_input.device_pixel_ratio)
            | self
                .statics
                .pan_by((across, 0.0), frame_input.device_pixel_ratio)
            | self.statics.dolly_by(into, frame_input.device_pixel_ratio);
        let orbited = self
            .statics
            .control
            .handle_events(&mut self.statics.camera, &mut frame_input.events);
        // A hand on the camera abandons framing and lean; `panned` includes walk and dolly.
        if panned || orbited {
            data.framing = false;
            data.leaning_at = None;
        }
        let orbiting = matches!(data.gesture, Some(Gesture::Orbiting))
            && data.dimensions() == Dimensions::Three;
        self.statics
            .track_cursor(self.ctx.window.as_ref().unwrap(), orbiting);
        if data.framing {
            // Every frame: the layout is still moving. Arrivals first, then the selection.
            data.framing = match data.arrival_bounds() {
                Some(bounds) => {
                    // Held after the camera catches up: the arrival is still moving.
                    self.statics.ease_to_frame(&bounds, seconds);
                    true
                }
                None => match data.framing_bounds() {
                    Some(bounds) => {
                        let easing = self.statics.ease_to_frame(&bounds, seconds);
                        // The tree is still spreading.
                        easing || (data.selected.is_none() && !data.graph.is_settled())
                    }
                    None => false,
                },
            };
        }

        // Latched: a lean outlives the hover, and only taking the camera stops it.
        if let Some(world) = data.pointed {
            data.leaning_at = Some(world);
        }
        // A framing move owns the camera while it runs.
        let leaning = self.statics.lean_toward(
            (self.overlay.as_ref().unwrap().leaning && !data.framing)
                .then(|| data.leaning_at.and_then(|world| data.world_at(world)))
                .flatten(),
            data.dimensions(),
            seconds,
        );

        data.pull_grabbed_node(&self.statics.camera);
        data.receive_atlas(seconds, ctx, self.overlay.as_ref().unwrap().gui.context());
        // Unclamped: the layout caps its own catch-up.
        let stepping = web_time::Instant::now();
        let stepped = data.graph.update(frame_seconds(&frame_input));
        self.stats.stepped(stepping.elapsed());
        // Billboards date when the camera turns.
        let turned = data.billboard != billboard(&self.statics.camera);
        // Clamped, so an arrival is not half over before it is drawn.
        let arriving = data.arrivals.tick(seconds);
        let recolored = std::mem::take(&mut data.recolored);
        // `repaint` uploads colors itself, leaving the geometry.
        let laid_out = stepped || arriving || profile::eager();
        let moved = laid_out || turned || recolored;
        if laid_out || turned {
            data.rebuild_instances(&self.statics.camera, laid_out);
        }
        // See [`AppEntities::march_dashes`].
        data.march_dashes(seconds);
        // These also collect arrived pictures, so a pending fetch runs them even if nothing moved.
        let reading = data.detail.pending();
        // After the instances, whose colors the full pictures borrow.
        if moved || reading {
            let magnified = data.magnified(&self.statics.camera, frame_input.viewport);
            data.detail.track(ctx, &magnified);
            data.place_unvisited(ctx, &self.statics.camera);
        }
        // The pointer can move the glow between rows while nothing else moves.
        if moved || data.glowing != data.pointed {
            data.glowing = data.pointed;
            data.aim_glow(&self.statics.camera);
        }

        if let Some(texture) = data.backdrop.texture.as_mut() {
            texture.transformation = panorama_transform(
                frame_input.viewport,
                frame_input.device_pixel_ratio,
                self.statics.camera.view_direction(),
            );
        }
        let screen = frame_input.screen();
        // Depth only: the backdrop overwrites every pixel. Measured on an IMR only; on a TBDR
        // (phones, Apple silicon) the clear is the LoadOp, and dropping it reloads every tile.
        screen.clear(ClearState::depth(1.0));
        // Split to order by coverage, not mesh centre: pictures first, culling the lines behind.
        let camera = &self.statics.camera;
        self.stats.timed("backdrop", ctx, || {
            write_to_screen(&screen, || {
                apply_screen_material(ctx, &data.backdrop, camera, &[]);
            });
            1
        });
        self.stats.timed("pictures", ctx, || {
            let pictures = data.drawn_thumbnails().into_iter();
            render_counted(&screen, camera, pictures.chain(data.detail.drawn()))
        });
        self.stats.timed("lines", ctx, || {
            render_counted(&screen, camera, data.edges.into_iter().chain(&data.dashes))
        });
        // Nothing while nothing is selected, the clear included.
        self.stats.timed("lit", ctx, || {
            if !data.lit_anything() {
                return 0;
            }
            // Clearing depth lets the overlay depth-test its own worlds; a TBDR pays little.
            screen.clear(ClearState::depth(1.0));
            render_counted(&screen, camera, data.drawn_lit())
        });
        // Last of the scene: it brightens whichever pass drew its world.
        self.stats.timed("glow", ctx, || {
            let glow = data.drawn_glow();
            screen.render(camera, glow, &[]);
            u32::from(glow.is_some())
        });
        let overlay = self.overlay.as_mut().unwrap();
        self.stats.timed("panel", ctx, || {
            let mut calls = 0;
            write_to_screen(&screen, || calls = overlay.gui.paint(window) as u32);
            calls
        });

        // Last, after everything that can start motion.
        let data = self.data.as_ref().unwrap();
        // A lean keeps easing after its turn is too small to date a billboard.
        let moving = moved
            || panned
            || orbited
            // The switch needs the frame it moves in.
            || dollied
            || walking
            || leaning
            || self.veil > 0.0
            || data.framing
            || data.gesture.is_some()
            || !data.graph.is_settled();
        self.still_seconds = match moving {
            true => 0.0,
            false => self.still_seconds + frame_seconds(&frame_input),
        };
        self.wanted = self.wanted(data);
        moved
    }

    /// A frontier renumbers the worlds, and a refresh adds to the map the person is reading.
    fn retire_graph(&mut self) {
        self.selected = None;
        self.before = self.data.as_ref().map(AppEntities::before);
        self.data = None;
    }
}

pub(super) fn frame_seconds(frame_input: &FrameInput) -> f32 {
    frame_input.elapsed_time as f32 * 1e-3
}

/// Clamped: a long frame, the build's above all, is not paid for by every animation at once.
pub(super) fn clamped_frame_seconds(frame_input: &FrameInput) -> f32 {
    frame_seconds(frame_input).min(0.05)
}

pub(super) fn write_to_screen(screen: &RenderTarget, draw: impl FnOnce()) {
    screen
        .write::<std::convert::Infallible>(|| {
            draw();
            Ok(())
        })
        .unwrap();
}

/// Answers how many objects it drew, which [`FrameStats`] counts as draw calls.
fn render_counted<'a>(
    screen: &RenderTarget,
    camera: &Camera,
    objects: impl IntoIterator<Item = &'a dyn Object>,
) -> u32 {
    let mut calls = 0;
    screen.render(camera, objects.into_iter().inspect(|_| calls += 1), &[]);
    calls
}
