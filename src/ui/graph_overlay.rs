//! The controls and labels that stand over the graph rather than in the sidebar: the layer
//! rocker, the right-click menu, and the hover tooltip.

use super::*;

impl Panel {
    /// Steps the lit layer in and out from the origin.
    ///
    /// An [`egui::Area`] rather than part of the sidebar, so it sits in the corner the graph is
    /// visible in whatever the sidebar shows. Its middle is the off position, so a layer can be
    /// dropped without clicking a world.
    pub(super) fn rocker(&mut self, ui: &mut egui::Ui, read: &PanelData, insets: egui::Margin) {
        let deepest = read.data.deepest;
        let lit = match read.selected {
            Some(Highlight::Layer(depth)) => Some(depth),
            _ => None,
        };
        egui::Area::new(egui::Id::new("layer rocker"))
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                [
                    -((insets.right + PANEL_MARGIN) as f32),
                    -((insets.bottom + PANEL_MARGIN) as f32),
                ],
            )
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    // Scaled rather than sized: the arrows and the padding around them grow
                    // together, so the buttons keep their shape and still follow the UI scale.
                    let style = ui.style_mut();
                    for font in style.text_styles.values_mut() {
                        font.size *= ROCKER_SCALE;
                    }
                    style.spacing.button_padding *= ROCKER_SCALE;
                    style.spacing.item_spacing *= ROCKER_SCALE;
                    ui.vertical_centered(|ui| {
                        if ui
                            .add_enabled(lit != Some(0), egui::Button::new(ICON_KEYBOARD_ARROW_UP))
                            .on_hover_text(t!("rocker-shallower"))
                            .clicked()
                        {
                            self.lit = Some(Some(Highlight::Layer(
                                lit.map_or(deepest, |depth| depth.saturating_sub(1)),
                            )));
                        }
                        if ui
                            .add_enabled(
                                lit != Some(deepest),
                                egui::Button::new(ICON_KEYBOARD_ARROW_DOWN),
                            )
                            .on_hover_text(t!("rocker-deeper"))
                            .clicked()
                        {
                            self.lit = Some(Some(Highlight::Layer(
                                lit.map_or(0, |depth| (depth + 1).min(deepest)),
                            )));
                        }
                    });
                });
            });
    }

    pub(super) fn menu(&mut self, ui: &mut egui::Ui, read: &PanelData) {
        let Some((world, at)) = read.menu else {
            return;
        };
        let data = read.data;
        egui::Area::new(egui::Id::new("world menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_max_width(POPUP_WIDTH);
                    ui.strong(data.titles[world].show());
                    ui.label(data.authors[data.author_of[world]].name.show());
                    if data.descendants[world] > 0 && ui.button(t!("menu-descendants")).clicked() {
                        self.lit = Some(Some(Highlight::Descendants(world)));
                    }
                    self.directions_menu(ui, read, world);
                    let named = data.titles[world].known();
                    if named && ui.button(t!("menu-open-wiki")).clicked() {
                        open_in_browser(&data.titles[world].wiki_url());
                        self.menu_taken = true;
                    }
                    if named && speaking_japanese() && ui.button(t!("world-english-wiki")).clicked()
                    {
                        open_in_browser(&world::wiki_url(&data.titles[world].en));
                        self.menu_taken = true;
                    }
                    if !data.maps[world].is_empty() && ui.button(t!("world-map-hint")).clicked() {
                        self.mapped = Some(world);
                        self.menu_taken = true;
                    }
                    // Such a world only exists while the graph is a frontier, so the entry is not
                    // there to be wondered at in an ordinary run.
                    if !named
                        && ui
                            .button(t!("menu-reveal"))
                            .on_hover_text(t!("menu-reveal-hint"))
                            .clicked()
                    {
                        self.revealed = Some(world);
                        self.menu_taken = true;
                    }
                });
            });
    }

    /// Not while a menu is open: the menu is over the same world and already names it.
    pub(super) fn tooltip(&self, ui: &mut egui::Ui, read: &PanelData) {
        let Some((world, at)) = read.hovered.filter(|_| read.menu.is_none()) else {
            return;
        };
        egui::Area::new(egui::Id::new("world tooltip"))
            .order(egui::Order::Tooltip)
            // Kept inside the window, so a world hovered near the right edge is still readable.
            .constrain(true)
            .fixed_pos(at + egui::vec2(TOOLTIP_OFFSET, TOOLTIP_OFFSET))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(POPUP_WIDTH);
                    ui.strong(read.data.titles[world].show());
                    ui.label(read.data.authors[read.data.author_of[world]].name.show());
                });
            });
    }
}
