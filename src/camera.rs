//! Camera motion: turn, orbit, pan, dolly, keyboard movement, pinch, easing onto a selection. The
//! Orbital arrangement turns in place at the Nexus, the wheel narrowing the field of view.

use super::*;
use std::f32::consts::{PI, TAU};
use three_d_asset::ProjectionType;

/// Vertical field of view the camera opens with and returns to.
pub(super) const FOV_Y_DEGREES: f32 = 45.0;
/// How high over a world's centre [`above`] stands, in world radii: clear of the picture's top.
const ABOVE_WORLD_RADII: f32 = 2.0;
/// Field of view the wheel may set, in degrees: narrow enough that a world on the outermost shell,
/// seen from the Nexus, fills the window.
const FOV_Y_RANGE_DEGREES: std::ops::RangeInclusive<f32> = 0.25..=90.0;
/// How far ahead of the eye a camera turning in place keeps the orbit centre, in world units: the
/// distance the pan and the keyboard move measure their speed at.
const LOOK_DISTANCE: f32 = 20.0;
/// How close to straight up or down a camera turning in place may look, in radians. Looking along
/// `up` itself leaves the view without a sideways direction.
const PITCH_LIMIT: f32 = 1.55;
/// How near the goal heading a turn has to be to count as arrived, in radians.
const TURN_ARRIVAL_TOLERANCE: f32 = 0.002;
/// Time constant of the camera's ease onto a selected route, in milliseconds: long enough to read
/// as travel across the graph, so the user keeps their bearings.
const FRAMING_WINDOW_MS: f32 = 600.0;
/// Slack around a framed route, as a multiple of the distance that fits the route to the window.
const FRAMING_MARGIN: f32 = 1.1;

/// How near the goal the camera has to be to count as arrived, as a fraction of the framed radius
/// and of the framing distance. The ease is asymptotic and needs a floor to stop at.
const FRAMING_ARRIVAL_TOLERANCE: f32 = 0.01;

/// Time constant of each of row-follow's two eases, in milliseconds. Much slower than
/// [`FRAMING_WINDOW_MS`], so hovering a row barely moves the camera.
const ROW_FOLLOW_WINDOW_MS: f32 = 1200.0;
/// Stop distance for row-follow, as a fraction of the camera's distance from the target, so the
/// threshold is constant in screen space.
const ROW_FOLLOW_ARRIVAL_TOLERANCE: f32 = 0.001;
pub(super) struct AppPersistent {
    pub(super) control: OrbitControl,
    pub(super) camera: Camera,
    /// Raised once a pan drag actually moves the camera rather than on the press, so a
    /// right-click that only opens a menu is not read as a pan.
    pub(super) panning: bool,
    /// The pointer the window was last given, so the window's pointer is set only on change.
    pub(super) cursor: CursorIcon,
    pub(super) touches: Touches,
    pub(super) keyboard_move: KeyboardMove,
    /// Intermediate target between the orbit centre and the hovered row's world. Two eases in
    /// series start smoothly; a single ease is fastest on the first frame.
    pub(super) row_follow_aim: Vec3,
    /// Whether the window has keyboard focus, which lets a settled layout stop being drawn
    /// instead of redrawing at [`pacing::IDLE_REDRAW_HZ`].
    ///
    /// Unread in the web build, where the canvas is unfocused until clicked and the browser pauses
    /// hidden tabs. A phone hands the surface back: see [`App::suspended`].
    pub(super) focused: bool,
}

/// Logical pixels a second, which [`AppPersistent::world_per_pixel`] scales into world units at the
/// camera's distance.
const KEYBOARD_SPEED: f32 = 2000.0;

/// The WASD keys held, read once a frame as a distance; key events report only presses and
/// releases.
#[derive(Default)]
pub(super) struct KeyboardMove {
    left: bool,
    right: bool,
    forward: bool,
    back: bool,
}

impl KeyboardMove {
    /// Emptied while egui has the keyboard, so typing a world's name into the search box does not
    /// also move the view across the graph.
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

    /// How far the held keys move the view over `dt` seconds, across and into the view, in logical
    /// pixels; across is signed as a drag, the input [`AppPersistent::pan_by`] takes.
    pub(super) fn travel(&self, dt: f32) -> (f32, f32) {
        let (across, into) = (
            (self.left as i32 - self.right as i32) as f32,
            (self.forward as i32 - self.back as i32) as f32,
        );
        // Normalized, so a diagonal moves no faster.
        let step = KEYBOARD_SPEED * dt / across.hypot(into).max(1.0);
        (across * step, into * step)
    }
}

/// The fingers on the screen, read off winit events: three-d turns a touch into a mouse and drops
/// the second finger, so a pinch would end in a tap and the two-finger pan would be lost.
#[derive(Default)]
pub(super) struct Touches {
    /// Every finger down, by winit id, at the latest position in physical pixels.
    fingers: Vec<(u64, (f32, f32))>,
    /// The fingers' last midpoint, while at least two are down.
    midpoint: Option<(f32, f32)>,
    /// Midpoint travel since the last [`Self::take_travel`], in physical pixels.
    pub(super) travel: (f32, f32),
    /// Whether a second finger has landed and not every finger has left since. Latched, as the
    /// fingers lift one at a time: the last one down would orbit, then release as a tap.
    pub(super) pinching: bool,
}

impl Touches {
    /// Returns whether a pinch has just begun, which cancels the single finger's pending gesture.
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

        // A finger arriving or leaving shifts the midpoint by half the gap: not a drag.
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

    /// The travel since this was last read, in the logical pixels a pan is measured in.
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
impl AppPersistent {
    /// Pans on the right and middle buttons, which [`OrbitControl`] leaves free. Called before
    /// [`OrbitControl`], so a pan drag is not also read as an orbit.
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

    /// Shared by mouse pans and two-finger pans on a touch screen.
    pub(super) fn pan_by(&mut self, delta: (f32, f32), device_pixel_ratio: f32) -> bool {
        if delta == (0.0, 0.0) {
            return false;
        }
        let scale = self.world_per_pixel(device_pixel_ratio);
        let translation = self.camera.up_orthogonal() * (delta.1 * scale)
            - self.camera.right_direction() * (delta.0 * scale);
        self.shift_view(translation);
        true
    }

    /// Moves the eye and the orbit centre `travel` logical pixels into the view; the gap between
    /// them holds, so the speed stays constant.
    pub(super) fn dolly_by(&mut self, travel: f32, device_pixel_ratio: f32) -> bool {
        if travel == 0.0 {
            return false;
        }
        let translation =
            self.camera.view_direction() * (travel * self.world_per_pixel(device_pixel_ratio));
        self.shift_view(translation);
        true
    }

    /// Moves the eye and the orbit centre together.
    fn shift_view(&mut self, translation: Vec3) {
        self.camera.translate(translation);
        self.control.target += translation;
    }

    /// Puts the orbit centre at `target` and the eye `distance` away, keeping the view direction.
    fn stand_off(&mut self, target: Vec3, distance: f32) {
        let direction = (self.camera.position() - self.control.target).normalize();
        let up = self.camera.up();
        self.camera
            .set_view(target + direction * distance, target, up);
        self.control.target = target;
    }

    /// World units per logical pixel on the plane through the orbit centre, so a drag keeps hold
    /// of the point under the cursor at any distance.
    fn world_per_pixel(&self, device_pixel_ratio: f32) -> f32 {
        let distance = self.control.target.distance(self.camera.position());
        let logical_height = self.camera.viewport().height as f32 / device_pixel_ratio;
        2.0 * distance * (self.field_of_view_y() * 0.5).tan() / logical_height
    }

    /// Sets the pointer for the two drag moves, a pan and an orbit; `orbiting` is false in 2D,
    /// where the turn is locked.
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

    /// Turns the camera square to the `z = 0` plane, keeping the target and distance, for 2D mode,
    /// which locks the turn: see [`take_left_drags`].
    pub(super) fn face_plane(&mut self) {
        self.set_field_of_view_y(FOV_Y_DEGREES.to_radians());
        let target = self.control.target;
        let distance = target.distance(self.camera.position());
        self.camera.set_view(
            target + vec3(0.0, 0.0, distance),
            target,
            vec3(0.0, 1.0, 0.0),
        );
    }

    /// How far off the camera has to stand to hold a sphere of `radius` within the narrower axis
    /// of the field of view, so a tall window still fits the sides.
    fn framing_distance(&self, radius: f32) -> f32 {
        let viewport = self.camera.viewport();
        let half_y = FOV_Y_DEGREES.to_radians() * 0.5;
        let half_x = (half_y.tan() * viewport.width as f32 / viewport.height as f32).atan();
        (radius * FRAMING_MARGIN / half_y.min(half_x).sin())
            .clamp(self.control.min_distance, self.control.max_distance)
    }

    /// Snaps the camera, for the opening view, to where [`Self::ease_to_frame`] would ease to.
    pub(super) fn snap_to_frame(&mut self, bounds: &Bounds) {
        self.stand_off(bounds.center, self.framing_distance(bounds.radius));
    }

    /// Eases the orbit centre toward the hovered row's world. Returns true while still moving.
    ///
    /// In 3D the eye stays put and turns; in 2D the eye pans with the target. The camera does not
    /// return when the hover ends, since the user may have taken control by then.
    pub(super) fn follow_hovered_row(
        &mut self,
        at: Option<Vec3>,
        dimensions: Dimensions,
        dt: f32,
    ) -> bool {
        let Some(at) = at else {
            // Reset so the next row-follow starts from rest.
            self.row_follow_aim = self.control.target;
            return false;
        };
        let goal = match dimensions {
            Dimensions::Two => at,
            // The eye stays put, so the centre also sets the orbit distance, within the limits.
            Dimensions::Three => self.within_orbit(at),
        };
        // Against the goal: the first ease starts level with the centre and would read as arrived.
        if goal.distance(self.control.target)
            < self.control.target.distance(self.camera.position()) * ROW_FOLLOW_ARRIVAL_TOLERANCE
        {
            return false;
        }

        // Applied twice over: see [`Self::row_follow_aim`].
        let step = fraction_covered(dt, ROW_FOLLOW_WINDOW_MS);
        self.row_follow_aim += (goal - self.row_follow_aim) * step;
        let travel = (self.row_follow_aim - self.control.target) * step;
        match dimensions {
            Dimensions::Two => self.shift_view(travel),
            Dimensions::Three => {
                self.control.target += travel;
                let (eye, up) = (self.camera.position(), self.camera.up());
                self.camera.set_view(eye, self.control.target, up);
            }
        }
        true
    }

    /// `at`, moved along the line of sight into the orbit's zoom limits; a nearer centre turns the
    /// view inside out.
    fn within_orbit(&self, at: Vec3) -> Vec3 {
        let eye = self.camera.position();
        let reach = at - eye;
        let span = reach.magnitude();
        let direction = match span > f32::EPSILON {
            true => reach / span,
            // A world on the eye keeps the current direction.
            false => self.camera.view_direction(),
        };
        eye + direction * span.clamp(self.control.min_distance, self.control.max_distance)
    }

    /// Eases the orbit centre and distance toward `bounds`, keeping the view direction the user
    /// set. Returns whether the camera still has ground to cover.
    pub(super) fn ease_to_frame(&mut self, bounds: &Bounds, dt: f32) -> bool {
        let goal_distance = self.framing_distance(bounds.radius);
        let distance = self.camera.position().distance(self.control.target);
        let arrived = (bounds.center - self.control.target).magnitude()
            < bounds.radius * FRAMING_ARRIVAL_TOLERANCE
            && (goal_distance - distance).abs() < goal_distance * FRAMING_ARRIVAL_TOLERANCE;
        if arrived {
            return false;
        }

        let step = fraction_covered(dt, FRAMING_WINDOW_MS);
        let target = self.control.target + (bounds.center - self.control.target) * step;
        self.stand_off(target, distance + (goal_distance - distance) * step);
        true
    }
}

/// The camera that stands at a point and turns there.
impl AppPersistent {
    /// Turns with the left drags and narrows or widens the view with the wheel, never moving the
    /// eye. Returns whether the view changed.
    pub(super) fn turn_in_place(&mut self, events: &mut [Event], device_pixel_ratio: f32) -> bool {
        let mut turned = false;
        for event in events.iter_mut() {
            match event {
                Event::MouseMotion {
                    button: Some(MouseButton::Left),
                    delta,
                    handled: handled @ false,
                    ..
                } => {
                    // Read against the field of view, so what was under the cursor stays there.
                    let logical_height = self.camera.viewport().height as f32 / device_pixel_ratio;
                    let per_pixel = self.field_of_view_y() / logical_height;
                    let (yaw, pitch) = heading(self.camera.view_direction());
                    self.look_along(direction_of(
                        yaw + delta.0 * per_pixel,
                        pitch + delta.1 * per_pixel,
                    ));
                    *handled = true;
                    turned = true;
                }
                Event::MouseWheel {
                    delta,
                    handled: handled @ false,
                    ..
                } => {
                    let narrowed = self.field_of_view_y() * (-delta.1 * 0.01).exp();
                    self.set_field_of_view_y(narrowed);
                    *handled = true;
                    turned = true;
                }
                _ => (),
            }
        }
        turned
    }

    /// Brings `bounds` into view: turning toward the bounds when standing at the Nexus, else
    /// closing on them. Returns whether there is moving left to do.
    pub(super) fn bring_into_view(&mut self, bounds: &Bounds, standing: bool, dt: f32) -> bool {
        match standing {
            true => self.turn_to_frame(bounds, dt),
            false => self.ease_to_frame(bounds, dt),
        }
    }

    /// Carries the camera back to the opening view of `home`: the eye over `home` when standing,
    /// else framed from [`opening_facing`]. Returns whether there is moving left to do.
    pub(super) fn reset_view(
        &mut self,
        home: &Bounds,
        standing: bool,
        dimensions: Dimensions,
        dt: f32,
    ) -> bool {
        if standing {
            return self.fly_to(above(home), dt);
        }
        let goal = opening_facing(dimensions);
        let facing = (self.camera.position() - self.control.target).normalize();
        let turning = facing.angle(goal).0 > TURN_ARRIVAL_TOLERANCE;
        if turning {
            let step = fraction_covered(dt, FRAMING_WINDOW_MS);
            let eased = (facing + (goal - facing) * step).normalize();
            let (target, distance) = (
                self.control.target,
                self.control.target.distance(self.camera.position()),
            );
            self.camera
                .set_view(target + eased * distance, target, vec3(0.0, 1.0, 0.0));
        }
        self.ease_to_frame(home, dt) || turning
    }

    /// Turns toward `bounds` from where the eye stands, narrowing the view to hold them.
    /// Returns whether there is turning left to do.
    pub(super) fn turn_to_frame(&mut self, bounds: &Bounds, dt: f32) -> bool {
        let reach = bounds.center - self.camera.position();
        let distance = reach.magnitude();
        let goal_fov = match distance > bounds.radius * FRAMING_MARGIN {
            true => self.fov_covering_sphere(bounds.radius * FRAMING_MARGIN / distance),
            // From inside, nothing narrower holds them.
            false => FOV_Y_DEGREES.to_radians(),
        };
        let fov = self.field_of_view_y();
        let fov_arrived = (goal_fov - fov).abs() < goal_fov * FRAMING_ARRIVAL_TOLERANCE;
        let step = fraction_covered(dt, FRAMING_WINDOW_MS);
        let turning = distance > f32::EPSILON && self.turn_toward_by(reach, step);
        if fov_arrived && !turning {
            return false;
        }
        self.set_field_of_view_y(fov + (goal_fov - fov) * step);
        true
    }

    /// Eases the view toward `at`, the eye staying put. Returns whether the view is still turning.
    pub(super) fn turn_toward(&mut self, at: Vec3, dt: f32) -> bool {
        let reach = at - self.camera.position();
        reach.magnitude() > f32::EPSILON
            && self.turn_toward_by(reach, fraction_covered(dt, ROW_FOLLOW_WINDOW_MS))
    }

    /// Carries the eye to `at` and the field of view back to the default, keeping the view
    /// direction. Returns whether there is ground left to cover.
    pub(super) fn fly_to(&mut self, at: Vec3, dt: f32) -> bool {
        let gap = at - self.camera.position();
        let (fov, goal_fov) = (self.field_of_view_y(), FOV_Y_DEGREES.to_radians());
        if gap.magnitude() < LOOK_DISTANCE * FRAMING_ARRIVAL_TOLERANCE
            && (goal_fov - fov).abs() < goal_fov * FRAMING_ARRIVAL_TOLERANCE
        {
            return false;
        }
        let step = fraction_covered(dt, FRAMING_WINDOW_MS);
        self.shift_view(gap * step);
        self.set_field_of_view_y(fov + (goal_fov - fov) * step);
        true
    }

    /// Puts the eye at `at` in one step, with the default field of view and looking along
    /// [`heading_zero`]: what an opening view starts from.
    pub(super) fn stand_at(&mut self, at: Vec3) {
        self.shift_view(at - self.camera.position());
        self.set_field_of_view_y(FOV_Y_DEGREES.to_radians());
        self.look_along(heading_zero());
    }

    /// Covers `step` of the turn toward `reach`. Returns whether the turn had not yet arrived.
    fn turn_toward_by(&mut self, reach: Vec3, step: f32) -> bool {
        let (yaw, pitch) = heading(self.camera.view_direction());
        let (goal_yaw, goal_pitch) = heading(reach.normalize());
        let goal_pitch = goal_pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
        // The short way round.
        let yaw_gap = (goal_yaw - yaw + PI).rem_euclid(TAU) - PI;
        let pitch_gap = goal_pitch - pitch;
        if yaw_gap.abs() < TURN_ARRIVAL_TOLERANCE && pitch_gap.abs() < TURN_ARRIVAL_TOLERANCE {
            return false;
        }
        self.look_along(direction_of(yaw + yaw_gap * step, pitch + pitch_gap * step));
        true
    }

    /// Points the camera along `direction` without moving the eye, keeping the orbit centre
    /// [`LOOK_DISTANCE`] ahead so the pan and the keyboard move keep their speed.
    fn look_along(&mut self, direction: Vec3) {
        let eye = self.camera.position();
        self.control.target = eye + direction * LOOK_DISTANCE;
        self.camera
            .set_view(eye, self.control.target, vec3(0.0, 1.0, 0.0));
    }

    /// The vertical field of view that holds a sphere whose radius is `sine` of the distance to
    /// the sphere, read against whichever axis of the window is narrower.
    fn fov_covering_sphere(&self, sine: f32) -> f32 {
        let half = sine.min(1.0).asin();
        let viewport = self.camera.viewport();
        let aspect = viewport.width as f32 / viewport.height as f32;
        let half_y = match aspect < 1.0 {
            true => (half.tan() / aspect).atan(),
            false => half,
        };
        2.0 * half_y
    }

    fn field_of_view_y(&self) -> f32 {
        match self.camera.projection_type() {
            ProjectionType::Perspective { field_of_view_y } => field_of_view_y.0,
            _ => FOV_Y_DEGREES.to_radians(),
        }
    }

    /// Held to [`FOV_Y_RANGE_DEGREES`].
    pub(super) fn reset_field_of_view(&mut self) {
        self.set_field_of_view_y(FOV_Y_DEGREES.to_radians());
    }

    fn set_field_of_view_y(&mut self, radians: f32) {
        let range = FOV_Y_RANGE_DEGREES;
        let clamped = radians.clamp(range.start().to_radians(), range.end().to_radians());
        let (near, far) = (self.camera.z_near(), self.camera.z_far());
        self.camera
            .set_perspective_projection(Rad(clamped), near, far);
    }
}

/// The level direction the horizon marker stands in.
pub(super) fn heading_zero() -> Vec3 {
    vec3(0.0, 0.0, 1.0)
}

/// The side the opening view frames the graph from: square to the plane in 2D, and in 3D a
/// direction picked off a settled layout.
pub(super) fn opening_facing(dimensions: Dimensions) -> Vec3 {
    match dimensions {
        Dimensions::Two => vec3(0.0, 0.0, 1.0),
        Dimensions::Three => vec3(-268.2, 126.4, -211.6).normalize(),
    }
}

/// Where the eye stands to look out from a world: over the world's picture, below the view.
pub(super) fn above(bounds: &Bounds) -> Vec3 {
    bounds.center + vec3(0.0, bounds.radius * ABOVE_WORLD_RADII, 0.0)
}

/// The turn about `up` and the tilt toward `up` that point the view along `direction`.
fn heading(direction: Vec3) -> (f32, f32) {
    (
        direction.x.atan2(direction.z),
        direction.y.clamp(-1.0, 1.0).asin(),
    )
}

/// Inverse of [`heading`], the tilt held within [`PITCH_LIMIT`].
fn direction_of(yaw: f32, pitch: f32) -> Vec3 {
    let pitch = pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
    vec3(
        pitch.cos() * yaw.sin(),
        pitch.sin(),
        pitch.cos() * yaw.cos(),
    )
}

/// How much of what is left of an exponential ease `seconds` covers. Per unit of time, not per
/// frame, so the move lasts as long at any frame rate.
fn fraction_covered(seconds: f32, window_ms: f32) -> f32 {
    1.0 - (-seconds / (window_ms * 1e-3)).exp()
}
/// Takes the left-drag motion [`OrbitControl`] would orbit with, returning the travel in logical
/// pixels. Called after the gesture is resolved, so node drags are already taken.
pub(super) fn take_left_drags(events: &mut [Event]) -> (f32, f32) {
    let mut travel = (0.0, 0.0);
    for event in events {
        if let Event::MouseMotion {
            button: Some(MouseButton::Left),
            delta,
            handled: handled @ false,
            ..
        } = event
        {
            travel.0 += delta.0;
            travel.1 += delta.1;
            *handled = true;
        }
    }
    travel
}

/// The one rotation, from the camera's basis, that turns every flat thumbnail to face the camera.
pub(super) fn billboard(camera: &Camera) -> Mat4 {
    let forward = camera.view_direction();
    // The orbit control never looks along `up`, so this cannot collapse.
    let right = camera.right_direction().normalize();
    let up = right.cross(forward);
    Mat4::from_cols(
        right.extend(0.0),
        up.extend(0.0),
        (-forward).extend(0.0),
        Vec4::unit_w(),
    )
}
