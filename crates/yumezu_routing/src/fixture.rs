//! An invented world tree for both programs' tests; the wiki's `data.json` is not ours to ship.
//! Every feature in the table is load-bearing for a test.
//!
//! ```text
//!     0  Urotsuki's Room      -> 1, 2, 3, and 12 behind a condition
//!     1  Lantern Causeway     -> 4
//!     2  Mural Vestibule      -> 4 on a 1/8 chance
//!     3  Quiet Ledger         -> 4 in Winter
//!     4  Nexus                -> 5, and 6 wearing the Lantern Cloak
//!     5  Sunken Orchard       -> 7
//!     6  Static Shoreline     -> 7 behind a condition
//!     7  Marzipan Road        -> 8, and 9 as a dead end
//!     8  Clockwork Dunes      -> 10 unlocking the far side, 14 behind a condition
//!     9  Violet Aquifer       -> nothing
//!     10 Chalk Observatory    -> 8 locked, 11 one way
//!     11 Hollow Carnival      -> 12, 14
//!     12 Glass Aviary         -> 13, behind a condition naming 13
//!     13 Ember Terrace        -> nothing
//!     14 Drowned Switchboard  -> 15
//!     15 Tin Solarium         -> nothing
//! ```

use crate::{ConnType, Connection, TypeParams, World};

/// A world of the tree: the wiki's English title, and the connections the world lists.
pub struct Place {
    pub title: &'static str,
    pub connections: Vec<Connection>,
}

impl World for Place {
    fn title(&self) -> &str {
        self.title
    }
    fn connections(&self) -> &[Connection] {
        &self.connections
    }
}

pub fn connection(to: usize, flags: ConnType, worded: Option<&str>) -> Connection {
    Connection {
        target_id: to,
        bits: flags.bits(),
        type_params: worded
            .map(|words| {
                (
                    flags.bits(),
                    TypeParams {
                        params: Some(words.to_owned()),
                        params_jp: None,
                    },
                )
            })
            .into_iter()
            .collect(),
    }
}

/// A line of worlds, each listing the next, every connection alike.
pub fn chain(titles: &[&'static str], flags: ConnType) -> Vec<Place> {
    titles
        .iter()
        .enumerate()
        .map(|(at, &title)| Place {
            title,
            connections: (at + 1 < titles.len())
                .then(|| connection(at + 1, flags, None))
                .into_iter()
                .collect(),
        })
        .collect()
}

fn free(to: usize) -> Connection {
    connection(to, ConnType::empty(), None)
}

fn cond(to: usize, words: &str) -> Connection {
    connection(to, ConnType::LOCKED_CONDITION, Some(words))
}

pub fn dream_tree() -> Vec<Place> {
    [
        (
            "Urotsuki's Room",
            vec![
                free(1),
                free(2),
                free(3),
                cond(12, "having seen the ledger"),
            ],
        ),
        ("Lantern Causeway", vec![free(4)]),
        (
            "Mural Vestibule",
            vec![connection(4, ConnType::CHANCE, Some("1/8"))],
        ),
        (
            "Quiet Ledger",
            vec![connection(4, ConnType::SEASONAL, Some("Winter"))],
        ),
        (
            "Nexus",
            vec![
                free(5),
                connection(6, ConnType::EFFECT, Some("Lantern Cloak")),
            ],
        ),
        ("Sunken Orchard", vec![free(7)]),
        ("Static Shoreline", vec![cond(7, "having lit the beacon")]),
        (
            "Marzipan Road",
            vec![free(8), connection(9, ConnType::DEAD_END, None)],
        ),
        (
            "Clockwork Dunes",
            vec![
                connection(10, ConnType::UNLOCK, None),
                cond(14, "having wound the drum"),
            ],
        ),
        ("Violet Aquifer", vec![]),
        (
            "Chalk Observatory",
            vec![
                connection(8, ConnType::LOCKED, None),
                connection(11, ConnType::ONE_WAY, None),
            ],
        ),
        ("Hollow Carnival", vec![free(12), free(14)]),
        // Met only by a player already in Ember Terrace: `Gate::Revisit`.
        (
            "Glass Aviary",
            vec![cond(13, "having been to Ember Terrace")],
        ),
        ("Ember Terrace", vec![]),
        ("Drowned Switchboard", vec![free(15)]),
        ("Tin Solarium", vec![]),
    ]
    .into_iter()
    .map(|(title, connections)| Place { title, connections })
    .collect()
}
