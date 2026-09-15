//! Everything this app fetches from the dump's server, and the addresses those requests go to.
//! See [`load`] and [`building`].

use super::*;

/// This project's own `dreamweaver`, deployed. Fetched rather than compiled in, so a build is not a
/// snapshot of the wiki -- worlds arrive weekly.
///
/// The reference explorer at `explorer.yume.wiki` answers the same two routes but is no fallback:
/// it publishes no `cell`, so every world would wear the placeholder.
///
/// A local `dreamweaver` is reached through the page instead -- `just serve` proxies to it, see
/// `Trunk.toml`. Only trunk serves `static/`, so a native build has no local host to fetch from.
#[cfg(not(target_family = "wasm"))]
const SERVER: &str = "https://explorer.yumemiru.dev";

/// The prefix the page rewrites out of every picture address, fetching from its own host instead.
///
/// A page cannot fetch from the wiki directly: the edge answers a cross-origin request with a
/// challenge page, and the browser sets `Origin` itself rather than honouring the header
/// `detail::ORIGIN` carries. The page's own host is same-origin and proxies on to the wiki.
#[cfg(target_family = "wasm")]
const WIKI_IMAGES: &str = "https://yume.wiki/images/";

/// Whole rather than the bare path the host sees, because these addresses reach the network
/// through `reqwest` rather than the document, which has no page to resolve a bare path against.
#[cfg(target_family = "wasm")]
fn proxied_images() -> &'static str {
    static PROXIED_IMAGES: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PROXIED_IMAGES.get_or_init(|| format!("{}/img/", origin()))
}

/// The only host the page may fetch anything from unbidden.
#[cfg(target_family = "wasm")]
pub(crate) fn origin() -> &'static str {
    static ORIGIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ORIGIN.get_or_init(|| {
        web_sys::window()
            .expect("the page has no window")
            .location()
            .origin()
            .expect("the page has no origin to fetch anything through")
    })
}

/// The host this run fetches from, which the page reads off itself.
///
/// A request straight at the server is cross-origin, which `dreamweaver` sends no
/// `Access-Control-Allow-Origin` to allow, and mixed-content over https. So the page fetches from
/// its own host under the same routes -- see the proxies in `Trunk.toml`.
///
/// Also where [`super::super::thumbnails`] reads the atlas from, which is why this is not private.
pub(crate) fn server() -> &'static str {
    #[cfg(not(target_family = "wasm"))]
    return SERVER;
    #[cfg(target_family = "wasm")]
    return origin();
}

fn url() -> String {
    format!("{}/data", server())
}

/// What the server says it is building, as the message to say about it.
///
/// A server rebuilding the dump says so rather than serving one it is about to replace, so the wait
/// can be a minute. `GET /pollUpdate` is the reference route, which `dreamweaver` also answers.
///
/// `None` for anything this app has no words for -- a server between syncs, a host with no such
/// route, the finer stages only the reference server names -- which all mean the plain wait.
pub async fn building() -> Option<&'static str> {
    let url = format!("{}/pollUpdate", server());
    let said = match download(&url).await {
        Ok(said) => said,
        Err(error) => {
            // Not a warning: most hosts have nothing to say about what they are building.
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

/// `Ok(None)` is the server saying it is building one, which is a wait rather than a failure --
/// the request is also what starts that build. [`building`] is what to say meanwhile.
///
/// `Err` carries what to say on screen rather than panicking: a document off the network failing
/// to resolve, or resolving into something else, is not worth taking the window down for.
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

/// `None` for a server that has no dump to send yet. Its own request rather than [`download`], the
/// dump being the one document with an answer that is neither itself nor a failure.
async fn dump(url: &str) -> Result<Option<String>, fetch::Error> {
    let response = fetch::client().get(url).send().await?;
    // The server is rebuilding. See `dreamweaver`'s `data`.
    if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        return Ok(None);
    }
    Ok(Some(response.error_for_status()?.text().await?))
}

/// For the documents that only ever answer with themselves.
pub(super) async fn download(url: &str) -> Result<String, fetch::Error> {
    Ok(fetch::client()
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// Rewrites the addresses this platform cannot use as they stand.
fn parse(json: &str, revealed: bool) -> serde_json::Result<Dump> {
    let mut dump = serde_json::from_str::<Dump>(json)?;
    hide(&mut dump.worlds, revealed);
    // Every picture address the app fetches at runtime passes through here and only here.
    // `tools/atlas` reads `data.json` itself and rightly misses this, running at build time.
    #[cfg(target_family = "wasm")]
    for world in &mut dump.worlds {
        world.image = world.image.replace(WIKI_IMAGES, proxied_images());
        if let Some(urls) = &mut world.map_url {
            // Whole rather than entry by entry: every address carries the same prefix.
            *urls = urls.replace(WIKI_IMAGES, proxied_images());
        }
    }
    Ok(dump)
}
