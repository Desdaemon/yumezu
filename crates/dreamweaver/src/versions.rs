//! Reading the release names the wiki writes into a world's infobox.
//!
//! Two of them are lists packed into one string, and both pack a `-` into entries whose version
//! names may themselves begin `pre-`. So neither can be split on the first dash it happens to
//! contain; both are split on the dash that separates the fields, which is the one a version name
//! starts after.

use crate::model::{VerGap, VerUpdated};

/// Where a version name ends inside `entry`, given that one starts at its beginning.
///
/// The separator is the first `-` that is not the one inside a leading `pre-`, and that a version
/// name follows -- which is either another `pre-` or a digit. `None` where `entry` is one name
/// and nothing else.
fn separator(entry: &str) -> Option<usize> {
    let from = if entry.starts_with("pre-") { 4 } else { 0 };
    entry[from..].char_indices().find_map(|(at, c)| {
        let at = from + at;
        let rest = &entry[at + 1..];
        (c == '-' && (rest.starts_with("pre-") || rest.starts_with(|c: char| c.is_ascii_digit())))
            .then_some(at)
    })
}

/// Splits `0.120d patch 21-0.120e,0.125a-0.125b` into the spans a world was absent for.
///
/// An entry with no separator is dropped rather than guessed at: a gap is two names by
/// definition, and half of one says nothing about when the world came back.
pub fn gaps(packed: &str) -> Option<Vec<VerGap>> {
    let gaps: Vec<VerGap> = packed
        .split(',')
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let at = separator(entry)?;
            Some(VerGap {
                ver_removed: entry[..at].to_owned(),
                ver_readded: entry[at + 1..].to_owned(),
            })
        })
        .collect();
    (!gaps.is_empty()).then_some(gaps)
}

/// Splits `0.118e,0.120a-c,0.121` into the releases that changed a world.
///
/// Here the part after the separator is the wiki's shorthand for the kind of change rather than
/// another version, so an entry without one is kept: it means a release changed the world and the
/// wiki did not say how.
pub fn updates(packed: &str) -> Option<Vec<VerUpdated>> {
    let updates: Vec<VerUpdated> = packed
        .split(',')
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            // Recognised by what follows it rather than by where it is: the shorthand is one or
            // two of `a-z` and `+`, which no version name is, so the dash inside `pre-` and the
            // dash before a patch number are both left alone.
            let shorthand = entry.rfind('-').filter(|&at| {
                at > 0
                    && !entry[at + 1..].is_empty()
                    && entry[at + 1..]
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '+')
            });
            match shorthand {
                Some(at) => VerUpdated {
                    ver_updated: entry[..at].to_owned(),
                    update_type: entry[at + 1..].to_owned(),
                },
                None => VerUpdated {
                    ver_updated: entry.to_owned(),
                    update_type: String::new(),
                },
            }
        })
        .collect();
    (!updates.is_empty()).then_some(updates)
}

#[cfg(test)]
mod tests {
    /// The dash inside `pre-` is part of a version's name, not a field separator, so a gap
    /// between two pre-release versions is still two versions.
    #[test]
    fn a_pre_release_name_is_not_split_down_the_middle() {
        let gaps = super::gaps("pre-0.098-pre-0.099").expect("one gap");
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].ver_removed, "pre-0.098");
        assert_eq!(gaps[0].ver_readded, "pre-0.099");
    }

    /// Nor is the dash before a patch number, which sits inside the version name a gap is
    /// written out of.
    #[test]
    fn a_gap_splits_on_the_dash_between_its_two_versions() {
        let gaps = super::gaps("0.120d patch 21-0.120e").expect("one gap");
        assert_eq!(gaps[0].ver_removed, "0.120d patch 21");
        assert_eq!(gaps[0].ver_readded, "0.120e");
    }

    /// An update's dash separates the version from the wiki's shorthand for what changed, which
    /// is a letter or two rather than another version.
    #[test]
    fn an_update_keeps_its_shorthand_and_a_bare_one_gets_none() {
        let updates = super::updates("0.118e,0.120a-c,pre-0.099").expect("three updates");
        assert_eq!(updates.len(), 3);
        assert_eq!(updates[0].ver_updated, "0.118e");
        assert_eq!(updates[0].update_type, "");
        assert_eq!(updates[1].ver_updated, "0.120a");
        assert_eq!(updates[1].update_type, "c");
        assert_eq!(updates[2].ver_updated, "pre-0.099");
        assert_eq!(updates[2].update_type, "");
    }

    /// A world the wiki says nothing about carries neither field rather than an empty one.
    #[test]
    fn nothing_packed_is_nothing_published() {
        assert!(super::gaps("").is_none());
        assert!(super::updates("").is_none());
        assert!(super::gaps("0.120e").is_none(), "half a gap says nothing");
    }
}
