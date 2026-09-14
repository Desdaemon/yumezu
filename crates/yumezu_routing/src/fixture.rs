//! An invented world tree for the tests of both programs that read a dump.
//!
//! The wiki's own `data.json` is the wiki's to distribute and is not in the tree. This is here
//! rather than in either program because the two hold their worlds in different types and need the
//! same graph under them: each maps this into its own.
//!
//! Each of these is load-bearing for a test, so an edit here costs one there: a free route and a
//! shorter conditional one to the same world, a world reachable only through a condition naming
//! itself, a connection only one side lists, and a world the hub does not touch.
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
//!     8  Clockwork Dunes      -> 10 unlocking its far side, 14 behind a condition
//!     9  Violet Aquifer       -> nothing
//!     10 Chalk Observatory    -> 8 locked, 11 one way
//!     11 Hollow Carnival      -> 12, 14
//!     12 Glass Aviary         -> 13, behind a condition naming 13
//!     13 Ember Terrace        -> nothing
//!     14 Drowned Switchboard  -> 15
//!     15 Tin Solarium         -> nothing
//! ```

use crate::{ConnType, Connection, TypeParams};

/// A world of the tree: the wiki's English title, and what it lists as its own connections.
pub struct Place {
    pub title: &'static str,
    pub connections: Vec<Connection>,
}

fn conn(to: usize, flags: ConnType, worded: Option<&str>) -> Connection {
    Connection {
        target_id: to,
        flags: flags.bits(),
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

fn free(to: usize) -> Connection {
    conn(to, ConnType::empty(), None)
}

fn cond(to: usize, words: &str) -> Connection {
    conn(to, ConnType::LOCKED_CONDITION, Some(words))
}

pub fn dream_tree() -> Vec<Place> {
    [
        ("Urotsuki's Room", vec![free(1), free(2), free(3), cond(12, "having seen the ledger")]),
        ("Lantern Causeway", vec![free(4)]),
        ("Mural Vestibule", vec![conn(4, ConnType::CHANCE, Some("1/8"))]),
        ("Quiet Ledger", vec![conn(4, ConnType::SEASONAL, Some("Winter"))]),
        ("Nexus", vec![free(5), conn(6, ConnType::EFFECT, Some("Lantern Cloak"))]),
        ("Sunken Orchard", vec![free(7)]),
        ("Static Shoreline", vec![cond(7, "having lit the beacon")]),
        ("Marzipan Road", vec![free(8), conn(9, ConnType::DEAD_END, None)]),
        ("Clockwork Dunes", vec![conn(10, ConnType::UNLOCK, None), cond(14, "having wound the drum")]),
        ("Violet Aquifer", vec![]),
        ("Chalk Observatory", vec![conn(8, ConnType::LOCKED, None), conn(11, ConnType::ONE_WAY, None)]),
        ("Hollow Carnival", vec![free(12), free(14)]),
        // The only way in to Ember Terrace, and its words name Ember Terrace: a condition only a
        // player already there could meet, which is what `Gate::Revisit` is.
        ("Glass Aviary", vec![cond(13, "having been to Ember Terrace")]),
        ("Ember Terrace", vec![]),
        ("Drowned Switchboard", vec![free(15)]),
        ("Tin Solarium", vec![]),
    ]
    .into_iter()
    .map(|(title, connections)| Place { title, connections })
    .collect()
}
