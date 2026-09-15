//! The settings tab: the knobs, the YNOproject account, and what the dump behind the graph is
//! dated. See [`Panel::settings`].

use chrono::{Datelike, Timelike};

use super::*;

impl Panel {
    /// A tab of its own because the knobs are set once and then left alone.
    pub(super) fn settings(
        &mut self,
        ui: &mut egui::Ui,
        sidebar: &mut Sidebar,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
    ) {
        self.language(ui);
        ui.add(
            egui::Slider::new(&mut self.hub_repulsion, HUB_REPULSION_RANGE).text(t!("hub-push")),
        )
        .on_hover_text(t!("hub-push-hint"));
        ui.add(egui::Slider::new(&mut self.link_reach, LINK_REACH_RANGE).text(t!("link-reach")))
            .on_hover_text(t!("link-reach-hint"));
        ui.add(egui::Slider::new(&mut self.ui_scale, UI_SCALE_RANGE).text(t!("ui-scale")))
            .on_hover_text(t!("ui-scale-hint"));
        self.antialiasing(ui);
        self.leaning(ui);
        profile::controls(ui);
        // The way back to a panel that was dismissed for good, so ticking that box is not a door
        // that locks behind the person who ticked it.
        self.guide |= ui.button(t!("show-controls")).clicked();
        Self::clear_cache(ui);
        update::controls(ui);
        Self::freshness(ui, dump);
        Self::yno(ui, sidebar, account, dump);

        if ui
            .hyperlink_to(
                format!("{GITHUB}  {}", t!("github-link")),
                "https://github.com/Desdaemon/yumezu",
            )
            .clicked()
        {
            open_in_browser("https://github.com/Desdaemon/yumezu");
        }

        if let Some(platform) = download::Platform::detected()
            && ui
                .hyperlink_to(
                    format!(
                        "{}  {}",
                        platform.icon().codepoint,
                        t!("download-for", platform = platform.name())
                    ),
                    download::RELEASES,
                )
                .clicked()
        {
            open_in_browser(download::RELEASES);
        }
    }

    /// How old the graph is: when the dump was built, and when the wiki behind it was last read
    /// whole. A fresh dump can still be missing an edit an incremental read never covered.
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
    /// Nothing of `self`: the account is told directly, and what comes of it is a graph built
    /// again rather than anything this panel draws.
    fn yno(
        ui: &mut egui::Ui,
        sidebar: &mut Sidebar,
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
            // The server's own words, which say whether it was the password or the network. Read
            // before the label, the color being borrowed out of the same `ui`.
            yno::State::Failed(why) => {
                let color = ui.visuals().error_fg_color;
                ui.colored_label(color, why);
            }
        }
        // The refresh below would drop a second request on the floor while one is already out.
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
                // The account is read once at startup and not again, while the person is off
                // playing the game and walking into places.
                if ui
                    .add_enabled(!working, egui::Button::new(t!("yno-refresh")))
                    .on_hover_text(t!("yno-refresh-hint"))
                    .clicked()
                {
                    account.refresh();
                }
                // Forgotten here and left standing on YNOproject: ending it there would also sign
                // out whatever browser the person plays the game in.
                if ui.button(t!("yno-sign-out")).clicked() {
                    account.sign_out();
                }
            });
            return;
        }
        ui.label(t!("yno-hint"));
        Self::promise(ui);
        ui.add(egui::TextEdit::singleline(&mut sidebar.yno_user).hint_text(t!("yno-user")));
        ui.add(
            egui::TextEdit::singleline(&mut sidebar.yno_password)
                .password(true)
                .hint_text(t!("yno-password")),
        );
        let ready = !sidebar.yno_user.is_empty() && !sidebar.yno_password.is_empty();
        if ui
            .add_enabled(ready, egui::Button::new(t!("yno-sign-in")))
            .clicked()
        {
            // Taken rather than copied: the password has no second use, and the field it was typed
            // into is the only place it was ever held.
            account.sign_in(
                std::mem::take(&mut sidebar.yno_user),
                std::mem::take(&mut sidebar.yno_password),
            );
        }
    }

    /// How much of the game this account has seen, measured against the whole dump rather than the
    /// graph beside it, which may be the frontier and would read a hundred per cent.
    ///
    /// Recomputed each frame it is drawn -- one pass over the titles, only while this tab is open
    /// -- because anything kept would go stale on either side.
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

    /// What signing in does with the account, beside the fields rather than behind a link. The
    /// link points at the file that makes the promise rather than at the project.
    fn promise(ui: &mut egui::Ui) {
        ui.label(t!("yno-promise"));
        if ui
            .hyperlink_to(format!("{GITHUB}  {}", t!("yno-source")), YNO_SOURCE)
            .clicked()
        {
            open_in_browser(YNO_SOURCE);
        }
    }

    /// Nothing of `self`, the cache's own state being the only state there is: one store behind
    /// one client, and [`fetch::cleared`] is its answer to everyone.
    ///
    /// Native only. The page's cache is the browser's, which this app neither built nor may empty.
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
        // Under the button rather than in it: the button says what it does, and this says what
        // came of the last press.
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

    /// The one setting whose cost is worth more than its look, so the choice is the person's:
    /// see [`MULTISAMPLES`].
    fn antialiasing(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.antialias, t!("antialias"))
            .on_hover_text(t!("antialias-hint"));
        if self.antialias != self.antialias_running {
            ui.label(t!("antialias-restart"));
        }
    }

    /// The motion a person is most likely to want stopped, being the one nothing called for: see
    /// [`AppStatics::lean_toward`].
    fn leaning(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.leaning, t!("leaning"))
            .on_hover_text(t!("leaning-hint"));
    }

    /// Every language names itself, so someone who cannot read the one the app opened in can still
    /// find theirs. The choice is left on the panel, so a frame half drawn in one language is not
    /// finished in another.
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

    // Padded to a fixed width by the message rather than by the caller, which is the part a
    // language is free to disagree about. The digits themselves are the machine's zone.
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
