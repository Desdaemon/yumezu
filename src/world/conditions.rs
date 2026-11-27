//! What a connection demands of a player: the wiki's own sentences where any were written, with
//! effect names as the locale files name them. See [`conditions_display`] and
//! [`conditions_glyphs`].

use egui_material_icons::{
    MaterialIcon,
    icons::{ICON_ARROW_BACK, ICON_ARROW_FORWARD, ICON_ARROW_RANGE, ICON_BLOCK},
};

use super::*;

/// The wiki's thirty-five effects, in the order the game gives them, and the only effect names
/// a condition uses. What each is called on screen is `effect-<name>` in the locale files.
pub static EFFECTS: [&str; 35] = [
    "Bike",
    "Boy",
    "Chainsaw",
    "Lantern",
    "Fairy",
    "Spacesuit",
    "Glasses",
    "Rainbow",
    "Wolf",
    "Eyeball Bomb",
    "Telephone",
    "Maiko",
    "Twintails",
    "Penguin",
    "Insect",
    "Spring",
    "Invisible",
    "Gakuran",
    "Plaster Cast",
    "Stretch",
    "Haniwa",
    "Trombone",
    "Cake",
    "Child",
    "Red Riding Hood",
    "Tissue",
    "Bat",
    "Polygon",
    "Teru Teru Bozu",
    "Marginal",
    "Drum",
    "Grave",
    "Crossing",
    "Bunny Ears",
    "Dice",
];

/// The message naming an effect on screen, from the wiki's name for the effect.
pub fn effect_message(effect: &str) -> String {
    format!("effect-{}", effect.to_lowercase().replace(' ', "-"))
}

/// Empty for [`Gate::Free`]: a row with nothing after the title is passable.
pub fn gate_display(gate: Gate) -> String {
    match gate {
        Gate::Free => String::new(),
        Gate::Effect => t!("gate-effect"),
        Gate::Chance => t!("gate-chance"),
        Gate::Seasonal => t!("gate-seasonal"),
        Gate::Locked => t!("gate-locked"),
        Gate::LockedCondition | Gate::Revisit => t!("gate-locked-condition"),
        Gate::ExitPoint => t!("gate-exit-point"),
        Gate::DeadEnd => t!("gate-dead-end"),
        Gate::Isolated => t!("gate-isolated"),
    }
}

/// Every condition the connection carries, harshest first, a line each. Empty for a connection
/// that demands nothing. A way that is locked *and* wants an effect needs both met.
pub fn conditions_display(conditions: &Conditions) -> String {
    conditions
        .demands
        .iter()
        .map(demand_display)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The wiki's own words where present, and the bare name of the condition otherwise.
pub(super) fn demand_display(demand: &Demand) -> String {
    let Some(detail) = demand.detail.as_deref() else {
        return gate_display(demand.gate);
    };
    match demand.gate {
        Gate::Effect => t!("gate-effect-detail", effects = effects(detail)),
        Gate::Chance => t!("gate-chance-detail", chance = detail),
        Gate::Seasonal => t!("gate-seasonal-detail", season = detail),
        // The wiki publishes this sentence in English only.
        Gate::LockedCondition | Gate::Revisit => detail.to_owned(),
        _ => gate_display(demand.gate),
    }
}

/// What a connection demands in effects, in the UI's language.
/// Listed without "and" or "or": the wiki does not say whether one effect is enough or all are.
pub(super) fn effects(detail: &str) -> String {
    manual_wiki_corrections(detail)
        .split(',')
        .map(|listed| named_effects(listed.trim()))
        .collect::<Vec<_>>()
        .join(&t!("effect-separator"))
}

/// Fixes an entity the dump leaves unescaped and an effect name the dump spells two ways.
///
/// TODO: corrections that belong on yume.wiki rather than here.
pub(super) fn manual_wiki_corrections(detail: &str) -> String {
    detail
        .replace("&comma;", ",")
        .replace("Teru Teru Bōzu", "Teru Teru Bozu")
}

/// The effect names within the wiki's own words, as the locale files name them. The text between
/// names, a separator, an "or" or a note, stays as the wiki wrote.
pub(super) fn named_effects(detail: &str) -> String {
    let mut said = String::with_capacity(detail.len());
    let mut at = 0;
    while at < detail.len() {
        let rest = &detail[at..];
        // Whole words only, so `Springfield` is not the Spring effect.
        let starting = detail[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric());
        let named = starting
            .then(|| {
                EFFECTS.iter().find(|effect| {
                    rest.get(..effect.len())
                        .is_some_and(|head| head.eq_ignore_ascii_case(effect))
                        && !rest[effect.len()..].starts_with(char::is_alphanumeric)
                })
            })
            .flatten();
        match named {
            Some(effect) => {
                said.push_str(&i18n::format(&effect_message(effect), None));
                at += effect.len();
            }
            None => {
                let next = rest.chars().next().expect("a non-empty string has a char");
                said.push(next);
                at += next.len_utf8();
            }
        }
    }
    said
}

/// One glyph per condition, harshest first, so a way that is locked *and* wants an effect reads as
/// both. Empty for a connection that demands nothing.
pub fn conditions_glyphs(conditions: &Conditions) -> String {
    conditions.demands.iter().map(demand_glyph).collect()
}

pub(super) fn demand_glyph(demand: &Demand) -> &'static str {
    match demand.gate {
        Gate::Free => "",
        Gate::Effect => "✨",
        Gate::Chance => "🍀",
        Gate::Locked => "🔒",
        Gate::LockedCondition | Gate::Revisit => "🔐",
        Gate::ExitPoint => "🚪",
        Gate::DeadEnd => "↩",
        Gate::Isolated => "🚩",
        Gate::Seasonal => match demand.detail.as_deref() {
            Some("Spring") => "🌸",
            Some("Summer") => "☀",
            Some("Fall") => "🍂",
            Some("Winter") => "❄",
            _ => "🗓",
        },
    }
}

/// [`ICON_BLOCK`] is the connection the dump lists but neither side can pass.
pub fn direction_icon(step: &Step) -> MaterialIcon {
    match (step.outgoing.is_some(), step.incoming.is_some()) {
        (true, true) => ICON_ARROW_RANGE,
        (true, false) => ICON_ARROW_FORWARD,
        (false, true) => ICON_ARROW_BACK,
        (false, false) => ICON_BLOCK,
    }
}
