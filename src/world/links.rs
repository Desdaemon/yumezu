//! Where a world, an author or a release is written up, on either wiki. The dump carries no page
//! addresses, so every one of these is built from a name the way the wiki that holds it would.

use super::*;

/// Authors whose yume2kki-t tag is not their name in the dump.
///
/// TODO: corrections that belong on yume.wiki rather than here.
static JAPANESE_AUTHOR_OVERRIDES: [(&str, &str); 10] = [
    ("Bean", "bean"),
    ("窯良", "窯良(oneirokamara)"),
    ("コンテンツ", "kontentsu"),
    ("Ouri", "ouri"),
    ("sniperbob", "Sniperbob"),
    ("Mokaccino", "Moka"),
    ("◆gH8PoF17WqX", "Ferdy"),
    ("Nightmare", "†Nightmare†"),
    ("tKp9vEGEfhCD", "◆tKp9vEGEfhCD"),
    ("Nulsdodage", "nulsdodage"),
];

pub(super) fn japanese_author(name: &str) -> &str {
    JAPANESE_AUTHOR_OVERRIDES
        .iter()
        .find_map(|&(from, to)| (from == name).then_some(to))
        .unwrap_or(name)
}

fn append_encoded(input: &str, output: &mut String) {
    for byte in input.bytes() {
        match byte {
            b' ' => output.push('_'),
            byte if byte.is_ascii_alphanumeric() => output.push(byte as char),
            b'-' | b'_' | b'.' | b'\'' | b'(' | b')' | b',' | b'!' | b'/' => {
                output.push(byte as char)
            }
            byte => output.push_str(&format!("%{byte:02X}")),
        }
    }
}

/// The dump carries no page address, only image ones, so this is built from the title the way the
/// wiki builds it.
pub fn wiki_url(title: &str) -> String {
    let mut url = String::from("https://yume.wiki/2kki/");
    append_encoded(title, &mut url);
    url
}

/// A different wiki with pages of its own rather than a translation of the English one.
const YUME2KKI_T: &str = "https://wikiwiki.jp/yume2kki-t/";

/// YNOproject's list of what that wiki calls each place, which is what the game's own client
/// addresses it by.
const YNOLOCATIONS: &str =
    "https://raw.githubusercontent.com/ynoproject/ynolocations/refs/heads/master/2kki/ja.json";

/// The few dozen worlds, out of fifteen hundred, whose Japanese page is not named after them: an
/// area written up inside another world's page, or a name filed under a longer path.
type Pages = std::collections::HashMap<String, String>;

/// Empty until [`load_pages`] has answered.
static PAGES: std::sync::OnceLock<Pages> = std::sync::OnceLock::new();

/// Started beside the dump rather than on the first Japanese link: a window opened after the click
/// has passed is a popup the browser blocks. A link clicked in the first moment of a run is
/// addressed without the list, which is right for all but the few dozen worlds in it -- hence also
/// the warning on failure.
pub async fn load_pages() {
    let pages = match download(YNOLOCATIONS).await {
        Ok(json) => parse_pages(&json),
        Err(error) => {
            log::warn!("cannot reach {YNOLOCATIONS}: {error}");
            return;
        }
    };
    log::info!(
        "{} japanese pages are named after something else",
        pages.len()
    );
    let _ = PAGES.set(pages);
}

/// The list names places by map rather than by world, nested several ways -- one place, several,
/// or a different one per map it leads on from. None of that matters, so it is walked rather than
/// modelled.
pub(super) fn parse_pages(json: &str) -> Pages {
    let mut pages = Pages::new();
    let Ok(list) = serde_json::from_str::<serde_json::Value>(json) else {
        log::warn!("{YNOLOCATIONS} is not JSON");
        return pages;
    };
    // Whole-name overrides first, so a per-map one wins where the list gives both.
    if let Some(titles) = list["locationUrlTitles"].as_object() {
        for (title, page) in titles {
            if let Some(page) = page.as_str() {
                pages.insert(title.clone(), page.to_owned());
            }
        }
    }
    collect_url_titles(&list["mapLocations"], &mut pages);
    pages
}

/// Every `title`/`urlTitle` pair anywhere under `value`.
fn collect_url_titles(value: &serde_json::Value, pages: &mut Pages) {
    match value {
        serde_json::Value::Object(fields) => match (fields.get("title"), fields.get("urlTitle")) {
            (Some(serde_json::Value::String(title)), Some(serde_json::Value::String(page))) => {
                pages.insert(title.clone(), page.clone());
            }
            _ => {
                for nested in fields.values() {
                    collect_url_titles(nested, pages);
                }
            }
        },
        serde_json::Value::Array(entries) => {
            for nested in entries {
                collect_url_titles(nested, pages);
            }
        }
        _ => {}
    }
}

/// The page an unlisted name is written up on: a world names one of its areas after itself and the
/// area, and only the world has a page. The colon is where the wiki's own `Locationbox` cuts.
fn area_page(title: &str) -> &str {
    title.split([':', '：']).next().unwrap_or(title)
}

pub fn yume2kki_t_url(title: &str) -> String {
    page_url(PAGES.get().unwrap_or(&Pages::new()), title)
}

/// An override may name an anchor within a page as well as the page, and that `#` has to stay one.
pub(super) fn page_url(pages: &Pages, title: &str) -> String {
    let page = pages
        .get(title)
        .map_or_else(|| area_page(title), String::as_str);
    let (page, anchor) = match page.split_once('#') {
        Some((page, anchor)) => (page, Some(anchor)),
        None => (page, None),
    };
    let mut url = String::from(YUME2KKI_T);
    append_encoded(page, &mut url);
    if let Some(anchor) = anchor {
        url.push('#');
        append_encoded(anchor, &mut url);
    }
    url
}

/// The English wiki files one person's work as a category.
pub fn author_url(author: &str) -> String {
    let mut url = String::from("https://yume.wiki/Category:");
    append_encoded(author, &mut url);
    url
}

/// The Japanese wiki tags a world's page with its author's name rather than giving each author a
/// page, so what there is to open is the search for the tag.
pub fn yume2kki_t_author_url(author: &str) -> String {
    let mut url = format!("{YUME2KKI_T}::cmd/taglist?tag=");
    // The wiki tags with the name and the honorific together. A query rather than a path, so a
    // space stays a space rather than the underscore a page name would want.
    append_query_encoded(author, &mut url);
    append_query_encoded("氏", &mut url);
    url
}

/// A release name as both wikis break it down: `0.129c patch 27` is release 129, revision `c`,
/// patch 27. `None` for the handful of names from the game's first years written some other way.
struct ReleaseName {
    number: u32,
    /// 0 for a release with no letter, 1 for `a`.
    revision: u32,
    patch: Option<u32>,
}

fn read_release(name: &str) -> Option<ReleaseName> {
    let rest = name.strip_prefix("0.")?;
    let (number, rest) = rest.split_at_checked(3)?;
    let number = number.parse().ok()?;
    let mut chars = rest.chars();
    let (revision, rest) = match chars.next() {
        Some(letter @ 'a'..='z') => (letter as u32 - 'a' as u32 + 1, chars.as_str()),
        _ => (0, rest),
    };
    let patch = match rest.trim_start().strip_prefix("patch") {
        Some(patch) => Some(patch.trim_start().parse().ok()?),
        None if rest.is_empty() => None,
        None => return None,
    };
    Some(ReleaseName {
        number,
        revision,
        patch,
    })
}

const VERSION_HISTORY: &str = "https://yume.wiki/2kki/Version_History";

/// The English wiki files five releases to a page, named for the range it holds and counting down.
/// Its patches share their release's section, so a patch opens where it was applied.
pub fn version_url(name: &str) -> String {
    let Some(release) = read_release(name) else {
        return VERSION_HISTORY.to_owned();
    };
    // The first hundred releases are two pages of their own, and the page that starts the regular
    // run holds six rather than five.
    let (high, low) = match release.number {
        0..=89 => (89, 0),
        90..=99 => (99, 90),
        100..=105 => (105, 100),
        number => {
            let high = number.div_ceil(5) * 5;
            (high, high - 4)
        }
    };
    let revision = match release.revision {
        0 => String::new(),
        revision => char::from(b'a' + revision as u8 - 1).to_string(),
    };
    format!(
        "{VERSION_HISTORY}/{high:04}-{low:04}#Version_0.{number:03}{revision}",
        number = release.number
    )
}

/// The Japanese wiki's own history page, which is the ten most recent releases and nothing else.
const UPDATE_HISTORY: &str = "ゆめ２っき更新履歴";

/// Where each of that wiki's past-updates pages starts, highest first, by the code its anchors are
/// numbered with. A page is cut when it grows too long rather than at a round release, so there is
/// nothing to compute these from.
const PAST_UPDATES: [(u32, &str); 12] = [
    (1294, "11"),
    (1280, "10"),
    (1265, "09"),
    (1244, "08"),
    (1226, "07"),
    (1204, "06"),
    (1184, "05"),
    (1140, "04"),
    (1080, "03"),
    (1007, "02"),
    (861, "01"),
    (0, "00"),
];

/// The Japanese wiki anchors a release by number and revision run together -- `0.129d` is `ver1294`
/// -- and gives most patches an anchor of their own. A patch it does not name lands at the top of
/// the right page.
pub fn yume2kki_t_version_url(name: &str) -> String {
    let mut url = String::from(YUME2KKI_T);
    let Some(release) = read_release(name) else {
        append_encoded(UPDATE_HISTORY, &mut url);
        return url;
    };
    let code = release.number * 10 + release.revision;
    let page = PAST_UPDATES
        .iter()
        .find(|&&(start, _)| code >= start)
        .map_or("", |&(_, page)| page);
    append_encoded(&format!("{UPDATE_HISTORY}/過去の更新内容{page}"), &mut url);
    url.push_str(&format!("#ver{code:04}"));
    if let Some(patch) = release.patch {
        url.push_str(&format!("p{patch}"));
    }
    url
}

fn append_query_encoded(input: &str, output: &mut String) {
    for byte in input.bytes() {
        match byte {
            byte if byte.is_ascii_alphanumeric() => output.push(byte as char),
            b'-' | b'_' | b'.' | b'~' => output.push(byte as char),
            byte => output.push_str(&format!("%{byte:02X}")),
        }
    }
}
