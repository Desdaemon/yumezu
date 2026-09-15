#[cfg(target_family = "wasm")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use super::World;

// The invented tree of `yumezu_routing::fixture`, as worlds this side reads. Built rather than
// parsed, `data.json` being the wiki's to distribute and not in the tree.
fn load() -> Vec<World> {
    yumezu_routing::fixture::dream_tree()
        .into_iter()
        .enumerate()
        .map(|(id, place)| World {
            title: place.title.to_owned(),
            title_jp: None,
            author: "Yumemiru".to_owned(),
            image: String::new(),
            added: Some(format!("0.1{id:02}")),
            map_url: None,
            map_label: None,
            secret: false,
            cell: Some(id),
            unknown: false,
            connections: place.connections,
        })
        .collect()
}

fn world(title: &str, secret: bool, out: &[usize]) -> World {
    World {
        title: title.to_owned(),
        title_jp: None,
        author: String::new(),
        image: String::new(),
        added: None,
        map_url: None,
        map_label: None,
        secret,
        cell: None,
        unknown: false,
        connections: out
            .iter()
            .map(|&target_id| super::Connection {
                target_id,
                bits: 0,
                type_params: Default::default(),
            })
            .collect(),
    }
}

/// 0 Nexus - 1 Debug Room (secret) - 2 Sofa Room, each joined to both the others.
fn secretive() -> Vec<World> {
    vec![
        world("Nexus", false, &[1, 2]),
        world("Debug Room", true, &[0, 2]),
        world("Sofa Room", false, &[0, 1]),
    ]
}

// Getting the renumbering wrong is silent: the graph still draws, with lines to the wrong
// worlds.
#[test]
fn hiding_a_world_renumbers_the_connections_that_outlive_it() {
    let mut worlds = secretive();
    super::hide(&mut worlds, false);

    let far = |world: &World| -> Vec<usize> {
        world
            .connections
            .iter()
            .map(|connection| connection.target_id)
            .collect()
    };
    assert_eq!(
        worlds
            .iter()
            .map(|world| (world.title.as_str(), far(world)))
            .collect::<Vec<_>>(),
        // Sofa Room has moved down to 1, and the connection each wrote to the debug room is
        // gone rather than pointing at whoever took its place.
        [("Nexus", vec![1]), ("Sofa Room", vec![0])]
    );
}

#[test]
fn the_code_is_only_taken_once_it_has_been_typed_whole() {
    fn type_out(code: &mut super::Code, keys: &str) -> bool {
        keys.chars().for_each(|key| code.typed(key));
        code.taken()
    }
    let code = &mut super::Code::default();

    assert!(!type_out(code, "2005072"));
    assert!(!type_out(code, "nexus"));
    // A false start, then the code from the top.
    assert!(type_out(code, "220050726"));
    // Half of it, dropped by a search, so what follows completes nothing.
    code.typed('2');
    code.forget();
    assert!(!type_out(code, "0050726"));
}

#[test]
fn the_code_keeps_the_secret_worlds_and_the_ways_into_them() {
    let mut worlds = secretive();
    super::hide(&mut worlds, true);

    assert_eq!(
        worlds
            .iter()
            .map(|world| (world.title.as_str(), world.connections.len()))
            .collect::<Vec<_>>(),
        [("Nexus", 2), ("Debug Room", 2), ("Sofa Room", 2)]
    );
}

#[test]
fn a_frontier_keeps_one_step_past_what_was_visited() {
    // Each links only the step onward, so the step back is read off the far side's listing.
    let dump = chain(&["Nexus", "Sofa Room", "Far Room", "Farther Room"], 0);
    let visited = ["Nexus".to_owned()].into_iter().collect();
    let shown = dump.showing(&visited);

    assert_eq!(
        shown
            .worlds
            .iter()
            .map(|world| (world.title.as_str(), world.cell()))
            .collect::<Vec<_>>(),
        // Kept as a place rather than a world: no cell, so it wears the placeholder.
        [("Nexus", Some(0)), ("Sofa Room", None)]
    );
    // Sofa Room's step onward led to a world no longer there, so it is gone rather than
    // pointing at whoever took its place.
    assert_eq!(
        shown
            .worlds
            .iter()
            .map(|world| world
                .connections
                .iter()
                .map(|connection| connection.target_id)
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        [vec![1], vec![]]
    );
}

#[test]
fn a_frontier_does_not_reach_through_a_passage_it_cannot_be_walked_down() {
    // Every listed step is one-way *into* the world listing it, so Nexus has no way onward.
    let dump = chain(
        &["Nexus", "Sofa Room", "Far Room", "Farther Room"],
        yumezu_routing::ConnType::NO_ENTRY.bits(),
    );
    let visited = ["Nexus".to_owned()].into_iter().collect();

    assert_eq!(
        dump.showing(&visited)
            .worlds
            .iter()
            .map(|world| world.title.as_str())
            .collect::<Vec<_>>(),
        // Sofa Room is a step the player cannot take.
        ["Nexus"]
    );
}

/// Worlds in a row, each listing the step onward under `flags`.
fn chain(titles: &[&str], flags: u16) -> super::Dump {
    let worlds = titles
        .iter()
        .enumerate()
        .map(|(at, title)| World {
            title: (*title).to_owned(),
            title_jp: None,
            author: String::new(),
            image: format!("{title}.png"),
            added: None,
            map_url: None,
            map_label: None,
            secret: false,
            cell: Some(at),
            unknown: false,
            connections: (at + 1 < titles.len())
                .then(|| super::Connection {
                    target_id: at + 1,
                    bits: flags,
                    type_params: Default::default(),
                })
                .into_iter()
                .collect(),
        })
        .collect();
    super::Dump {
        worlds,
        releases: Vec::new(),
        credits: Vec::new(),
        last_update: None,
        last_full_update: None,
    }
}

#[test]
fn a_connection_reads_the_same_from_either_end() {
    let worlds = load();
    let connections = super::connections(&worlds);
    for (from, steps) in connections.iter().enumerate() {
        for step in steps {
            let far = connections[step.world]
                .iter()
                .find(|far| far.world == from)
                .expect("the world at the far end carries it too");
            assert_eq!(step.out.is_some(), far.back.is_some());
            assert_eq!(step.back.is_some(), far.out.is_some());
            assert_eq!(step.one_way(), far.one_way());
        }
    }
}

#[test]
fn a_condition_is_read_out_in_the_words_the_dump_carries() {
    // The words asserted below are the English ones.
    crate::i18n::speak_english();
    let worlds = load();
    let connections = super::connections(&worlds);
    let conditions: Vec<String> = connections
        .iter()
        .flatten()
        .filter_map(|step| step.out.as_ref())
        .map(super::sentences)
        .collect();
    let any = |wanted: &str| {
        conditions
            .iter()
            .any(|conditions| conditions.contains(wanted))
    };
    assert!(any(" chance"), "no odds are read out");
    assert!(any("in Winter"), "no season is read out");
    assert!(
        conditions
            .iter()
            .any(|conditions| conditions.starts_with("needs ") && conditions != "needs an effect"),
        "no effect is named"
    );
}

#[test]
fn an_effect_is_named_and_the_words_around_it_are_left_alone() {
    crate::i18n::speak_english();
    let named = |detail: &str| super::named_effects(&super::corrected(detail));
    assert_eq!(super::effects("Bat&comma; Fairy"), "Bat, Fairy");
    assert_eq!(named("fairy"), "Fairy");
    assert_eq!(named("Teru Teru Bozu"), "Teru Teru Bōzu");
    assert_eq!(
        named("Polygon (Mystery Zone A) or Crossing"),
        "Polygon (Mystery Zone A) or Crossing"
    );
    // A name is a whole word, and a word that merely starts with one is not it.
    assert_eq!(named("Springfield"), "Springfield");
}

#[test]
fn the_origin_roots_the_route_tree() {
    let worlds = load();
    let origin = super::origin_world(&worlds);
    assert_eq!(worlds[origin].title, yumezu_routing::START);
    let routes = super::canonical_routes(&worlds);
    assert_eq!(routes.depth[origin], Some(0));
    assert!(routes.parents[origin].is_none());
    // What a seed at the wrong world does not do: everything unreached is at no depth, and
    // the graph draws that as one layer.
    let unreached: Vec<_> = routes
        .depth
        .iter()
        .enumerate()
        .filter(|(_, depth)| depth.is_none())
        .map(|(world, _)| worlds[world].title.as_str())
        .collect();
    assert_eq!(unreached, [] as [&str; 0]);
}

#[test]
fn directions_start_where_the_walk_is_read_from() {
    let worlds = load();
    let connections = super::connections(&worlds);
    // Well away from the origin, which is the case the canonical routes never exercise.
    let from = (super::origin_world(&worlds) + worlds.len() / 2) % worlds.len();
    let routes = super::routes_from(&connections, from);
    assert_eq!(routes.depth[from], Some(0));
    assert!(routes.parents[from].is_none());
    let reached = routes.depth.iter().filter(|depth| depth.is_some()).count();
    assert!(reached > 1, "{} leads nowhere", worlds[from].title);

    for to in 0..worlds.len() {
        if routes.depth[to].is_none() {
            continue;
        }
        let mut step = to;
        while let Some(parent) = routes.parents[step] {
            let onward = connections[parent]
                .iter()
                .find(|onward| onward.world == step)
                .expect("a route steps between worlds that are connected");
            assert!(
                onward.out.is_some(),
                "{} is walked to {} the way it cannot be",
                worlds[parent].title,
                worlds[step].title
            );
            step = parent;
        }
        assert_eq!(step, from, "{} is not walked from", worlds[from].title);
    }
}

#[test]
fn a_free_way_nothing_is_shorter_than_is_offered_alone() {
    let worlds = load();
    let connections = super::connections(&worlds);
    let hub = super::hub_world(&worlds);
    let from = super::origin_world(&worlds);
    // A neighbour walked to for nothing: no second way there is as short, and none is freer.
    let to = connections[from]
        .iter()
        .find(|step| {
            step.out
                .as_ref()
                .is_some_and(|step| step.gate == super::Gate::Free)
        })
        .expect("the origin walks somewhere for nothing")
        .world;

    let ways = super::ways(&connections, from, to, 5, hub);
    assert_eq!(ways.len(), 1, "an alternative to walking straight there");
    assert_eq!(ways[0].walk, [from, to]);
}

#[test]
fn the_eyeball_bomb_is_a_way_back_from_anywhere() {
    let worlds = load();
    let connections = super::connections(&worlds);
    let hub = super::hub_world(&worlds).expect("the dump has the Nexus");
    // One that does not lead there itself, so the way back is the effect and nothing else.
    let from = connections
        .iter()
        .position(|steps| steps.iter().all(|step| step.world != hub))
        .expect("a world the Nexus is not connected to");

    let ways = super::ways(&connections, from, hub, 5, Some(hub));
    let bomb = ways
        .iter()
        .find(|way| way.walk == [from, hub])
        .expect("no way back with the bomb");
    assert_eq!((bomb.conditions, bomb.demands), (super::Gate::Effect, 1));
    assert_eq!(
        super::step_conditions(&connections, Some(hub), from, hub)
            .as_ref()
            .and_then(super::Conditions::detail),
        Some(yumezu_routing::ESCAPE)
    );
}

#[test]
fn every_way_offered_is_one_a_player_could_walk() {
    let worlds = load();
    let connections = super::connections(&worlds);
    let from = super::origin_world(&worlds);
    // Well away from the origin, so there is more than one way to be had.
    let to = (from + worlds.len() / 2) % worlds.len();
    let hub = super::hub_world(&worlds);
    let ways = super::ways(&connections, from, to, 5, hub);
    assert!(!ways.is_empty(), "no way to {}", worlds[to].title);
    assert!(ways.len() <= 5);

    for way in &ways {
        assert_eq!(
            (way.walk.first(), way.walk.last()),
            (Some(&from), Some(&to))
        );
        let mut seen = std::collections::HashSet::new();
        assert!(
            way.walk.iter().all(|&world| seen.insert(world)),
            "a way walks through the same world twice"
        );
        let (mut conditions, mut demands) = (super::Gate::Free, 0);
        for pair in way.walk.windows(2) {
            let step = super::step_conditions(&connections, hub, pair[0], pair[1])
                .expect("a way is walked the way it can be");
            conditions = conditions.max(step.gate);
            demands += u32::from(step.gate != super::Gate::Free);
        }
        assert_eq!((conditions, demands), (way.conditions, way.demands));
        let onward: Vec<_> = way
            .walk
            .windows(2)
            .map(|pair| {
                super::step_conditions(&connections, hub, pair[0], pair[1])
                    .is_some_and(|step| step.gate.onward())
            })
            .collect();
        assert!(
            onward.windows(2).all(|pair| pair[0] >= pair[1]),
            "a way walks on out of an isolated section"
        );
    }

    let traded: Vec<_> = ways
        .iter()
        .map(|way| (way.demands, way.walk.len(), way.backs_out))
        .collect();
    assert!(
        traded.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "the ways are not offered by what they demand: {traded:?}"
    );
    for &(demands, connections, backs_out) in &traded {
        let room = match backs_out {
            true => 1,
            false => demands.max(1) as usize,
        };
        assert!(
            traded
                .iter()
                .filter(|way| (way.0, way.2) == (demands, backs_out))
                .count()
                <= room,
            "a class of demand is offered more ways than it demands things: {traded:?}"
        );
        assert!(
            traded
                .iter()
                .all(|&(class, len, _)| class >= demands || len > connections),
            "a way is offered that demands more without saving a connection: {traded:?}"
        );
    }
    let walks: std::collections::HashSet<_> = ways.iter().map(|way| &way.walk).collect();
    assert_eq!(walks.len(), ways.len(), "one way is offered twice");
}

#[test]
fn a_player_who_can_pass_everything_is_never_sent_further() {
    let worlds = load();
    let connections = super::connections(&worlds);
    let from = super::origin_world(&worlds);
    let walked = super::routes_from(&connections, from);
    let open = yumezu_routing::routes_from_passing(&connections, from, super::Gate::Revisit);

    let mut nearer = 0;
    for (to, world) in worlds.iter().enumerate() {
        let Some(depth) = walked.depth[to] else {
            continue;
        };
        let open = open.depth[to].expect("a world reached is reached with nothing in the way");
        assert!(
            open <= depth,
            "{} is further away with every gate open",
            world.title
        );
        nearer += u32::from(open < depth);
    }
    assert!(nearer > 0, "no world is nearer with every gate open");
}

// The depth and the route the overlay walks are one thing seen twice.
#[test]
fn depth_is_the_length_of_the_canonical_route() {
    let worlds = load();
    let routes = super::canonical_routes(&worlds);
    for (world, depth) in routes.depth.iter().enumerate() {
        let Some(depth) = *depth else {
            assert!(routes.parents[world].is_none(), "{world} is at no depth");
            continue;
        };
        let mut steps = 0;
        let mut step = world;
        while let Some(parent) = routes.parents[step] {
            steps += 1;
            step = parent;
            assert!(steps <= depth, "{} loops", worlds[world].title);
        }
        assert_eq!(
            step,
            super::origin_world(&worlds),
            "{} walks back to {step}",
            worlds[world].title
        );
        assert_eq!(steps, depth, "{} walks {steps} steps", worlds[world].title);
    }
}

// Compared against a walk ignoring both direction and conditions.
#[test]
fn conditions_only_ever_push_a_world_deeper() {
    let worlds = load();
    let routes = super::canonical_routes(&worlds);

    let mut neighbours = vec![Vec::new(); worlds.len()];
    for (from, world) in worlds.iter().enumerate() {
        for connection in &world.connections {
            let to = connection.target_id;
            if from != to {
                neighbours[from].push(to);
                neighbours[to].push(from);
            }
        }
    }
    let origin = super::origin_world(&worlds);
    let mut shortest = vec![None; worlds.len()];
    shortest[origin] = Some(0);
    let mut queue = std::collections::VecDeque::from([origin]);
    while let Some(world) = queue.pop_front() {
        let hops = shortest[world].unwrap() + 1;
        for &next in &neighbours[world] {
            if shortest[next].is_none() {
                shortest[next] = Some(hops);
                queue.push_back(next);
            }
        }
    }

    let mut deeper = Vec::new();
    for (world, (canonical, shortest)) in routes.depth.iter().zip(&shortest).enumerate() {
        let (Some(canonical), Some(shortest)) = (canonical, shortest) else {
            continue;
        };
        assert!(
            canonical >= shortest,
            "{} is {canonical} deep but {shortest} hops away",
            worlds[world].title
        );
        if canonical > shortest {
            deeper.push(worlds[world].title.as_str());
        }
    }
    // The worlds the two conditional shortcuts stand in front of.
    assert_eq!(
        deeper,
        [
            "Static Shoreline",
            "Clockwork Dunes",
            "Chalk Observatory",
            "Hollow Carnival",
            "Glass Aviary",
            "Ember Terrace",
            "Drowned Switchboard",
            "Tin Solarium",
        ]
    );
}

#[test]
fn a_world_counts_every_world_that_comes_after_it() {
    //   0 ── 1 ── 2 ── 3
    //     └── 4
    let routes = super::Routes {
        parents: vec![None, Some(0), Some(1), Some(2), Some(0)],
        depth: vec![Some(0), Some(1), Some(2), Some(3), Some(1)],
    };
    assert_eq!(routes.descendant_counts(), [4, 2, 1, 0, 0]);
}

#[test]
fn a_title_addresses_its_own_wiki_page() {
    assert_eq!(
        super::wiki_url("Urotsuki's Room"),
        "https://yume.wiki/2kki/Urotsuki's_Room"
    );
    assert_eq!(
        super::wiki_url("Fluorescent Cité"),
        "https://yume.wiki/2kki/Fluorescent_Cit%C3%A9"
    );
}

#[test]
fn a_release_addresses_the_section_it_is_written_up_in() {
    assert_eq!(
        super::version_url("0.129d"),
        "https://yume.wiki/2kki/Version_History/0130-0126#Version_0.129d"
    );
    // The English wiki gives a patch no section of its own, so it opens on the release it was
    // applied to.
    assert_eq!(
        super::version_url("0.129c patch 27"),
        "https://yume.wiki/2kki/Version_History/0130-0126#Version_0.129c"
    );
    // The page that starts the regular run holds six releases, and the two before it hold the
    // first hundred between them.
    assert_eq!(
        super::version_url("0.100"),
        "https://yume.wiki/2kki/Version_History/0105-0100#Version_0.100"
    );
    assert_eq!(
        super::version_url("0.090"),
        "https://yume.wiki/2kki/Version_History/0099-0090#Version_0.090"
    );
    assert_eq!(
        super::version_url("0.010"),
        "https://yume.wiki/2kki/Version_History/0089-0000#Version_0.010"
    );
    // A name from the first years that the wiki never filed this way.
    assert_eq!(
        super::version_url("0.078+"),
        "https://yume.wiki/2kki/Version_History"
    );
}

#[test]
fn a_release_addresses_the_japanese_wikis_own_history() {
    assert_eq!(
        super::yume2kki_t_version_url("0.129d"),
        "https://wikiwiki.jp/yume2kki-t/%E3%82%86%E3%82%81%EF%BC%92%E3%81%A3%E3%81%8D%E6%9B%B4%E6%96%B0%E5%B1%A5%E6%AD%B4/%E9%81%8E%E5%8E%BB%E3%81%AE%E6%9B%B4%E6%96%B0%E5%86%85%E5%AE%B911#ver1294"
    );
    assert_eq!(
        super::yume2kki_t_version_url("0.129c patch 27"),
        "https://wikiwiki.jp/yume2kki-t/%E3%82%86%E3%82%81%EF%BC%92%E3%81%A3%E3%81%8D%E6%9B%B4%E6%96%B0%E5%B1%A5%E6%AD%B4/%E9%81%8E%E5%8E%BB%E3%81%AE%E6%9B%B4%E6%96%B0%E5%86%85%E5%AE%B910#ver1293p27"
    );
    // Where the dump writes a patch without the space the wiki's own names have.
    assert_eq!(
        super::yume2kki_t_version_url("0.106 patch2"),
        "https://wikiwiki.jp/yume2kki-t/%E3%82%86%E3%82%81%EF%BC%92%E3%81%A3%E3%81%8D%E6%9B%B4%E6%96%B0%E5%B1%A5%E6%AD%B4/%E9%81%8E%E5%8E%BB%E3%81%AE%E6%9B%B4%E6%96%B0%E5%86%85%E5%AE%B902#ver1060p2"
    );
    assert_eq!(
        super::yume2kki_t_version_url("0.010"),
        "https://wikiwiki.jp/yume2kki-t/%E3%82%86%E3%82%81%EF%BC%92%E3%81%A3%E3%81%8D%E6%9B%B4%E6%96%B0%E5%B1%A5%E6%AD%B4/%E9%81%8E%E5%8E%BB%E3%81%AE%E6%9B%B4%E6%96%B0%E5%86%85%E5%AE%B900#ver0100"
    );
    assert_eq!(
        super::yume2kki_t_version_url("0.078+"),
        "https://wikiwiki.jp/yume2kki-t/%E3%82%86%E3%82%81%EF%BC%92%E3%81%A3%E3%81%8D%E6%9B%B4%E6%96%B0%E5%B1%A5%E6%AD%B4"
    );
}

#[test]
fn an_author_addresses_a_tag_on_the_japanese_wiki() {
    assert_eq!(
        super::yume2kki_t_author_url("185 Go"),
        "https://wikiwiki.jp/yume2kki-t/::cmd/taglist?tag=185%20Go%E6%B0%8F"
    );
    // And where the wiki writes a name differently from the dump, its own writing of it.
    let bean = super::japanese_author("Bean");
    assert_eq!(
        super::yume2kki_t_author_url(bean),
        "https://wikiwiki.jp/yume2kki-t/::cmd/taglist?tag=bean%E6%B0%8F"
    );
    assert_eq!(
        super::yume2kki_t_author_url("かえるD"),
        "https://wikiwiki.jp/yume2kki-t/::cmd/taglist?tag=%E3%81%8B%E3%81%88%E3%82%8BD%E6%B0%8F"
    );
}

#[test]
fn a_japanese_title_addresses_the_page_the_wiki_files_it_under() {
    // A slice of the list carrying each of the four ways it writes a place: a bare name, a name
    // with its page, a map leading to several places, and one leading somewhere different per map
    // it came from.
    let pages = super::parse_pages(
        r#"{
            "urlRoot": "https://wikiwiki.jp/yume2kki-t/",
            "mapLocations": {
                "0011": "青い腕の通路",
                "0058": [
                    "昭和路地",
                    { "title": "昭和路地：バスツアー", "urlTitle": "昭和路地" }
                ],
                "0230": {
                    "0229": { "title": "製作者の部屋", "urlTitle": "うろつき邸#map0230" },
                    "else": "うろつき邸"
                }
            },
            "locationUrlTitles": { "ミニゲームA": "ミニゲーム/A" }
        }"#,
    );
    // Its own name, the ordinary case, and what an unread list leaves every name at.
    let plain = "https://wikiwiki.jp/yume2kki-t/%E6%B9%96%E4%B8%8A%E3%81%AE%E6%A9%8B";
    assert_eq!(super::page_url(&pages, "湖上の橋"), plain);
    assert_eq!(super::yume2kki_t_url("湖上の橋"), plain);
    // An area written up inside another world's page: the anchor stays an anchor.
    assert_eq!(
        super::page_url(&pages, "製作者の部屋"),
        "https://wikiwiki.jp/yume2kki-t/%E3%81%86%E3%82%8D%E3%81%A4%E3%81%8D%E9%82%B8#map0230"
    );
    // A place written up on a bigger page, and one filed under a path -- the latter is the only
    // kind the list keeps outside `mapLocations`. The slash stays a slash.
    assert_eq!(
        super::page_url(&pages, "昭和路地：バスツアー"),
        "https://wikiwiki.jp/yume2kki-t/%E6%98%AD%E5%92%8C%E8%B7%AF%E5%9C%B0"
    );
    assert_eq!(
        super::page_url(&pages, "ミニゲームA"),
        "https://wikiwiki.jp/yume2kki-t/%E3%83%9F%E3%83%8B%E3%82%B2%E3%83%BC%E3%83%A0/A"
    );
    // An area the list does not carry: the world it is named after is what has a page.
    let potato = "https://wikiwiki.jp/yume2kki-t/%E3%83%9D%E3%83%86%E5%A1%94";
    assert_eq!(super::page_url(&pages, "ポテ塔：サカナ"), potato);
    assert_eq!(super::page_url(&pages, "ポテ塔: バーガーショップ"), potato);
}

// Nothing off to the side, however near.
#[test]
fn a_subtree_is_a_world_and_everything_behind_it() {
    //   0 ── 1 ── 2 ── 3
    //     └── 4
    let routes = super::Routes {
        parents: vec![None, Some(0), Some(1), Some(2), Some(0)],
        depth: vec![Some(0), Some(1), Some(2), Some(3), Some(1)],
    };
    assert_eq!(routes.subtree(1), [1, 2, 3]);
    assert_eq!(routes.subtree(4), [4]);
    assert_eq!(routes.subtree(0), [0, 1, 4, 2, 3]);
}
// `format` reads an unknown name out as itself, so a stage whose message was renamed or never
// written would put `dump-task-worlds` on screen rather than a sentence.
#[test]
fn every_stage_the_server_can_name_is_something_this_app_can_say() {
    super::super::i18n::speak_english();
    for (task, said) in super::STAGES {
        assert_eq!(super::stage(task), Some(said), "{task} names {said}");
        assert_ne!(
            super::super::i18n::format(said, None),
            said,
            "{said} is not a message any language has"
        );
    }
    assert_eq!(
        super::stage("fetchEffectData"),
        None,
        "a stage with no words"
    );
}

// A revisit is no way in: a route walked in through one is the last thing `Gate::Revisit`
// exists for, so a world with any other way in should never be reached by one.
#[test]
fn no_route_is_walked_in_through_a_revisit_a_player_could_stand_off() {
    let worlds = load();
    let routes = super::canonical_routes(&worlds);
    let steps = super::walkable_steps(&worlds);
    let gate = |from: usize, to: usize| {
        steps[from]
            .iter()
            .find(|(next, _)| *next == to)
            .map(|(_, conditions)| conditions.gate)
    };
    // A route ending in an isolated section leaves a player where the world's other
    // connections are behind a wall, so nothing there is a way in to anywhere.
    let stranded = |world: usize| {
        let mut at = world;
        while let Some(parent) = routes.parents[at] {
            if gate(parent, at).is_some_and(|gate| !gate.onward()) {
                return true;
            }
            at = parent;
        }
        false
    };

    let walked_in: Vec<_> = routes
        .parents
        .iter()
        .enumerate()
        .filter(|(world, parent)| {
            parent.is_some_and(|parent| gate(parent, *world) == Some(super::Gate::Revisit))
        })
        .map(|(world, _)| world)
        .collect();
    // Without this the loop below has nothing to iterate and the test passes proving nothing.
    assert_eq!(
        walked_in
            .iter()
            .map(|&world| worlds[world].title.as_str())
            .collect::<Vec<_>>(),
        ["Ember Terrace"]
    );

    for world in walked_in {
        let standing: Vec<_> = (0..worlds.len())
            .filter(|&from| gate(from, world).is_some_and(|gate| gate != super::Gate::Revisit))
            .filter(|&from| routes.depth[from].is_some() && !stranded(from))
            .map(|from| worlds[from].title.as_str())
            .collect();
        assert_eq!(
            standing,
            [] as [&str; 0],
            "{} is walked in through a revisit",
            worlds[world].title
        );
    }
}

// The panel reads a step's demand off `connections`; one missing there drops it silently.
#[test]
fn every_canonical_step_is_walkable_where_the_panel_reads_it() {
    let worlds = load();
    let routes = super::canonical_routes(&worlds);
    let connections = super::connections(&worlds);
    for (world, parent) in routes.parents.iter().enumerate() {
        let Some(parent) = *parent else { continue };
        let step = connections[parent]
            .iter()
            .find(|step| step.world == world)
            .unwrap_or_else(|| panic!("{} is joined to no parent", worlds[world].title));
        assert!(
            step.out.is_some(),
            "{} is walked in from {} and no way there",
            worlds[world].title,
            worlds[parent].title
        );
    }
}
