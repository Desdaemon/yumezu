//! Everything this app fetches from the dump's server, and the addresses those requests go to.
//! See [`load`] and [`building`].

use super::*;

/// This project's deployed `dreamweaver`, fetched at runtime since worlds arrive weekly.
/// `explorer.yume.wiki` serves the same routes without `cell`, leaving every world a placeholder.
#[cfg(not(target_family = "wasm"))]
const SERVER: &str = "https://explorer.yumemiru.dev";

/// The prefix the web build rewrites out of picture addresses, to fetch through the page's origin.
/// The wiki's edge challenges cross-origin requests; the browser will not send `lod::ORIGIN`.
#[cfg(target_family = "wasm")]
const WIKI_IMAGES: &str = "https://yume.wiki/images/";

/// An absolute URL: `reqwest` has no document to resolve a bare path against.
#[cfg(target_family = "wasm")]
fn proxied_images() -> &'static str {
    static PROXIED_IMAGES: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PROXIED_IMAGES.get_or_init(|| format!("{}/img/", web_origin()))
}

/// The only host the page may fetch anything from unbidden.
#[cfg(target_family = "wasm")]
pub(crate) fn web_origin() -> &'static str {
    static ORIGIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ORIGIN.get_or_init(|| {
        web_sys::window()
            .expect("the page has no window")
            .location()
            .origin()
            .expect("the page has no origin to fetch anything through")
    })
}

/// The host the app fetches from; the web build uses the page's origin, proxied in `Trunk.toml`, as
/// `dreamweaver` sends no `Access-Control-Allow-Origin` and https forbids mixed content.
pub(crate) fn server() -> &'static str {
    #[cfg(not(target_family = "wasm"))]
    return SERVER;
    #[cfg(target_family = "wasm")]
    return web_origin();
}

fn url() -> String {
    format!("{}/data", server())
}

/// The message for the stage of a dump rebuild, which can take a minute, from `GET /pollUpdate`.
/// `None` without a message: no rebuild under way, no such route, or a stage this app cannot name.
pub async fn building() -> Option<&'static str> {
    let url = format!("{}/pollUpdate", server());
    let said = match fetch::client().get(&url).fetch_text().await {
        Ok(said) => said,
        Err(error) => {
            // Most hosts have no build status to report.
            log::debug!("cannot reach {url}: {error}");
            return None;
        }
    };
    stage(
        serde_json::from_str::<serde_json::Value>(&said)
            .ok()?
            .get("task")?
            .as_str()?,
    )
}

pub(super) fn stage(task: &str) -> Option<&'static str> {
    STAGES
        .iter()
        .find(|(named, _)| *named == task)
        .map(|(_, said)| *said)
}

/// The names on the left come from `dreamweaver`'s `progress`.
pub(super) const STAGES: [(&str, &str); 4] = [
    ("init", "dump-task-changes"),
    ("fetchWorldData", "dump-task-worlds"),
    ("fetchConnData", "dump-task-connections"),
    ("prepareWorldData", "dump-task-assembling"),
];

/// `Ok(None)` while the server builds the dump, which this request starts; see [`building`].
/// `Err` carries the message to show when the dump cannot be fetched or parsed.
pub async fn load(revealed: bool) -> Result<Option<Dump>, String> {
    let url = url();
    let Some(json) = dump(&url)
        .await
        .map_err(|error| format!("cannot reach {url}: {error}"))?
    else {
        return Ok(None);
    };
    parse(&json, revealed)
        .map(Some)
        .map_err(|error| format!("{url} is not the expected world dump: {error}"))
}

/// `None` for a server with no dump yet. Sent directly, as [`fetch::FetchExt`] fails on a 503.
async fn dump(url: &str) -> Result<Option<String>, fetch::Error> {
    let response = fetch::client().get(url).send().await?;
    // The server is rebuilding.
    if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        return Ok(None);
    }
    Ok(Some(response.error_for_status()?.text().await?))
}

/// Rewrites the addresses this platform cannot use as they stand.
fn parse(json: &str, revealed: bool) -> serde_json::Result<Dump> {
    let mut dump = serde_json::from_str::<Dump>(json)?;
    hide(&mut dump.worlds, revealed);
    #[cfg(target_family = "wasm")]
    for world in &mut dump.worlds {
        world.image = world.image.replace(WIKI_IMAGES, proxied_images());
        if let Some(urls) = &mut world.map_url {
            *urls = urls.replace(WIKI_IMAGES, proxied_images());
        }
    }
    Ok(dump)
}
