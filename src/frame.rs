//! One frame of the graph, from what arrived since the last one through to the passes that draw
//! it. See [`App::draw`].

use super::*;

impl App {
    /// One frame: takes in whatever arrived, moves the camera, steps the layout, and draws it.
    /// Answers whether it got as far as rebuilding the geometry, which [`FrameStats`] counts.
    pub(super) fn draw(&mut self) -> bool {
        // Until the end of the frame says otherwise. The loading frame below returns through
        // here, and it animates.
        self.wanted = Wanted::Now;
        // Whether or not there is a graph yet: a resumed session reads its account while the dump
        // is still on its way, and the answer has to be in hand before the graph is built.
        self.yno.poll();
        if self.yno.restated() {
            // A frontier is a different numbering of the worlds, so what was lit means nothing in
            // the graph about to be built.
            self.selected = None;
            // Where this graph had got to, for the next to carry on from: the person is looking at
            // a map, and a refresh should add to it rather than replace it.
            self.before = self.data.as_ref().map(AppEntities::before);
            self.data = None;
            self.build();
        }
        // The worlds the code shows never reached this run: `world::hide` drops them as the dump
        // is read, so showing them is another dump rather than another drawing of the one in hand.
        if !self.revealed && self.code.taken() {
            self.revealed = true;
            self.selected = None;
            self.before = self.data.as_ref().map(AppEntities::before);
            self.data = None;
            self.dump = Dump::Loading(fetch::spawn(world::load(true)));
        }
        // Only while there is nothing to draw: every frame after this one has a graph in it and
        // nothing left to wait for.
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

        // Clamped as the layout's own step is: the frame the graph was built on is a long one, and
        // the reveal should not be spent paying for it.
        if self.veil > 0.0 {
            let step = (frame_input.elapsed_time as f32 * 1e-3).min(0.05);
            self.veil = (self.veil - step / LOADING_FADE_SECONDS).max(0.0);
        }
        // Whatever the loading frame was saying when it gave way, still saying it as it goes.
        let fading = (self.veil > 0.0).then(|| (self.said.clone(), self.veil));
        let data = self.data.as_mut().unwrap();
        self.statics.camera.set_viewport(frame_input.viewport);
        let dollied = profile::pan_aside(&mut self.statics, data);
        let account = &mut self.yno;
        // The whole game, which is the yardstick the settings tab measures one person's share
        // against: the graph beside it may be only the frontier.
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
            // Repulsion acts along the offset between two nodes, so a layout flattened onto the
            // plane has no depth to reinflate. Restarting is cheaper than reseeding that axis.
            scatter(data);
            if data.graph.parameters().dimensions == Dimensions::Two {
                // Squared onto the plane before the turn is locked, or a camera left oblique by
                // the three-dimensional view would stay that way with no way to straighten it.
                self.statics.face_plane();
            }
        }
        // Ahead of the pan and the orbit control, which both take whatever left-button motion this
        // leaves unhandled.
        data.track_gesture(
            &self.statics.camera,
            &mut frame_input.events,
            self.statics.touches.pinching,
        );
        if data.graph.parameters().dimensions == Dimensions::Two {
            // A flat layout has one face worth looking at. Swallowing the drag the orbit control
            // reads leaves it the zoom and leaves the pan alone.
            lock_rotation(&mut frame_input.events);
        }
        // Two fingers do both jobs off the same pair of positions: three-d reads the gap between
        // them as a wheel, and their midpoint's travel is the pan. Independent, so a drag that
        // also spreads does both.
        let dragged = self
            .statics
            .touches
            .take_travel(frame_input.device_pixel_ratio);
        // Read after the overlay, which settles whether the keys are being typed into the search
        // box rather than walked with.
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
        let (across, into) = self
            .statics
            .walk
            .travel((frame_input.elapsed_time as f32 * 1e-3).min(0.05));
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
        // The camera belongs to whoever last touched it: an orbit, a pan or a zoom abandons the
        // framing and the lean rather than fighting either. `panned` carries the walk keys and the
        // dolly too, so this is every way it is moved by hand.
        if panned || orbited {
            data.framing = false;
            data.leaning_at = None;
        }
        let orbiting = matches!(data.gesture, Some(Gesture::Orbiting))
            && data.graph.parameters().dimensions == Dimensions::Three;
        self.statics
            .track_cursor(self.ctx.window.as_ref().unwrap(), orbiting);
        let dt = (frame_input.elapsed_time as f32 * 1e-3).min(0.05);
        if data.framing {
            // Recomputed every frame rather than fixed when the selection was made: the layout is
            // usually still moving, and a goal taken once would be stale before the camera got
            // there. What is arriving comes first, the selection having the camera back once it is
            // over.
            data.framing = match data.arrival_bounds() {
                Some(bounds) => {
                    // Kept on whether or not the camera has caught up, unlike a selection: what it
                    // follows is still being pushed about by the worlds that landed in it.
                    self.statics.ease_to_frame(&bounds, dt);
                    true
                }
                None => match data.framing_bounds() {
                    Some(bounds) => {
                        let easing = self.statics.ease_to_frame(&bounds, dt);
                        // Kept whether or not the camera has caught up, as an arrival is: the tree
                        // is still spreading, so arriving means arriving at how far it had got.
                        easing || (data.selected.is_none() && !data.graph.is_settled())
                    }
                    None => false,
                },
            };
        }

        // Latched rather than read off `pointed` each frame: a lean carries on after the pointer
        // has left the row, and only taking the camera stops it.
        if let Some(world) = data.pointed {
            data.leaning_at = Some(world);
        }
        // A framing move owns the camera while it runs, so the lean waits rather than dragging on
        // the goal that move is easing onto and leaving it never arrived.
        let leaning = self.statics.lean_toward(
            (self.overlay.as_ref().unwrap().leaning && !data.framing)
                .then(|| data.leaning_at.and_then(|world| data.world_at(world)))
                .flatten(),
            data.graph.parameters().dimensions,
            dt,
        );

        data.pull_grabbed_node(&self.statics.camera);
        data.receive_atlas(
            (frame_input.elapsed_time as f32 * 1e-3).min(0.05),
            ctx,
            self.overlay.as_ref().unwrap().gui.context(),
        );
        // Unclamped: the layout steps at a fixed rate and caps how much of a long frame it catches
        // up on itself, so a stalled tab is already its problem.
        let stepping = web_time::Instant::now();
        let stepped = data.graph.update(frame_input.elapsed_time as f32 * 1e-3);
        self.stats.stepped(stepping.elapsed());
        // The quads face the camera, so turning it dates their transformations even over a layout
        // that has not moved at all.
        let turned = data.billboard != billboard(&self.statics.camera);
        // Clamped as the reveal is, and for the same reason: an arrival should not be half over
        // before it is first drawn.
        let arriving = data
            .arrivals
            .tick((frame_input.elapsed_time as f32 * 1e-3).min(0.05));
        // Everything below reads the node quads, the camera, or the instance colors, so this is
        // what says any of it has to be worked out again.
        let recolored = std::mem::take(&mut data.recolored);
        // A recolor is not in it: `repaint` writes the colors and uploads them itself, leaving
        // the geometry as it was.
        let laid_out = stepped || arriving || profile::eager();
        let moved = laid_out || turned || recolored;
        if laid_out || turned {
            data.rebuild_instances(&self.statics.camera, laid_out);
        }
        // Whether or not anything else moved: see [`AppEntities::march_dashes`].
        data.march_dashes((frame_input.elapsed_time as f32 * 1e-3).min(0.05));
        // Both of these are also where an arrived picture is taken out of its fetch, and nothing
        // else reads it -- so a frame with one still on its way runs them even if nothing moved,
        // or the fetch would stay pending for good.
        let reading = data.detail.pending();
        // After the instances, whose colors the full pictures borrow.
        if moved || reading {
            let magnified = data.magnified(&self.statics.camera, frame_input.viewport);
            data.detail.track(ctx, &magnified);
        }
        if moved || reading {
            // What it copies is the node quads rebuilt just above, and the camera it is lifted
            // against is the one that turned them.
            data.place_unvisited(ctx, &self.statics.camera);
        }
        // Also on a frame that only moved the pointer from one list row to another, which moves
        // the glow without moving anything it is copied from.
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
        // Depth only: the backdrop written straight after covers every pixel of the target,
        // with the depth test off and no blending, so the color is written twice otherwise.
        //
        // Only measured on an IMR, which keeps the target in memory, so this is a write saved. A
        // TBDR resolves per tile and the clear is its LoadOp: drop it and every tile loads the
        // last frame back instead. Phones and Apple silicon are TBDR -- measure there first.
        screen.clear(ClearState::depth(1.0));
        // Split only to fix the order: one call would sort these by each mesh's centre, which says
        // nothing about which covers which. Pictures before lines, so the depth they write drops
        // the lines behind them unshaded.
        let camera = &self.statics.camera;
        self.stats.timed("backdrop", ctx, || {
            screen
                .write::<std::convert::Infallible>(|| {
                    apply_screen_material(ctx, &data.backdrop, camera, &[]);
                    Ok(())
                })
                .unwrap();
            1
        });
        self.stats.timed("pictures", ctx, || {
            let mut calls = 0;
            screen.render(
                camera,
                data.drawn_thumbnails()
                    .into_iter()
                    .chain(data.detail.drawn())
                    .inspect(|_| calls += 1),
                &[],
            );
            calls
        });
        self.stats.timed("lines", ctx, || {
            let mut calls = 0;
            screen.render(
                camera,
                data.edges
                    .into_iter()
                    .chain(&data.dashes)
                    .inspect(|_| calls += 1),
                &[],
            );
            calls
        });
        // What the selection lights, again, over everything the passes above drew. Nothing at all
        // while nothing is selected, the clear included.
        self.stats.timed("lit", ctx, || {
            let mut calls = 0;
            if data.lit_anything() {
                // Depth only, and only the scene's own: clearing what the layout parked between
                // the overlay and the eye is what lets the overlay keep a real depth test among
                // its own worlds. A clear inside a pass is a tile op, so a TBDR pays little.
                screen.clear(ClearState::depth(1.0));
                screen.render(camera, data.drawn_lit().inspect(|_| calls += 1), &[]);
            }
            calls
        });
        // Last of the scene, because it brightens whichever pass above drew the world it is over.
        self.stats.timed("glow", ctx, || {
            let glow = data.drawn_glow();
            screen.render(camera, glow, &[]);
            u32::from(glow.is_some())
        });
        // Over the scene, into the same target, which is what makes it an overlay.
        let overlay = self.overlay.as_mut().unwrap();
        self.stats.timed("panel", ctx, || {
            let mut calls = 0;
            screen
                .write::<std::convert::Infallible>(|| {
                    calls = overlay.gui.paint(window) as u32;
                    Ok(())
                })
                .unwrap();
            calls
        });

        // Last, so that everything able to start something moving has had its turn.
        let data = self.data.as_ref().unwrap();
        // `moved` covers the layout stepping, the camera turning and a selection repainting. The
        // rest leaves the frame looking the same -- a pan or a dolly turns nothing -- but a lean
        // goes on easing after its turn has grown too small to date a billboard.
        let moving = moved
            || panned
            || orbited
            // Nothing on screen, but the switch that moves it needs the frame it moves in.
            || dollied
            || walking
            || leaning
            || self.veil > 0.0
            || data.framing
            || data.gesture.is_some()
            || !data.graph.is_settled();
        self.still = match moving {
            true => 0.0,
            false => self.still + frame_input.elapsed_time as f32 * 1e-3,
        };
        self.wanted = self.wanted(data);
        moved
    }
}
