//! What a connection demands of a player: the wiki's own sentences where it wrote any, and the
//! effect names within them as the locale files name them. See [`sentences`] and [`glyphs`].

use egui_material_icons::{
    MaterialIcon,
    icons::{ICON_ARROW_BACK, ICON_ARROW_FORWARD, ICON_ARROW_RANGE, ICON_BLOCK},
};

use super::*;

/// The wiki's thirty-five effects, in the order the game gives them, and the only names it writes
/// a condition's effects in. What each is called on screen is `effect-<name>` in the locale files.
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

/// The message naming an effect on screen, from the name the wiki writes it by.
pub fn effect_message(effect: &str) -> String {
    format!("effect-{}", effect.to_lowercase().replace(' ', "-"))
}

/// Empty for a condition that demands nothing: a row with nothing after the title is a way a
/// player can walk unconditionally.
pub fn gate_sentence(gate: Gate) -> String {
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
/// that demands nothing.
///
/// All of them rather than the harshest alone: a way that is locked *and* wants an effect is not
/// walkable by meeting either, and a reader told only the lock would go and fail.
pub fn sentences(conditions: &Conditions) -> String {
    conditions
        .demands
        .iter()
        .map(demand_sentence)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The wiki's own words where it has any, and the bare name of the condition otherwise.
pub(super) fn demand_sentence(demand: &Demand) -> String {
    let Some(detail) = demand.detail.as_deref() else {
        return gate_sentence(demand.gate);
    };
    match demand.gate {
        Gate::Effect => t!("gate-effect-detail", effects = effects(detail)),
        Gate::Chance => t!("gate-chance-detail", chance = detail),
        Gate::Seasonal => t!("gate-seasonal-detail", season = detail),
        // The wiki's own sentence, which it writes in English and publishes no Japanese for.
        Gate::LockedCondition | Gate::Revisit => detail.to_owned(),
        _ => gate_sentence(demand.gate),
    }
}

/// What a connection demands in effects, in whichever language is being spoken.
///
/// Listed as the wiki lists them and joined no further: the wiki does not say whether one effect is
/// enough or all are needed, and an "and" or "or" would settle it here.
pub(super) fn effects(detail: &str) -> String {
    corrected(detail)
        .split(',')
        .map(|listed| named_effects(listed.trim()))
        .collect::<Vec<_>>()
        .join(&t!("effect-separator"))
}

/// An entity the dump leaves unescaped, and a name it writes two ways.
///
/// TODO: corrections that belong on yume.wiki rather than here.
pub(super) fn corrected(detail: &str) -> String {
    detail
        .replace("&comma;", ",")
        .replace("Teru Teru Bōzu", "Teru Teru Bozu")
}

/// The effect names within the wiki's own words, as the locale files name them. What lies between
/// them stays as the wiki wrote it, which of a separator, an "or" or a note it means being the
/// wiki's to say.
pub(super) fn named_effects(detail: &str) -> String {
    let mut said = String::with_capacity(detail.len());
    let mut at = 0;
    while at < detail.len() {
        let rest = &detail[at..];
        // A name only where a word starts and ends, so `Springfield` is not the Spring effect.
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
pub fn glyphs(conditions: &Conditions) -> String {
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

/// [`ICON_BLOCK`] is the connection the dump lists but neither side can walk.
pub fn arrow(step: &Step) -> MaterialIcon {
    match (step.out.is_some(), step.back.is_some()) {
        (true, true) => ICON_ARROW_RANGE,
        (true, false) => ICON_ARROW_FORWARD,
        (false, true) => ICON_ARROW_BACK,
        (false, false) => ICON_BLOCK,
    }
}
