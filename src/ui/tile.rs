//! The row every list in the panel is built out of, and the long press that stands in for a hover
//! on a touch screen. See [`Tile`].

use super::*;

/// A list tile: leading atoms, a title and trailing atoms, all one button the width of the list.
/// The whole row answers a press, and a thumb can see how far the row reaches.
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

    /// The text a pointer reads by hovering the row, and a finger by holding the row.
    /// On a touch screen, drawn [`HOVER_FINGER_GAP`] clear of the fingertip.
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
        // The title shrinks first: a truncated leading or trailing glyph says nothing.
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
        // A click releases the hover text; only a press held past a click keeps the text.
        if row.clicked() {
            release_hover(ui.ctx());
            return row;
        }
        let pressing = row.is_pointer_button_down_on();
        if pressing {
            // A resting finger sends no events.
            ui.ctx().request_repaint();
        }
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
        // Over the finger, since the row spans the panel, and as tall as the row.
        let finger = egui::Rect::from_min_max(
            egui::pos2(held.at, row.rect.top()),
            egui::pos2(held.at, row.rect.bottom()),
        );
        // `for_widget`: `for_enabled` closes as the finger lifts, and the hold keeps this open.
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

/// Whether the finger has rested long enough, near where the press landed, to be reading the row.
/// [`egui::Tooltip::should_show_tooltip`] restarts on any movement, and a finger never keeps still.
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

/// The row whose hover text a finger called up, with the row's rect and the press position.
/// Kept across frames until [`release_held_hover`]: egui closes a tooltip as the finger lifts.
#[derive(Clone, Copy)]
struct HeldHover {
    tile: egui::Id,
    row: egui::Rect,
    /// Where the finger came down across the row, fixed at the press origin to keep the text still.
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

/// Releases a held hover on the first press that lands off the held row.
/// Called once per frame before drawing, so a hold on another row finds the tooltip layer free.
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
