//! Everything this app says, in whichever language it is being spoken.
//!
//! One message per thing said, kept in `locales/<tag>/main.ftl` and named there rather than
//! written out where it is read: see [`t`]. Fluent, because half of what is said is a count of
//! something and the languages disagree about how a count is said -- English has one form for
//! one world and another for the rest, Japanese has one for both -- and that disagreement
//! belongs in the files rather than in the panel.
//!
//! Every language is parsed at once, on the first thing said, because English is not only one of
//! them: it is what every other language falls back to for a message not written in it yet, so
//! it has to be resident whatever is being spoken.
//!
//! Nothing here is locked. The parsed messages never change once they are parsed, and the one
//! thing that does change -- which language is being spoken -- is a single integer, so it is an
//! atomic of its own rather than a field behind a lock over the messages. That matters because
//! [`speaking`] is not read at the rate the messages are: a message is said once per label, and
//! the language is asked for once per world name on screen, a few thousand times a frame. See
//! [`world::Title::show`](super::world::Title::show).

use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use unic_langid::LanguageIdentifier;

/// What the chosen language is kept under. A tag rather than a number, so a store written by a
/// version that offered a different set of languages still says which one it meant. See
/// [`super::store`].
const LANGUAGE: &str = "language";

/// A language this app is written in.
///
/// A closed set rather than a table of tags, so that anything that reads differently in one
/// language than another can say so in a `match` the compiler checks. Adding a language is a
/// variant here, an entry in [`Language::ALL`], and a file for [`Language::ftl`] to name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Language {
    /// The fallback, and not merely by custom: it is what every other language falls back to for
    /// a message not written in it yet, so it is the one that has to carry every message.
    English,
    Japanese,
}

impl Language {
    /// All of them, in the order the picker offers them. English first, being the fallback.
    pub(super) const ALL: [Language; 2] = [Language::English, Language::Japanese];

    /// BCP 47, which is what the store keeps and what a device asks for its language in.
    pub(super) fn tag(self) -> &'static str {
        match self {
            Self::English => "en-US",
            Self::Japanese => "ja",
        }
    }

    /// Its messages, as published.
    fn ftl(self) -> &'static str {
        match self {
            Self::English => include_str!("../locales/en-US/main.ftl"),
            Self::Japanese => include_str!("../locales/ja/main.ftl"),
        }
    }

    /// What it calls itself, which is what the picker offers it as.
    ///
    /// Read out of its own messages, so a language names itself rather than being named by
    /// whichever one happens to be open: someone who cannot read the language the app came up in
    /// can still find their own in the list.
    pub(super) fn name(self) -> String {
        CATALOG
            .say(self, "language-name", None)
            .unwrap_or_else(|| self.tag().to_owned())
    }
}

/// Which of [`Language::ALL`] is being spoken, as its own place in that array, or [`UNSETTLED`]
/// before anything has asked.
///
/// Relaxed throughout: the integer is the whole of what is wanted, and nothing is published
/// alongside it for another thread to have to see first.
static SPEAKING: AtomicUsize = AtomicUsize::new(UNSETTLED);

/// What [`SPEAKING`] holds before the first ask settles it. Settled on the first ask rather than
/// at startup so that nothing has to remember to settle it.
const UNSETTLED: usize = usize::MAX;

/// The parsed messages.
///
/// Immutable once built, which is what leaves this without a lock. The bundles are the concurrent
/// ones for the same reason: a plain [`fluent_bundle::FluentBundle`] is not [`Sync`] and so
/// cannot be a `static` at all.
static CATALOG: LazyLock<Catalog> = LazyLock::new(Catalog::new);

struct Catalog {
    /// One per entry of [`Language::ALL`], in the same order.
    bundles: Vec<FluentBundle<FluentResource>>,
}

impl Catalog {
    /// Parses every language.
    ///
    /// Panics on a malformed file: the files are compiled in, so a failure here is a broken build
    /// rather than something the running app could recover from -- the same reason `data.json` is
    /// parsed the way it is.
    fn new() -> Self {
        let bundles = Language::ALL
            .into_iter()
            .map(|language| {
                let tag: LanguageIdentifier = language
                    .tag()
                    .parse()
                    .expect("a language is tagged with something that is not a language tag");
                let resource = FluentResource::try_new(language.ftl().to_owned())
                    .expect("a language's messages are not valid Fluent");
                let mut bundle = FluentBundle::new_concurrent(vec![tag]);
                // Fluent wraps every value it substitutes in the marks that keep a right-to-left
                // value from reordering the sentence around it. Nothing here is written
                // right-to-left, and egui draws the marks as the empty boxes of a glyph no font
                // carries, so they are turned off rather than shown.
                bundle.set_use_isolating(false);
                bundle
                    .add_resource(resource)
                    .expect("a language names a message twice");
                bundle
            })
            .collect();
        Self { bundles }
    }

    /// What `id` says in one language, or `None` where that language does not say it.
    ///
    /// A message whose values do not add up counts as unsaid, so it falls through to the next
    /// language and finally to its own name: a name on screen is a broken message anybody can
    /// report, where a half-substituted sentence is one nobody can.
    fn say(&self, language: Language, id: &str, args: Option<&FluentArgs>) -> Option<String> {
        let bundle = &self.bundles[language as usize];
        let pattern = bundle.get_message(id)?.value()?;
        let mut errors = Vec::new();
        let said = bundle.format_pattern(pattern, args, &mut errors);
        errors.is_empty().then(|| said.into_owned())
    }
}

/// Which language is being spoken. Whatever reads differently in one language than another reads
/// this to find out which it is in.
///
/// Free of any lock: one relaxed load of [`SPEAKING`], which is what lets a caller ask this per
/// world name rather than having to be told once and carry the answer around.
pub(super) fn speaking() -> Language {
    match SPEAKING.load(Relaxed) {
        UNSETTLED => {
            // Whoever asks first works out the same answer as anyone racing them, since it is
            // read off the store and the device and neither changes under this. So the racers
            // settle on one of two identical answers and neither has to win.
            let language = chosen();
            SPEAKING.store(language as usize, Relaxed);
            language
        }
        // Indexed rather than matched, and without a panic for a value out of range: nothing but
        // this module writes the integer, so there is no such value, and the branch a bounds
        // check would leave behind is one this is read too often to carry.
        at => Language::ALL.get(at).copied().unwrap_or(Language::English),
    }
}

/// Whether what is being spoken is Japanese.
///
/// The one language whose glyphs no font this app starts with carries, and the one the wiki has a
/// second site for, so it is asked about by name often enough to be worth asking for by name.
/// See [`super::japanese`] and `world::yume2kki_t_url`.
pub(super) fn speaking_japanese() -> bool {
    speaking() == Language::Japanese
}

/// Speaks another language from here on, and remembers to open in it next time.
pub(super) fn speak(language: Language) {
    SPEAKING.store(language as usize, Relaxed);
    super::store::write(LANGUAGE, Some(language.tag()));
}

/// What `id` says, in the language being spoken.
///
/// Read through [`t!`] rather than called, which is what names the values a message asks for at
/// the point it is said.
pub(super) fn format(id: &str, args: Option<&FluentArgs>) -> String {
    let speaking = speaking();
    CATALOG
        .say(speaking, id, args)
        .or_else(|| match speaking {
            Language::English => None,
            _ => CATALOG.say(Language::English, id, args),
        })
        .unwrap_or_else(|| id.to_owned())
}

/// What a message says, by name, with the values it asks for named where it is said:
///
/// ```ignore
/// t!("graph-size", worlds = 1574, connections = 4402)
/// ```
///
/// The names are the message's own `$variables`, so a message and the place it is said read
/// alike, and a name that does not match one leaves the message unsaid rather than silently
/// dropping a number out of a sentence. See [`format`].
macro_rules! t {
    ($id:literal) => {
        $crate::i18n::format($id, None)
    };
    ($id:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = fluent_bundle::FluentArgs::new();
        $(args.set(stringify!($name), $value);)+
        $crate::i18n::format($id, Some(&args))
    }};
}
pub(super) use t;

/// Reads out of English for the rest of this process, and does not remember it.
///
/// For the tests that assert on what something says. A test has nobody to have chosen a language
/// and would otherwise read out in whichever one the machine running it is set to.
#[cfg(test)]
pub(super) fn speak_english() {
    SPEAKING.store(Language::English as usize, Relaxed);
}

/// The language to open in: the one an earlier run was left in, or the closest this app has to
/// one the device asks for, or English.
fn chosen() -> Language {
    super::store::read(LANGUAGE)
        .and_then(|tag| matching(&tag))
        // Every language the device asks for, in the order it prefers them, so a device that
        // asks for two this app has is answered in the one it would rather read.
        .or_else(|| sys_locale::get_locales().find_map(|tag| matching(&tag)))
        .unwrap_or(Language::English)
}

/// Which language answers to `tag`, if any does.
///
/// The whole tag first, then the language on its own: a device asking for `ja-JP` is asking for
/// the Japanese this app has, and one asking for `en-GB` is better served by the American
/// English here than by nothing.
fn matching(tag: &str) -> Option<Language> {
    let wanted: LanguageIdentifier = tag.parse().ok()?;
    let tags: Vec<LanguageIdentifier> = Language::ALL
        .into_iter()
        .map(|language| {
            language
                .tag()
                .parse()
                .expect("a language tag that is not one")
        })
        .collect();
    let at = tags.iter().position(|have| *have == wanted).or_else(|| {
        tags.iter()
            .position(|have| have.language == wanted.language)
    })?;
    Some(Language::ALL[at])
}

#[cfg(test)]
mod tests {
    use super::Language;

    /// The messages a language names, read off the file rather than out of the bundle: a bundle
    /// answers whether it has a message but will not list the ones it has.
    fn named(ftl: &str) -> Vec<&str> {
        ftl.lines()
            // A message starts a line, where a comment is prefixed, a variant is indented and a
            // value continued over several lines is indented too.
            .filter(|line| line.starts_with(|first: char| first.is_ascii_alphabetic()))
            .filter_map(|line| line.split_once('=').map(|(id, _)| id.trim()))
            .collect()
    }

    /// Every language names itself, which is what the picker offers it as, and names nothing
    /// English does not: a message English has not got is one that falls back to nothing and is
    /// read out as its own name. See [`super::format`].
    #[test]
    fn every_language_is_named_and_says_nothing_english_does_not() {
        let english = named(Language::English.ftl());
        for language in Language::ALL {
            assert!(
                super::CATALOG
                    .say(language, "language-name", None)
                    .is_some(),
                "{} does not name itself",
                language.tag()
            );
            for id in named(language.ftl()) {
                assert!(
                    english.contains(&id),
                    "{} says {id}, which English does not",
                    language.tag()
                );
            }
        }
    }

    /// Every English message says something, rather than falling through to its own name.
    ///
    /// The one failure this design has no other guard against: a message that asks for a value by
    /// a name nothing passes it is read out as `some-message-id` on screen, and it compiles and
    /// parses either way. So every message is formatted here against the whole set of values the
    /// app ever passes -- which means a message asking for anything outside that set is a message
    /// nothing can satisfy, and that is the failure being caught.
    #[test]
    fn every_english_message_says_something() {
        let mut args = fluent_bundle::FluentArgs::new();
        for numeric in ["count", "total", "shown", "connections", "degree", "depth"] {
            args.set(numeric, 1);
        }
        for worded in [
            "fps", "worlds", "name", "released", "title", "kind", "out", "back", "effects",
            "chance", "season",
        ] {
            args.set(worded, "x");
        }
        for id in named(Language::English.ftl()) {
            // Asked of the bundle rather than through [`super::format`], which answers with the
            // name of a message it cannot say -- and a couple of these are worded the same as
            // their own name, so the name is not the evidence. See [`super::Catalog::say`].
            assert!(
                super::CATALOG
                    .say(Language::English, id, Some(&args))
                    .is_some(),
                "{id} says nothing: see whether it asks for a value by a name no caller passes"
            );
        }
    }

    /// The messages whose names the app builds rather than writes out, and which the compiler
    /// therefore cannot check: the two halves of every row of the controls, and either wording of
    /// how much of a list is being shown. See `Guide::show` and `showing`.
    #[test]
    fn the_messages_named_at_runtime_are_all_there() {
        let rows = [
            "fly",
            "strafe",
            "orbit-mouse",
            "orbit-touch",
            "options",
            "pan",
            "pinch",
            "scroll",
        ];
        let built = rows
            .into_iter()
            .flat_map(|row| [format!("guide-{row}-input"), format!("guide-{row}-action")])
            .chain(
                ["authors", "versions"]
                    .into_iter()
                    .flat_map(|list| [format!("showing-{list}"), format!("showing-{list}-cut")]),
            );
        for id in built {
            assert!(
                super::CATALOG.bundles[Language::English as usize].has_message(&id),
                "English does not say {id}"
            );
        }
    }
}
