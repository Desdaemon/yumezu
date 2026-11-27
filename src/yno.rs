//! The player's YNOproject account and visited worlds, with ids named via `/api/gamelocations`.
//! The session is a cookie only; the same value sent as a header reads as signed out.

use super::fetch;
use super::prelude::*;
use super::store;
use std::collections::HashSet;

/// YNOproject serves one host per game, and this app draws exactly one: Yume 2kki.
#[cfg(not(target_family = "wasm"))]
const API: &str = "https://api.ynoproject.net/2kki/api";
/// Sign-in is a separate service from [`API`].
#[cfg(not(target_family = "wasm"))]
const AUTH: &str = "https://auth.ynoproject.net";

/// Relayed by the web build's own host: YNOproject allows exactly one origin, and only a
/// same-origin cookie is kept. The host sees each relayed session; see `dreamweaver`'s `relay`.
#[cfg(target_family = "wasm")]
const API_PATH: &str = "/yno";
/// A separate path, since sign-in is a separate host. See [`API_PATH`].
#[cfg(target_family = "wasm")]
const AUTH_PATH: &str = "/ynoauth";

const SESSION: &str = "yno-session";
/// Native build only: on the web build the browser carries the cookie.
#[cfg(not(target_family = "wasm"))]
const COOKIE: &str = "auth";
const FRONTIER: &str = "frontier";

/// World names as the wiki writes them, the only key YNOproject and the dump share.
pub type Visited = HashSet<String>;

/// Progress of a sign-in or resumption, shown in the settings tab.
pub enum State<'a> {
    /// Nobody has signed in since the app started, and no session was kept from before.
    SignedOut,
    /// Signing in, or fetching the visited worlds.
    Working,
    SignedIn,
    /// Why the last attempt failed, in the server's words where given.
    Failed(&'a str),
}

/// Held by the app, since the graph is built from the visited worlds: a sign-in rebuilds the graph
/// as the dump's arrival does.
#[derive(Default)]
pub struct Account {
    /// Persisted, so the user signs in once.
    session: Option<String>,
    visited: Option<Visited>,
    /// Session and visited worlds, from either a sign-in or a resumption.
    pending: Option<fetch::Pending<Result<(String, Visited), String>>>,
    /// Why the last attempt failed, until the next one starts.
    failed: Option<String>,
    /// Separate from [`Account::visited`], so turning the frontier off keeps the user signed in,
    /// and turning the frontier on is instant once the visits are fetched.
    frontier: bool,
    restated: bool,
    force_reveal_worlds: Visited,
}

impl Account {
    /// Resumes a kept session at startup, so the frontier switch responds at once.
    pub fn new() -> Self {
        let session = store::read(SESSION).filter(|session| !session.is_empty());
        let pending = session.clone().map(|session| fetch::spawn(resume(session)));
        Self {
            session,
            pending,
            frontier: store::read_flag(FRONTIER),
            ..Default::default()
        }
    }

    /// Whether a request is pending, which keeps the window redrawing.
    pub fn working(&self) -> bool {
        self.pending.is_some()
    }

    /// Called once a frame; takes the answer once one arrives.
    pub fn poll(&mut self) {
        let Some(answer) = self.pending.as_ref().and_then(fetch::Pending::take) else {
            return;
        };
        self.pending = None;
        // An unchanged refresh keeps the layout; an answer always replaces a pretence.
        let mut changed = !self.force_reveal_worlds.is_empty();
        self.force_reveal_worlds.clear();
        match answer {
            Ok((session, visited)) => {
                log::info!("signed in to ynoproject, {} locations seen", visited.len());
                changed |= self.visited.as_ref() != Some(&visited);
                store::write(SESSION, Some(&session));
                self.session = Some(session);
                self.visited = Some(visited);
                self.failed = None;
            }
            Err(error) => {
                log::warn!("cannot read what this account has seen: {error}");
                // Usually an expired session, which would fail again next time the app starts.
                self.drop_session();
                self.failed = Some(error);
            }
        }
        // A failure may have taken a frontier away; with the switch off there is none.
        self.restated |= self.frontier && changed;
    }

    fn drop_session(&mut self) {
        store::write(SESSION, None);
        self.session = None;
        self.visited = None;
    }

    /// Also fetches the visited worlds, the only use of the session.
    pub fn sign_in(&mut self, user: String, password: String) {
        self.failed = None;
        self.pending = Some(fetch::spawn(sign_in(user, password)));
    }

    /// Drops only this app's copy of the session: YNOproject's sign-out ends the user's game too.
    pub fn sign_out(&mut self) {
        forget();
        self.drop_session();
        self.pending = None;
        self.failed = None;
        self.force_reveal_worlds.clear();
        // See [`Account::poll`].
        self.restated |= self.frontier;
    }

    /// Refetches the visited worlds, which change as the user plays in another window. Does
    /// nothing without a session, or while a request is pending.
    pub fn refresh(&mut self) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if self.pending.is_some() {
            return;
        }
        self.failed = None;
        self.pending = Some(fetch::spawn(resume(session)));
    }

    /// Every world the account has visited, whether or not the frontier switch is on; see
    /// [`Account::frontier`].
    pub fn visited(&self) -> Option<&Visited> {
        self.visited.as_ref()
    }

    /// Whether the frontier switch is on, even before any visits are fetched.
    pub fn frontier_wanted(&self) -> bool {
        self.frontier
    }

    pub fn set_frontier(&mut self, frontier: bool) {
        if frontier == self.frontier {
            return;
        }
        self.frontier = frontier;
        store::write_flag(FRONTIER, frontier);
        self.restated |= self.visited.is_some();
    }

    /// Marks a world visited locally, so the frontier opens past that world, until the next answer
    /// or sign-out. Kept out of [`Account::visited`], which the reported completion reads.
    pub fn force_reveal(&mut self, title: String) {
        let already_visited = self
            .visited
            .as_ref()
            .is_some_and(|visited| visited.contains(&title));
        if already_visited || !self.force_reveal_worlds.insert(title.clone()) {
            return;
        }
        log::info!("force-revealing {title}");
        self.restated |= self.frontier && self.visited.is_some();
    }

    /// `None` when the graph shows the whole game: the switch off, or no visits fetched. Owned only
    /// when worlds are force-revealed.
    pub fn frontier(&self) -> Option<std::borrow::Cow<'_, Visited>> {
        let visited = self.visited.as_ref().filter(|_| self.frontier)?;
        Some(match self.force_reveal_worlds.is_empty() {
            true => std::borrow::Cow::Borrowed(visited),
            false => {
                std::borrow::Cow::Owned(visited.union(&self.force_reveal_worlds).cloned().collect())
            }
        })
    }

    /// Whether the frontier changed since the last call; the call clears the flag.
    pub fn restated(&mut self) -> bool {
        std::mem::take(&mut self.restated)
    }

    /// Whether the frontier is known: with the switch off, or once the pending request ends, even
    /// in failure. Keeps the graph from being built on the whole dump and then discarded.
    pub fn settled(&self) -> bool {
        !self.frontier || self.pending.is_none()
    }

    pub fn state(&self) -> State<'_> {
        if self.pending.is_some() {
            return State::Working;
        }
        match (&self.visited, &self.failed) {
            (Some(_), _) => State::SignedIn,
            (None, Some(failed)) => State::Failed(failed),
            (None, None) => State::SignedOut,
        }
    }

    /// Decides whether the tab shows the sign-in fields or the sign-out button.
    pub fn signed_in(&self) -> bool {
        self.session.is_some()
    }
}

/// Absolute, with the web build's origin written in: `reqwest` has no document to resolve a bare
/// path against.
fn api() -> &'static str {
    #[cfg(not(target_family = "wasm"))]
    return API;
    #[cfg(target_family = "wasm")]
    {
        static API: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        API.get_or_init(|| format!("{}{API_PATH}", super::world::web_origin()))
    }
}

fn auth() -> &'static str {
    #[cfg(not(target_family = "wasm"))]
    return AUTH;
    #[cfg(target_family = "wasm")]
    {
        static AUTH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        AUTH.get_or_init(|| format!("{}{AUTH_PATH}", super::world::web_origin()))
    }
}

/// On the web build only the host that set the session cookie can drop the cookie. Not awaited:
/// the sign-out is already done locally.
#[cfg(target_family = "wasm")]
fn forget() {
    let url = format!("{}/forget", auth());
    drop(fetch::spawn(async move {
        if let Err(error) = fetch::client().post(&url).send().await {
            log::warn!("cannot reach {url}: {error}");
        }
    }));
}

/// No-op: the native build keeps the session itself.
#[cfg(not(target_family = "wasm"))]
fn forget() {}

async fn sign_in(user: String, password: String) -> Result<(String, Visited), String> {
    let session = login(&user, &password).await?;
    let visited = visited(&session).await?;
    Ok((session, visited))
}

/// Returns the session with the visits, as a sign-in does.
async fn resume(session: String) -> Result<(String, Visited), String> {
    let visited = visited(&session).await?;
    Ok((session, visited))
}

/// The service answers by setting a cookie. The native build takes the cookie off the response,
/// keeps the value and sends the value back; on the web build the browser keeps the cookie.
async fn login(user: &str, password: &str) -> Result<String, String> {
    let url = format!("{}/login", auth());
    let response = fetch::client()
        .post(&url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "user={}&password={}",
            super::world::query_encoded(user),
            super::world::query_encoded(password)
        ))
        .send()
        .await
        .map_err(|error| format!("cannot reach {url}: {error}"))?;
    let issued = session::issued(&response);
    let status = response.status();
    if !status.is_success() {
        // The service writes a short reason for ordinary failures; a longer body is a page.
        let said = response.text().await.unwrap_or_default();
        return Err(match said.trim() {
            reason if !reason.is_empty() && reason.len() < 120 => reason.to_owned(),
            _ => format!("{url} answered {status}"),
        });
    }
    issued.ok_or_else(|| format!("{url} did not answer with a session"))
}

/// Where the sign-in is held: natively the cookie's value, set on every signed-in request; on the
/// web build only a marker, since scripts cannot read or send the browser's cookie.
mod session {
    #[cfg(target_family = "wasm")]
    const HELD_BY_THE_BROWSER: &str = "browser";

    #[cfg(not(target_family = "wasm"))]
    pub fn issued(response: &reqwest::Response) -> Option<String> {
        response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|cookie| cookie.to_str().ok())
            .find_map(value)
    }

    /// Scripts cannot read the cookie on the web build.
    #[cfg(target_family = "wasm")]
    pub fn issued(_: &reqwest::Response) -> Option<String> {
        Some(HELD_BY_THE_BROWSER.to_owned())
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn sign(
        request: super::fetch::RequestBuilder,
        session: &str,
    ) -> super::fetch::RequestBuilder {
        request.header(
            reqwest::header::COOKIE,
            format!("{}={session}", super::COOKIE),
        )
    }

    /// The browser adds the cookie, and refuses a script-set `Cookie` header.
    #[cfg(target_family = "wasm")]
    pub fn sign(request: super::fetch::RequestBuilder, _: &str) -> super::fetch::RequestBuilder {
        request
    }

    /// Cookie attributes after the first `;` are ignored.
    #[cfg(not(target_family = "wasm"))]
    fn value(cookie: &str) -> Option<String> {
        let (name, session) = cookie.split(';').next()?.split_once('=')?;
        if name.trim() != super::COOKIE {
            return None;
        }
        let session = session.trim().trim_matches('"');
        (!session.is_empty()).then(|| session.to_owned())
    }

    #[cfg(test)]
    mod tests {
        #[test]
        #[cfg(not(target_family = "wasm"))]
        fn the_session_is_read_out_of_the_cookie_that_carries_it() {
            assert_eq!(
                super::value("auth=s3cr3t; Path=/; Domain=.ynoproject.net; HttpOnly"),
                Some("s3cr3t".to_owned())
            );
            assert_eq!(super::value("othercookie=whatever; Path=/"), None);
        }
    }
}

async fn visited(session: &str) -> Result<Visited, String> {
    let seen = seen(session).await?;
    let named = named().await?;
    let visited: Visited = seen
        .iter()
        .filter_map(|id| named.get(id).cloned())
        .collect();
    // The two lists are maintained apart, so few mismatches are ordinary and near-total ones a bug.
    if visited.len() < seen.len() {
        log::info!(
            "{} of the {} places visited are not worlds this draws",
            seen.len() - visited.len(),
            seen.len()
        );
    }
    Ok(visited)
}

/// Only the visited locations are read.
#[derive(serde::Deserialize)]
struct Info {
    /// `None` for a request with no session, or an expired one.
    #[serde(rename = "locationIds")]
    location_ids: Option<Vec<u64>>,
}

/// Only the id and the name are read: the dump already carries the rest.
#[derive(serde::Deserialize)]
struct Location {
    id: u64,
    title: String,
}

async fn seen(session: &str) -> Result<Vec<u64>, String> {
    let url = format!("{}/info", api());
    let said = session::sign(fetch::client().get(&url), session)
        .fetch_text()
        .await
        .map_err(|error| format!("cannot fetch {url}: {error}"))?;
    serde_json::from_str::<Info>(&said)
        .map_err(|error| format!("{url} is not an account: {error}"))?
        .location_ids
        // An expired session.
        .ok_or_else(|| t!("yno-signed-out"))
}

/// Public and the same for everyone, so fetched without a session and left to the ordinary
/// HTTP cache, which outlives the app.
async fn named() -> Result<std::collections::HashMap<u64, String>, String> {
    let url = format!("{}/gamelocations", api());
    let said = fetch::client()
        .get(&url)
        .fetch_text()
        .await
        .map_err(|error| format!("cannot fetch {url}: {error}"))?;
    Ok(serde_json::from_str::<Vec<Location>>(&said)
        .map_err(|error| format!("{url} is not a list of places: {error}"))?
        .into_iter()
        .map(|location| (location.id, location.title))
        .collect())
}
