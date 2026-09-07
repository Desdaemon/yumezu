//! Everything this app says, in whichever language it is being spoken.
//!
//! Fluent, because half of what is said is a count and the languages disagree about how a count
//! is said -- English has one form for one world and another for the rest, Japanese has one for
//! both -- and that disagreement belongs in `locales/<tag>/main.ftl` rather than in the panel.
//!
//! Every language is parsed at once because English has to be resident whatever is being spoken:
//! it is what the rest fall back to for a message not written in them yet.
//!
//! Nothing here is locked. The messages never change, and the one thing that does -- which
//! language is being spoken -- is a single integer, so it is an atomic of its own rather than a
//! field behind a lock over the messages. [`speaking`] is read far harder than the messages are:
//! once per world name on screen, a few thousand times a frame.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use unic_langid::LanguageIdentifier;

// Kept as a tag rather than a number, so a store written by a version that offered a different
// set of languages still says which one it meant.
const LANGUAGE: &str = "language";

/// A closed set rather than a table of tags, so anything reading differently in one language than
/// another can say so in a `match` the compiler checks. Adding a language is a variant here, an
/// entry in [`Language::ALL`], and a file for [`Language::ftl`] to name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Language {
    /// The fallback, and so the one language that has to carry every message.
    English,
    Japanese,
}

impl Language {
    /// In the order the picker offers them, English first as the fallback.
    pub(super) const ALL: [Language; 2] = [Language::English, Language::Japanese];

    /// BCP 47, which is what the store keeps and what a device asks for its language in.
    pub(super) fn tag(self) -> &'static str {
        match self {
            Self::English => "en-US",
            Self::Japanese => "ja",
        }
    }

    fn ftl(self) -> &'static str {
        match self {
            Self::English => include_str!("../locales/en-US/main.ftl"),
            Self::Japanese => include_str!("../locales/ja/main.ftl"),
        }
    }

    /// Read out of its own messages rather than the language that happens to be open, so someone
    /// who cannot read the one the app came up in can still find theirs in the picker.
    pub(super) fn name(self) -> String {
        CATALOG
            .say(self, "language-name", None)
            .unwrap_or_else(|| self.tag().to_owned())
    }
}

/// An index into [`Language::ALL`], or [`UNSETTLED`] before anything has asked. Relaxed
/// throughout: the integer is the whole of what is wanted, and nothing is published alongside it
/// for another thread to have to see first.
static SPEAKING: AtomicUsize = AtomicUsize::new(UNSETTLED);

/// Settled on the first ask rather than at startup, so nothing has to remember to settle it.
const UNSETTLED: usize = usize::MAX;

/// Immutable once built, which is what leaves this without a lock. The bundles are the concurrent
/// ones for the same reason: a plain [`fluent_bundle::FluentBundle`] is not [`Sync`] and so
/// cannot be a `static` at all.
static CATALOG: LazyLock<Catalog> = LazyLock::new(Catalog::new);

struct Catalog {
    /// One per entry of [`Language::ALL`], in the same order.
    bundles: Vec<FluentBundle<FluentResource>>,
}

impl Catalog {
    /// Panics on a malformed file: the files are compiled in, so a failure here is a broken build
    /// rather than anything the running app could recover from.
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
                // Fluent otherwise wraps every substituted value in the marks that keep a
                // right-to-left value from reordering the sentence around it. Nothing here is
                // written right-to-left, and egui draws the marks as empty boxes.
                bundle.set_use_isolating(false);
                bundle
                    .add_resource(resource)
                    .expect("a language names a message twice");
                bundle
            })
            .collect();
        Self { bundles }
    }

    /// A message whose values do not add up counts as unsaid, so it falls through to the next
    /// language and finally to its own name: a name on screen is a broken message anybody can
    /// report, where a half-substituted sentence is not.
    fn say(&self, language: Language, id: &str, args: Option<&FluentArgs>) -> Option<String> {
        let bundle = &self.bundles[language as usize];
        let pattern = bundle.get_message(id)?.value()?;
        let mut errors = Vec::new();
        let said = bundle.format_pattern(pattern, args, &mut errors);
        errors.is_empty().then(|| said.into_owned())
    }
}

/// One relaxed load of [`SPEAKING`] and no lock, which is what lets a caller ask per world name
/// rather than be told once and carry the answer around.
pub(super) fn speaking() -> Language {
    match SPEAKING.load(Relaxed) {
        UNSETTLED => {
            // Racers work out the same answer -- it is read off the store and the device, neither
            // of which changes under this -- so neither has to win.
            let language = chosen();
            SPEAKING.store(language as usize, Relaxed);
            language
        }
        // Indexed rather than matched, and no panic out of range: nothing but this module writes
        // the integer, and this is read too often to carry the branch.
        at => Language::ALL.get(at).copied().unwrap_or(Language::English),
    }
}

/// The one language whose glyphs no font this app starts with carries, and the one the wiki has a
/// second site for, so it is asked about often enough to be worth its own name. See
/// [`super::japanese`] and `world::yume2kki_t_url`.
pub(super) fn speaking_japanese() -> bool {
    speaking() == Language::Japanese
}

pub(super) fn speak(language: Language) {
    SPEAKING.store(language as usize, Relaxed);
    super::store::write(LANGUAGE, Some(language.tag()));
}

/// Reached through [`t!`] rather than called, which is what names the values a message asks for
/// at the point it is said.
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
/// The names are the message's own `$variables`, so a name that does not match one leaves the
/// message unsaid rather than silently dropping a number out of a sentence. See [`format`].
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

/// For the tests that assert on what something says: a test has nobody to have chosen a language
/// and would otherwise read out in whichever one the machine running it is set to. Not written to
/// the store.
#[cfg(test)]
pub(super) fn speak_english() {
    SPEAKING.store(Language::English as usize, Relaxed);
}

fn chosen() -> Language {
    super::store::read(LANGUAGE)
        .and_then(|tag| matching(&tag))
        // In the order the device prefers them, so one asking for two this app has is answered in
        // the one it would rather read.
        .or_else(|| sys_locale::get_locales().find_map(|tag| matching(&tag)))
        .unwrap_or(Language::English)
}

/// The whole tag first, then the language on its own: a device asking for `ja-JP` wants the
/// Japanese this app has, and one asking for `en-GB` is better served by the American English
/// here than by nothing.
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

    // Read off the file rather than out of the bundle: a bundle answers whether it has a message
    // but will not list the ones it has.
    fn named(ftl: &str) -> Vec<&str> {
        ftl.lines()
            // A message starts a line; comments are prefixed and both variants and continued
            // values are indented.
            .filter(|line| line.starts_with(|first: char| first.is_ascii_alphabetic()))
            .filter_map(|line| line.split_once('=').map(|(id, _)| id.trim()))
            .collect()
    }

    // A message English has not got falls back to nothing and is read out as its own name.
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

    // The one failure nothing else guards against: a message asking for a value by a name nothing
    // passes it compiles, parses, and is read out on screen as `some-message-id`. So every message
    // is formatted against the whole set of values the app ever passes.
    #[test]
    fn every_english_message_says_something() {
        let mut args = fluent_bundle::FluentArgs::new();
        for numeric in [
            "count",
            "total",
            "shown",
            "connections",
            "degree",
            "depth",
            "seen",
        ] {
            args.set(numeric, 1);
        }
        for worded in [
            "fps", "worlds", "name", "released", "title", "kind", "out", "back", "effects",
            "chance", "season", "percent", "when",
        ] {
            args.set(worded, "x");
        }
        for id in named(Language::English.ftl()) {
            // Asked of the bundle rather than through `format`, which answers with the name of a
            // message it cannot say -- and a couple of these are worded the same as their own
            // name, so the name is no evidence.
            assert!(
                super::CATALOG
                    .say(Language::English, id, Some(&args))
                    .is_some(),
                "{id} says nothing: see whether it asks for a value by a name no caller passes"
            );
        }
    }

    // Names the app builds rather than writes out, and which the compiler therefore cannot check.
    // See `Guide::show` and `showing`.
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
