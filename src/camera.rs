//! Where the view is and how it is moved: the orbit, the pan, the dolly, the walk, the pinch,
//! and the ease onto what a selection framed. See [`AppStatics`].

use super::*;

/// Vertical field of view. The camera uses it, and so does the pan, which converts cursor pixels
/// into world units through it.
pub(super) const FOV_Y_DEGREES: f32 = 45.0;
/// Time constant of the camera's ease onto a selected route, in milliseconds. Long enough that the
/// move reads as travel across the graph rather than a cut, so the person keeps their bearings.
const FRAMING_WINDOW_MS: f32 = 600.0;
/// Slack left around a framed route, as a fraction of the distance that would touch it to the
/// window edges. The framed sphere already holds the whole of the pictures on its rim, so this is
/// breathing room and nothing more.
const FRAMING_MARGIN: f32 = 1.1;

/// How near the goal the camera has to be to count as arrived, as a fraction of the framed radius
/// and of the framing distance. The ease is asymptotic, so it needs a floor to stop at.
const FRAMING_ARRIVAL_TOLERANCE: f32 = 0.01;

/// Time constant of each of the lean's two eases, in milliseconds. Far longer than
/// [`FRAMING_WINDOW_MS`]: a framing move is asked for and should feel like travel, where a lean is
/// only a row under the pointer and should barely register as motion at all.
const LEAN_WINDOW_MS: f32 = 1200.0;
/// How near its goal the lean has to be to stop, as a fraction of how far the camera stands off
/// its centre -- the yardstick that keeps the floor the same size on screen however far out the
/// view is. Asymptotic as the framing ease is, and needing a floor for the same reason.
const LEAN_ARRIVAL_TOLERANCE: f32 = 0.001;
pub(super) struct AppStatics {
    pub(super) control: OrbitControl,
    pub(super) camera: Camera,
    /// Raised once a pan drag actually moves the camera rather than on the press, so a
    /// right-click that only opens a menu is not read as a pan.
    pub(super) panning: bool,
    /// The pointer the window was last given, so it is only set when it changes.
    pub(super) cursor: CursorIcon,
    pub(super) touches: Touches,
    pub(super) walk: Walk,
    /// What the orbit centre is easing onto, itself easing onto where the pointed world is. Two
    /// eases in series rather than one: a single ease is at its fastest the instant it starts,
    /// which is the jolt a row under the pointer should never give.
    pub(super) lean_aim: Vec3,
    /// Whether the window is the one being typed at. What the dashes march for is somebody
    /// watching them, so this is what lets a settled layout stop being drawn at all rather than
    /// go on at [`IDLE_REDRAW_HZ`] for as long as the app is open.
    ///
    /// Not read on the page, where the canvas is unfocused until it is clicked and the browser
    /// already stops serving frames to a tab nobody is looking at. A phone hands the drawing
    /// surface back instead: see [`App::suspended`].
    pub(super) focused: bool,
}

/// A key says only that it went down or came up, and walking has to carry on between the two, so
/// what is held is kept here and read once a frame as the distance it stands for.
/// Logical pixels a second: read against the window rather than the graph, because
/// [`AppStatics::world_per_pixel`] already scales a screen distance into world units at whatever
/// distance the camera stands.
const WALK_SPEED: f32 = 800.0;

#[derive(Default)]
pub(super) struct Walk {
    left: bool,
    right: bool,
    forward: bool,
    back: bool,
}

impl Walk {
    /// Emptied while egui has the keyboard, so typing a world's name into the search box does not
    /// also walk the view across the graph.
    pub(super) fn track(&mut self, events: &[Event], typing: bool) {
        if typing {
            *self = Self::default();
            return;
        }
        for event in events {
            let (key, down) = match event {
                Event::KeyPress { kind, .. } => (kind, true),
                Event::KeyRelease { kind, .. } => (kind, false),
                _ => continue,
            };
            match key {
                Key::W => self.forward = down,
                Key::A => self.left = down,
                Key::S => self.back = down,
                Key::D => self.right = down,
                _ => (),
            }
        }
    }

    /// How far the held keys walk over `dt` seconds: across the view, and into it.
    ///
    /// Both in logical pixels, so [`AppStatics::pan_by`] and [`AppStatics::dolly_by`] scale them
    /// alike and a step sideways covers as much ground as a step forward. Across is signed as a
    /// drag rather than a move, because that is what a pan is given: pushing the view right is
    /// pulling the graph left.
    pub(super) fn travel(&self, dt: f32) -> (f32, f32) {
        let (across, into) = (
            (self.left as i32 - self.right as i32) as f32,
            (self.forward as i32 - self.back as i32) as f32,
        );
        // Normalized, so two keys at once carry the view as far as one rather than over the
        // diagonal.
        let step = WALK_SPEED * dt / across.hypot(into).max(1.0);
        (across * step, into * step)
    }
}

/// The fingers on the screen, which the mouse events the app is otherwise driven by cannot say
/// enough about.
///
/// three-d turns a touch into a mouse: the first finger presses, drags and releases the left
/// button, and a second turns the pair into a wheel so a pinch zooms. What it never carries is
/// that the second finger is there at all, so a pinch ends with a left release that reads as a tap
/// and throws the selection away, and the travel of the pair across the screen -- the only pan
/// gesture a screen with no second button has -- is dropped. Both are read here instead, off the
/// events winit delivers before three-d ever sees them.
#[derive(Default)]
pub(super) struct Touches {
    /// Every finger down, by the id winit gave it, at its latest position in physical pixels.
    fingers: Vec<(u64, (f32, f32))>,
    /// Where their midpoint last was, while at least two of them are down.
    midpoint: Option<(f32, f32)>,
    /// How far that midpoint has travelled since a frame last took it, in physical pixels.
    pub(super) travel: (f32, f32),
    /// Whether a second finger has landed and not every finger has left since.
    ///
    /// Latched rather than read off the count, because the fingers do not lift together: the one
    /// still down when the other leaves would otherwise carry on as an orbit, and the last to
    /// leave would release as a tap.
    pub(super) pinching: bool,
}

impl Touches {
    /// Returns whether a pinch has just begun, which is the moment the gesture the single finger
    /// before it had nominated stops being any of the things it could have been.
    pub(super) fn track(&mut self, touch: &Touch) -> bool {
        let at = (touch.location.x as f32, touch.location.y as f32);
        match touch.phase {
            TouchPhase::Started => self.fingers.push((touch.id, at)),
            TouchPhase::Moved => {
                if let Some(finger) = self.fingers.iter_mut().find(|(id, _)| *id == touch.id) {
                    finger.1 = at;
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.fingers.retain(|(id, _)| *id != touch.id)
            }
        }
        let began = self.fingers.len() > 1 && !self.pinching;
        self.pinching = (self.pinching || began) && !self.fingers.is_empty();

        // Only a move measures travel: a finger arriving or leaving moves the midpoint by half
        // the gap between the fingers, which is not a drag and would fling the camera.
        let midpoint = self.midpoint();
        if let (Some(now), Some(was), TouchPhase::Moved) = (midpoint, self.midpoint, touch.phase) {
            self.travel.0 += now.0 - was.0;
            self.travel.1 += now.1 - was.1;
        }
        self.midpoint = midpoint;
        began
    }

    /// `None` until there are enough fingers for a midpoint to mean anything.
    fn midpoint(&self) -> Option<(f32, f32)> {
        (self.fingers.len() > 1).then(|| {
            let sum = self
                .fingers
                .iter()
                .fold((0.0, 0.0), |sum, (_, at)| (sum.0 + at.0, sum.1 + at.1));
            let count = self.fingers.len() as f32;
            (sum.0 / count, sum.1 / count)
        })
    }

    /// The travel since this was last asked, in the logical pixels a pan is measured in.
    pub(super) fn take_travel(&mut self, device_pixel_ratio: f32) -> (f32, f32) {
        let travel = std::mem::take(&mut self.travel);
        (travel.0 / device_pixel_ratio, travel.1 / device_pixel_ratio)
    }
}

/// The sphere a framing move has to bring into view.
pub(super) struct Bounds {
    pub(super) center: Vec3,
    pub(super) radius: f32,
}
impl AppStatics {
    /// [`OrbitControl`] only orbits and zooms, so panning lives here, on the buttons it leaves
    /// alone. Called before it, so a pan drag is not also read as an orbit.
    pub(super) fn pan(&mut self, events: &mut [Event], device_pixel_ratio: f32) -> bool {
        let mut panned = false;
        for event in events.iter_mut() {
            if let Event::MouseRelease {
                button: MouseButton::Right | MouseButton::Middle,
                ..
            } = event
            {
                self.panning = false;
            }
            let Event::MouseMotion {
                button: Some(MouseButton::Right | MouseButton::Middle),
                delta,
                handled,
                ..
            } = event
            else {
                continue;
            };
            if *handled {
                continue;
            }
            let delta = *delta;
            *handled = true;
            panned |= self.pan_by(delta, device_pixel_ratio);
            self.panning = true;
        }
        panned
    }

    /// Shared by the two ways of asking for a pan: a button the mouse has spare, and the two
    /// fingers a screen has instead.
    pub(super) fn pan_by(&mut self, delta: (f32, f32), device_pixel_ratio: f32) -> bool {
        if delta == (0.0, 0.0) {
            return false;
        }
        let scale = self.world_per_pixel(device_pixel_ratio);
        // Opposite the drag: moving the camera left pushes the graph right.
        let translation = self.camera.up_orthogonal() * (delta.1 * scale)
            - self.camera.right_direction() * (delta.0 * scale);
        self.camera.translate(translation);
        self.control.target += translation;
        true
    }

    /// `travel` is in logical pixels, as a pan's is.
    ///
    /// The orbit centre travels with the camera, which is what makes this a move through the graph
    /// rather than the wheel's zoom: the two never close on each other, so the view keeps its
    /// speed instead of creeping to a halt against a point it can never reach.
    pub(super) fn dolly_by(&mut self, travel: f32, device_pixel_ratio: f32) -> bool {
        if travel == 0.0 {
            return false;
        }
        let translation =
            self.camera.view_direction() * (travel * self.world_per_pixel(device_pixel_ratio));
        self.camera.translate(translation);
        self.control.target += translation;
        true
    }

    /// World units per logical pixel on the plane through the orbit centre. What keeps a drag
    /// holding whatever it started on, and a walk covering the same apparent ground however far
    /// out the camera is standing.
    fn world_per_pixel(&self, device_pixel_ratio: f32) -> f32 {
        let distance = self.control.target.distance(self.camera.position());
        let logical_height = self.camera.viewport().height as f32 / device_pixel_ratio;
        2.0 * distance * (FOV_Y_DEGREES.to_radians() * 0.5).tan() / logical_height
    }

    /// Only the two moves that take a drag are named: a pan holds the graph, an orbit turns it.
    /// The orbit is left unnamed in two dimensions, where the turn it would promise is locked.
    pub(super) fn track_cursor(&mut self, window: &Window, orbiting: bool) {
        let cursor = match () {
            _ if self.panning => CursorIcon::Move,
            _ if orbiting => CursorIcon::Grabbing,
            _ => CursorIcon::Default,
        };
        if cursor != self.cursor {
            self.cursor = cursor;
            window.set_cursor(cursor);
        }
    }

    /// Turns the camera square to the `z = 0` plane, keeping where it looks and how far off it
    /// stands. The one turn two-dimensional mode makes on its own, because it is also the one the
    /// person can no longer make: see [`lock_rotation`].
    pub(super) fn face_plane(&mut self) {
        let target = self.control.target;
        let distance = target.distance(self.camera.position());
        self.camera.set_view(
            target + vec3(0.0, 0.0, distance),
            target,
            vec3(0.0, 1.0, 0.0),
        );
    }

    /// How far off the camera has to stand to hold a sphere of `radius`.
    ///
    /// Read against whichever field of view angle is narrower, so a route that fits vertically
    /// cannot still hang off the sides of a tall window, and against the bounding sphere, so the
    /// fit does not depend on which way what is framed is turned relative to the camera.
    fn framing_distance(&self, radius: f32) -> f32 {
        let viewport = self.camera.viewport();
        let half_y = FOV_Y_DEGREES.to_radians() * 0.5;
        let half_x = (half_y.tan() * viewport.width as f32 / viewport.height as f32).atan();
        (radius * FRAMING_MARGIN / half_y.min(half_x).sin())
            .clamp(self.control.min_distance, self.control.max_distance)
    }

    /// Puts the camera where [`Self::ease_to_frame`] would carry it, in one step: what an opening
    /// view is framed with, there being nothing on screen yet to travel from.
    pub(super) fn snap_to_frame(&mut self, bounds: &Bounds) {
        let offset = self.camera.position() - self.control.target;
        let up = self.camera.up();
        let distance = self.framing_distance(bounds.radius);
        self.camera.set_view(
            bounds.center + offset / offset.magnitude() * distance,
            bounds.center,
            up,
        );
        self.control.target = bounds.center;
    }

    /// Returns whether it still has ground to cover.
    ///
    /// In three dimensions the eye stays put and turns onto the world; a flat view is held square
    /// to the plane and has no such turn to make, so there the eye travels with what it looks at.
    ///
    /// Nothing is given back when the pointing stops -- that would be the app moving the camera
    /// at the one moment the person has taken it over.
    pub(super) fn lean_toward(
        &mut self,
        at: Option<Vec3>,
        dimensions: Dimensions,
        dt: f32,
    ) -> bool {
        let Some(at) = at else {
            // Primed at the view, so the next lean starts from rest rather than from wherever the
            // last one was heading.
            self.lean_aim = self.control.target;
            return false;
        };
        let goal = match dimensions {
            Dimensions::Two => at,
            // The eye does not travel here, so the centre landing on the world also sets how far
            // off the eye stands: only as near as the orbit will stand it.
            Dimensions::Three => self.within_orbit(at),
        };
        // Against the goal, not against what the first ease has reached: that starts out level
        // with the centre and would read as arrived before either had moved.
        if goal.distance(self.control.target)
            < self.control.target.distance(self.camera.position()) * LEAN_ARRIVAL_TOLERANCE
        {
            return false;
        }

        // A fixed fraction of what is left per unit of time, as a framing move eases, and applied
        // twice over: see [`Self::lean_aim`].
        let step = 1.0 - (-dt / (LEAN_WINDOW_MS * 1e-3)).exp();
        self.lean_aim += (goal - self.lean_aim) * step;
        let travel = (self.lean_aim - self.control.target) * step;
        if dimensions == Dimensions::Two {
            self.camera.translate(travel);
        }
        self.control.target += travel;
        if dimensions == Dimensions::Three {
            let (eye, up) = (self.camera.position(), self.camera.up());
            self.camera.set_view(eye, self.control.target, up);
        }
        true
    }

    /// `at`, pulled along the line the eye sees it on to the nearest point the orbit's own zoom
    /// limits allow a centre. A centre nearer than the zoom can go turns the view inside out, and
    /// one on the eye leaves no direction to look at all.
    fn within_orbit(&self, at: Vec3) -> Vec3 {
        let eye = self.camera.position();
        let reach = at - eye;
        let span = reach.magnitude();
        let direction = match span > f32::EPSILON {
            true => reach / span,
            // A world sitting on the eye: the way it already looks is the only honest answer.
            false => self.camera.view_direction(),
        };
        eye + direction * span.clamp(self.control.min_distance, self.control.max_distance)
    }

    /// Returns whether the camera still has ground to cover.
    ///
    /// Only the orbit centre and the distance move: the direction the camera looks from is left
    /// exactly as the person left it, so the graph does not spin under them while it closes in.
    pub(super) fn ease_to_frame(&mut self, bounds: &Bounds, dt: f32) -> bool {
        let goal_distance = self.framing_distance(bounds.radius);
        let offset = self.camera.position() - self.control.target;
        let up = self.camera.up();
        let distance = offset.magnitude();
        let arrived = (bounds.center - self.control.target).magnitude()
            < bounds.radius * FRAMING_ARRIVAL_TOLERANCE
            && (goal_distance - distance).abs() < goal_distance * FRAMING_ARRIVAL_TOLERANCE;
        if arrived {
            return false;
        }

        // A fixed fraction of what is left, per unit of time rather than per frame, so the move
        // takes as long on a slow frame rate as on a fast one.
        let step = 1.0 - (-dt / (FRAMING_WINDOW_MS * 1e-3)).exp();
        let target = self.control.target + (bounds.center - self.control.target) * step;
        let distance = distance + (goal_distance - distance) * step;
        self.camera
            .set_view(target + offset / offset.magnitude() * distance, target, up);
        self.control.target = target;
        true
    }
}
/// Swallows the left-button motion [`OrbitControl`] would turn the camera with.
///
/// Called after the gesture is resolved, so a drag belonging to a node has already been taken and
/// only the camera's share is left. Presses and releases are left alone: they still resolve
/// clicks, and neither turns anything.
pub(super) fn lock_rotation(events: &mut [Event]) {
    for event in events {
        if let Event::MouseMotion {
            button: Some(MouseButton::Left),
            handled,
            ..
        } = event
        {
            *handled = true;
        }
    }
}

/// A thumbnail is flat, so without this a node would be seen edge-on from half the angles the
/// graph can be turned to. Taken from the camera's own basis, so every node shares one rotation.
pub(super) fn billboard(camera: &Camera) -> Mat4 {
    let forward = camera.view_direction();
    // Normalized because the camera's own up need only be the axis the view is kept upright
    // against, not a unit vector square to it. The orbit control refuses to look straight along
    // it, so the cross product cannot collapse.
    let right = camera.right_direction().normalize();
    let up = right.cross(forward);
    Mat4::from_cols(
        right.extend(0.0),
        up.extend(0.0),
        (-forward).extend(0.0),
        Vec4::unit_w(),
    )
}
