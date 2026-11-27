//! The settings tab: the knobs, the YNOproject account, and what the dump behind the graph is
//! dated. See [`Panel::settings`].

use chrono::{Datelike, Timelike};

use super::*;

impl Panel {
    /// A preview of the theme in use, beside the button that opens the picker dialog.
    fn system_theme(&mut self, ui: &mut egui::Ui, picking: &mut bool) {
        ui.horizontal(|ui| {
            let preview = skin::skin_button(ui, self.skin, false, SYSTEM_THEME_PREVIEW);
            if preview.clicked() || ui.button(t!("system-theme")).clicked() {
                *picking = true;
            }
        });
    }

    /// Choosing applies at once, so the dialog stays open to compare one theme with the next.
    pub(super) fn system_theme_dialog(&mut self, ctx: &egui::Context, picking: &mut bool) {
        egui::Window::new(t!("system-theme"))
            .id(egui::Id::new(SYSTEM_THEME_ID))
            .open(picking)
            .collapsible(false)
            .auto_sized()
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                let room = ctx.content_rect().width()
                    - 2.0 * (f32::from(PANEL_MARGIN) + f32::from(skin::WINDOW_PADDING));
                let width = SYSTEM_THEME_WIDTH.min(room);
                ui.set_width(width);
                let spacing = ui.spacing().item_spacing.x;
                let columns = ((width + spacing) / (SYSTEM_THEME_SIDE + spacing))
                    .floor()
                    .max(1.0);
                let button_size = egui::Vec2::splat((width + spacing) / columns - spacing);
                let skins: Vec<Skin> = Skin::every().collect();
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * SYSTEM_THEME_HEIGHT_SHARE)
                    .show(ui, |ui| {
                        for row in skins.chunks(columns as usize) {
                            ui.horizontal(|ui| {
                                for &skin in row {
                                    let chosen = skin == self.skin;
                                    let button = skin::skin_button(ui, skin, chosen, button_size);
                                    if button.clicked() {
                                        self.skin = skin;
                                    }
                                }
                            });
                        }
                    });
            });
    }

    /// A separate tab, as the knobs are set once and then left alone.
    pub(super) fn settings(
        &mut self,
        ui: &mut egui::Ui,
        browse: &mut Browse,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
    ) {
        self.language(ui);
        self.system_theme(ui, &mut browse.picking_theme);
        ui.add(
            egui::Slider::new(&mut self.hub_repulsion, HUB_REPULSION_RANGE)
                .logarithmic(true)
                .text(t!("hub-push")),
        )
        .on_hover_text(t!("hub-push-hint"));
        ui.add(egui::Slider::new(&mut self.link_reach, LINK_REACH_RANGE).text(t!("link-reach")))
            .on_hover_text(t!("link-reach-hint"));
        ui.add_enabled(
            self.arrangement.radial(),
            egui::Checkbox::new(&mut self.pull_across_branches, t!("pull-across-branches")),
        )
        .on_hover_text(t!("pull-across-branches-hint"))
        .on_disabled_hover_text(t!("pull-across-branches-hint"));
        self.restart_layout |= ui
            .button(t!("restart-layout"))
            .on_hover_text(t!("restart-layout-hint"))
            .clicked();
        ui.add(egui::Slider::new(&mut self.ui_scale, UI_SCALE_RANGE).text(t!("ui-scale")));
        self.antialiasing(ui);
        self.row_follow_switch(ui);
        profile::controls(ui);
        // Restores a guide dismissed for good.
        self.guide |= ui.button(t!("show-controls")).clicked();
        Self::clear_cache(ui);
        app_update::controls(ui);
        Self::freshness(ui, dump);
        Self::yno(ui, browse, account, dump);

        let github = format!("{GITHUB}  {}", t!("github-link"));
        browser_link(ui, github, "https://github.com/Desdaemon/yumezu");

        if let Some(platform) = download::Platform::detected() {
            let label = format!(
                "{}  {}",
                platform.icon().codepoint,
                t!("download-for", platform = platform.name())
            );
            browser_link(ui, label, download::RELEASES);
        }
    }

    /// How old the graph is: when the dump was built, and when the wiki was last read in full.
    fn freshness(ui: &mut egui::Ui, dump: Option<&world::Dump>) {
        let Some(dump) = dump else {
            return;
        };
        if let Some(built) = dump.last_update.as_deref().and_then(Self::stamped) {
            ui.label(t!("last-update", when = built))
                .on_hover_text(t!("last-update-hint"));
        }
        if let Some(whole) = dump.last_full_update.as_deref().and_then(Self::stamped) {
            ui.label(t!("last-full-update", when = whole))
                .on_hover_text(t!("last-full-update-hint"));
        }
    }

    /// `2026-09-07T00:05:25.000Z` said in the reader's own zone, to the minute. A stamp that will
    /// not parse is not drawn at all.
    fn stamped(iso: &str) -> Option<String> {
        let when = chrono::DateTime::parse_from_rfc3339(iso)
            .ok()?
            .with_timezone(&chrono::Local);
        Some(t!(
            "stamp",
            year = when.year().to_string(),
            month = format!("{:02}", when.month()),
            day = format!("{:02}", when.day()),
            hour = format!("{:02}", when.hour()),
            minute = format!("{:02}", when.minute()),
        ))
    }

    /// Signing in to YNOproject, and drawing only as much of the graph as that account has seen.
    ///
    /// Takes no `self`: signing in rebuilds the graph, which this panel does not draw.
    fn yno(
        ui: &mut egui::Ui,
        browse: &mut Browse,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
    ) {
        ui.separator();
        ui.strong(t!("yno"));
        match account.state() {
            yno::State::SignedOut => {}
            yno::State::Working => {
                ui.label(t!("yno-working"));
            }
            yno::State::SignedIn => {
                ui.label(t!("yno-signed-in"));
            }
            // Read before the label, which borrows the same `ui`.
            yno::State::Failed(why) => {
                let color = ui.visuals().error_fg_color;
                ui.colored_label(color, why);
            }
        }
        let working = matches!(account.state(), yno::State::Working);
        if account.signed_in() {
            Self::completion(ui, account, dump);
            let mut frontier = account.frontier_wanted();
            if ui
                .checkbox(&mut frontier, t!("frontier"))
                .on_hover_text(t!("frontier-hint"))
                .changed()
            {
                account.set_frontier(frontier);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!working, egui::Button::new(t!("yno-refresh")))
                    .on_hover_text(t!("yno-refresh-hint"))
                    .clicked()
                {
                    account.refresh();
                }
                // The YNOproject login stays: ending that login signs out the game's browser too.
                if ui.button(t!("yno-sign-out")).clicked() {
                    account.sign_out();
                }
            });
            return;
        }
        ui.label(t!("yno-hint"));
        Self::promise(ui);
        ui.add(egui::TextEdit::singleline(&mut browse.yno_user).hint_text(t!("yno-user")));
        ui.add(
            egui::TextEdit::singleline(&mut browse.yno_password)
                .password(true)
                .hint_text(t!("yno-password")),
        );
        let ready = !browse.yno_user.is_empty() && !browse.yno_password.is_empty();
        if ui
            .add_enabled(ready, egui::Button::new(t!("yno-sign-in")))
            .clicked()
        {
            account.sign_in(
                std::mem::take(&mut browse.yno_user),
                std::mem::take(&mut browse.yno_password),
            );
        }
    }

    /// How much of the game this account has visited, against the whole dump: a frontier graph
    /// reads 100%. Recomputed each frame while this tab is open, as either side can change.
    fn completion(ui: &mut egui::Ui, account: &yno::Account, dump: Option<&world::Dump>) {
        let (Some(visited), Some(dump)) = (account.visited(), dump) else {
            return;
        };
        let worlds = dump.worlds.len();
        let seen = dump.visited(visited);
        let share = seen as f32 / worlds.max(1) as f32;
        ui.add(egui::ProgressBar::new(share).text(t!(
            "yno-completion",
            seen = seen,
            worlds = worlds,
            percent = format!("{:.1}", share * 100.0)
        )))
        .on_hover_text(t!("yno-completion-hint"));
    }

    /// What signing in does with the account, shown beside the fields, with a link to the source
    /// file that keeps the promise.
    fn promise(ui: &mut egui::Ui) {
        ui.label(t!("yno-promise"));
        browser_link(ui, format!("{GITHUB}  {}", t!("yno-source")), YNO_SOURCE);
    }

    /// Takes no `self`: the cache is one store behind one client, reported by [`fetch::cleared`].
    /// Native only: the web build's cache belongs to the browser.
    #[cfg(not(target_family = "wasm"))]
    fn clear_cache(ui: &mut egui::Ui) {
        let cleared = fetch::cleared();
        let clearing = cleared == fetch::Cleared::Clearing;
        if ui
            .add_enabled(!clearing, egui::Button::new(t!("clear-cache")))
            .on_hover_text(t!("clear-cache-hint"))
            .clicked()
        {
            fetch::clear();
        }
        match cleared {
            fetch::Cleared::Never => {}
            fetch::Cleared::Clearing => {
                ui.label(t!("clear-cache-clearing"));
            }
            fetch::Cleared::Done => {
                ui.label(t!("clear-cache-done"));
            }
            fetch::Cleared::Failed => {
                ui.label(t!("clear-cache-failed"));
            }
        }
    }

    /// Nothing to draw: see the native [`Self::clear_cache`] above.
    #[cfg(target_family = "wasm")]
    fn clear_cache(_: &mut egui::Ui) {}

    /// The one setting whose cost can outweigh the look, so the user chooses: see [`MULTISAMPLES`].
    fn antialiasing(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.antialias, t!("antialias"))
            .on_hover_text(t!("antialias-hint"));
        if self.antialias != self.antialias_running {
            ui.label(t!("antialias-restart"));
        }
    }

    /// The row-follow switch. See [`AppPersistent::follow_hovered_row`].
    fn row_follow_switch(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.row_follow, t!("leaning"))
            .on_hover_text(t!("leaning-hint"));
    }

    /// Each language is named in that language, so any user can find theirs. The choice applies
    /// after the frame, so no frame mixes two languages.
    fn language(&mut self, ui: &mut egui::Ui) {
        let speaking = i18n::speaking();
        let mut chosen = speaking;
        egui::ComboBox::from_label(t!("language"))
            .selected_text(speaking.name())
            .show_ui(ui, |ui| {
                for other in i18n::Language::ALL {
                    ui.selectable_value(&mut chosen, other, other.name());
                }
            });
        if chosen != speaking {
            self.language = Some(chosen);
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_family = "wasm")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    #[test]
    fn a_stamp_is_said_in_the_reader_s_zone() {
        i18n::speak_english();
        let said = Panel::stamped("2026-09-07T00:05:25.000Z").expect("a stamp that will not say");
        let (date, time) = said.split_once(' ').expect("a stamp with no time in it");
        let parts: Vec<&str> = date.split('-').collect();
        assert_eq!(parts[0].len(), 4, "{said}");
        assert!(parts[1..].iter().all(|part| part.len() == 2), "{said}");
        assert!(time.split(':').all(|part| part.len() == 2), "{said}");
    }

    #[test]
    fn a_stamp_that_will_not_parse_is_not_drawn() {
        assert_eq!(Panel::stamped("whenever"), None);
    }
}
