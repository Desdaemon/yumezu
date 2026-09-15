//! The row every list in the panel is built out of, and the hold that stands in for a hover on a
//! touch screen. See [`Tile`].

use super::*;

/// A row of a list, built the way a list tile is: something leading it, what it says, something
/// trailing it, and all of it one press the width of the list.
///
/// [`egui::Ui::selectable_label`] is as wide as its own text and as tall as a line, so a thumb aims
/// at a word and a reader cannot see how far a row reaches. Anything set beside it in an
/// [`egui::Ui::horizontal`] is a widget of its own answering no press, so a row that reads as one
/// thing behaves as three; here they are atoms of the row itself.
pub(super) struct Tile<'a> {
    selected: bool,
    leading: egui::Atoms<'a>,
    title: egui::WidgetText,
    trailing: egui::Atoms<'a>,
    hover: Option<egui::WidgetText>,
}

impl<'a> Tile<'a> {
    pub(super) fn new(selected: bool, title: impl Into<egui::WidgetText>) -> Self {
        Self {
            selected,
            leading: egui::Atoms::default(),
            title: title.into(),
            trailing: egui::Atoms::default(),
            hover: None,
        }
    }

    pub(super) fn leading(mut self, leading: impl egui::IntoAtoms<'a>) -> Self {
        self.leading = egui::Atoms::new(leading);
        self
    }

    pub(super) fn trailing(mut self, trailing: impl egui::IntoAtoms<'a>) -> Self {
        self.trailing = egui::Atoms::new(trailing);
        self
    }

    /// What a pointer reads by hovering the row, and a finger by holding it.
    ///
    /// Not [`egui::Response::on_hover_text`] on the row after the fact, which draws the text where
    /// the hand is. Here it is put on the far side of the row and stood off far enough to be read
    /// past a fingertip. See [`HOVER_FINGER_GAP`].
    pub(super) fn hover(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.hover = Some(text.into());
        self
    }
}

impl egui::Widget for Tile<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        use egui::AtomExt as _;

        let Self {
            selected,
            mut leading,
            title,
            trailing,
            hover,
        } = self;
        // The title is what gives way when the row is too narrow for everything in it: what leads
        // and trails a row is a glyph or two, and a truncated one says nothing at all.
        leading.push_right(egui::Atom::from(title).atom_shrink(true));
        leading.push_right(egui::Atom::grow());
        leading.extend_right(trailing);
        let touch = ui.input(|input| input.has_touch_screen());
        let height = ui.spacing().interact_size.y
            + match touch {
                true => TILE_PADDING_TOUCH,
                false => 0.0,
            };
        let row = ui.add(
            egui::Button::selectable(selected, leading)
                .min_size(egui::vec2(ui.available_width(), height))
                .truncate(),
        );
        let Some(hover) = hover else { return row };
        if !touch {
            return row.on_hover_text(hover);
        }
        // A press that also pressed the row ends its own peek, what the text named being about to
        // be gone. Only a hold long enough to stop being a click leaves the text standing.
        if row.clicked() {
            release_hover(ui.ctx());
            return row;
        }
        let pressing = row.is_pointer_button_down_on();
        if pressing {
            // The wait ends on a frame no event would call for: a finger resting still sends none.
            ui.ctx().request_repaint();
        }
        // Called up by the finger that is on the row now, or by one that was and has gone.
        let called_up = pressing && held_long_enough(&row);
        if called_up {
            let at = ui
                .input(|input| input.pointer.press_origin())
                .map_or(row.rect.center().x, |origin| origin.x);
            hold_hover(
                ui.ctx(),
                HeldHover {
                    tile: row.id,
                    row: row.rect,
                    at,
                },
            );
        }
        let Some(held) = held_hover(ui.ctx()).filter(|held| held.tile == row.id) else {
            return row;
        };
        // Over the finger rather than the middle of the row, which runs the width of the panel.
        // The row's own height still, so the text clears the row and not merely the touch.
        let finger = egui::Rect::from_min_max(
            egui::pos2(held.at, row.rect.top()),
            egui::pos2(held.at, row.rect.bottom()),
        );
        // `for_widget` rather than `for_enabled`, which would close the moment the finger lifted:
        // what keeps this open is the hold, not egui's own reading of the pointer.
        let mut tooltip = egui::Tooltip::for_widget(&row).gap(HOVER_FINGER_GAP);
        tooltip.popup = tooltip
            .popup
            .anchor(finger)
            .align(egui::RectAlign::TOP)
            .align_alternatives(&HOVER_ALIGNS);
        tooltip.show(|ui| ui.label(hover));
        row
    }
}

/// Whether the finger on this row has been there long enough, and stayed near enough to where it
/// landed, to be reading what the row is rather than pressing it.
///
/// Timed here rather than left to [`egui::Tooltip::should_show_tooltip`], which restarts its wait
/// on every movement and by default refuses unless the pointer is still. A hand never is.
fn held_long_enough(row: &egui::Response) -> bool {
    row.ctx.input(|input| {
        let pointer = &input.pointer;
        let held = pointer
            .press_start_time()
            .is_some_and(|started| input.time - started >= HOVER_HOLD_SECONDS);
        let near = pointer
            .press_origin()
            .zip(pointer.latest_pos())
            .is_none_or(|(from, at)| from.distance(at) <= HOVER_HOLD_SLACK);
        held && near
    })
}

/// The row whose hover text a finger called up, where that row was, and where along it the finger
/// landed.
///
/// egui closes a tooltip as soon as the pointer leaves, which on a touch screen is the instant the
/// finger lifts. Held here across frames instead and let go by [`release_held_hover`]. The rect
/// tells a press meant for the text from any other, and `at` keeps the text where the finger was.
#[derive(Clone, Copy)]
struct HeldHover {
    tile: egui::Id,
    row: egui::Rect,
    /// Where the finger came down, across the row. Taken once, from where the press started: read
    /// afresh every frame it would shiver along with the hand.
    at: f32,
}

fn held_hover(ctx: &egui::Context) -> Option<HeldHover> {
    ctx.data(|data| data.get_temp(egui::Id::new(HELD_HOVER_ID)))
}

fn hold_hover(ctx: &egui::Context, held: HeldHover) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(HELD_HOVER_ID), held));
}

fn release_hover(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<HeldHover>(egui::Id::new(HELD_HOVER_ID)));
}

/// Let a held hover go on the first press that lands off its row.
///
/// Once per frame and before anything is drawn, rather than from the row holding it: the press
/// that starts a hold on another row is one of the presses this drops, and a tooltip still
/// claiming the layer would keep that row's own from opening.
pub(super) fn release_held_hover(ctx: &egui::Context) {
    let Some(held) = held_hover(ctx) else { return };
    let pressed = ctx.input(|input| {
        input
            .pointer
            .any_pressed()
            .then(|| input.pointer.interact_pos())
            .flatten()
    });
    if pressed.is_some_and(|at| !held.row.contains(at)) {
        release_hover(ctx);
    }
}
