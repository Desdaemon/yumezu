//! The player's own account on YNOproject, and the worlds it says they have been to.
//!
//! The game is played at <https://ynoproject.net>, which records every place its client has seen
//! the player standing in. That record turns this from a map of the whole game into a map of one
//! person's game: see [`Account::frontier`].
//!
//! Two documents make it up, and they have to be read together: `GET /api/info` answers with
//! YNOproject's own database ids, which mean nothing to a dump the wiki numbers by the game's
//! maps, and `GET /api/gamelocations` turns them into names -- the one thing the two sides write
//! the same way.
//!
//! The credential is a cookie, and only a cookie: a request carrying its value as a header
//! instead is answered as if nobody had signed in. That is what splits the two platforms here. A
//! native run may set a cookie by hand and so keeps the session itself; on the page the cookie is
//! the browser's, neither readable nor writable by a script and sent whether this asks or not.

use super::i18n::t;
use super::{fetch, store};
use std::collections::HashSet;

/// YNOproject serves one host per game, and this app draws exactly one: Yume 2kki.
#[cfg(not(target_family = "wasm"))]
const API: &str = "https://api.ynoproject.net/2kki/api";
/// Signing in is a service of its own rather than a route of [`API`]'s.
#[cfg(not(target_family = "wasm"))]
const AUTH: &str = "https://auth.ynoproject.net";

/// Relayed by the page's own host rather than asked of [`API`] directly: YNOproject allows
/// exactly one origin -- its own site -- and a browser will not send the request from anywhere
/// else. Being same-origin is also what makes the sign-in's cookie keepable at all, a browser only
/// keeping one for the origin that served it. See `dreamweaver`'s `relay`, which answers these,
/// and `Trunk.toml`, which is how a development page reaches it.
///
/// The cost is that the host sees the session a relayed request carries. A run that would rather
/// not pay that is a native run, where nothing leaves the machine.
#[cfg(target_family = "wasm")]
const API_PATH: &str = "/yno";
/// A path of its own because signing in is its own host. See [`API_PATH`].
#[cfg(target_family = "wasm")]
const AUTH_PATH: &str = "/ynoauth";

// What [`session`] leaves between runs, which is not the same thing on both platforms.
const SESSION: &str = "yno-session";
/// The cookie YNOproject's sign-in issues, which is the credential every signed-in request
/// carries. Named only where this app is the one carrying it: on the page it is the browser's,
/// and nothing here ever writes it out.
#[cfg(not(target_family = "wasm"))]
const COOKIE: &str = "auth";
// Present for on, absent for off.
const FRONTIER: &str = "frontier";

/// The titles of the worlds an account has stood in, as the wiki writes them: names rather than
/// either side's numbering, that being the only thing YNOproject and the dump have in common.
pub type Visited = HashSet<String>;

/// Where a sign-in or a resumption got to, which is the whole of what the settings tab draws.
pub enum State<'a> {
    /// Nobody has signed in this run and no earlier run left a session behind.
    SignedOut,
    /// Signing in, or fetching what the account has seen: both are the same wait for the same
    /// answer, and not worth telling apart on screen.
    Working,
    SignedIn,
    /// Why the last attempt came to nothing, in the server's own words where it wrote any.
    Failed(&'a str),
}

/// Held by the app rather than the overlay, because it is what the graph is built out of: a
/// sign-in rebuilds the graph, the same way the dump landing does. See `app`'s `App::build`.
#[derive(Default)]
pub struct Account {
    /// Kept between runs, so signing in is something a person does once rather than every time
    /// they open the app.
    session: Option<String>,
    visited: Option<Visited>,
    /// The sign-in or resumption in flight. Answers with both session and visits, so the caller
    /// has the same two things to write down whichever it was.
    asking: Option<fetch::Pending<Result<(String, Visited), String>>>,
    /// Why the last attempt came to nothing, until the next one is started.
    failed: Option<String>,
    /// Whether the person wants the graph cut back to what they have seen. Kept apart from
    /// [`Account::visited`] so that leaving the frontier does not sign out of the account, and so
    /// the switch answers instantly once the visits are in hand.
    frontier: bool,
    restated: bool,
    pretended: Visited,
}

impl Account {
    /// Picks up whatever an earlier run left, fetching the visits on the way in rather than when
    /// the switch is turned on: the switch is meant to answer at once, and the fetch is two
    /// documents off someone else's server.
    pub fn new() -> Self {
        let session = store::read(SESSION).filter(|session| !session.is_empty());
        let asking = session.clone().map(|session| fetch::spawn(resume(session)));
        Self {
            session,
            asking,
            frontier: store::read(FRONTIER).is_some(),
            ..Default::default()
        }
    }

    /// Called once a frame, and does nothing on the frames there is nothing to read.
    pub fn poll(&mut self) {
        let Some(answer) = self.asking.as_ref().and_then(fetch::Pending::take) else {
            return;
        };
        self.asking = None;
        // A refresh that turned nothing up is not a change: rebuilding would throw away a layout
        // and a selection to arrive back at the same picture. An answer landing on top of a
        // pretence always is -- it puts back a world the pretence had taken the graph past.
        let mut changed = !self.pretended.is_empty();
        self.pretended.clear();
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
                // The one thing that fails this way in ordinary use is a session that has run
                // out, and keeping it would only fail again on the next run.
                store::write(SESSION, None);
                self.session = None;
                self.visited = None;
                self.failed = Some(error);
            }
        }
        // A failure is as much a reason to build the graph again, since it may be what took a
        // frontier away. Only while the switch is on: with it off the graph is the whole game.
        self.restated |= self.frontier && changed;
    }

    /// Reads what the account has seen in the same go: the session is only ever wanted for the
    /// reading, so there is no state between the two worth showing.
    pub fn sign_in(&mut self, user: String, password: String) {
        self.failed = None;
        self.asking = Some(fetch::spawn(sign_in(user, password)));
    }

    /// YNOproject is not told: it has a route for ending a session, and using it would end the one
    /// the person is playing the game with too. Only this app's copy is dropped. See [`forget`].
    pub fn sign_out(&mut self) {
        forget();
        store::write(SESSION, None);
        self.session = None;
        self.visited = None;
        self.asking = None;
        self.failed = None;
        self.pretended.clear();
        // See [`Account::poll`] for why the switch is what settles this.
        self.restated |= self.frontier;
    }

    /// The one thing that goes stale here: the app reads the account once at startup while the
    /// person goes on walking into places in another window. A button rather than a poll, the
    /// answer only mattering when someone wants to look at it.
    ///
    /// Does nothing without a session to ask with, or while an attempt is already in flight.
    pub fn refresh(&mut self) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if self.asking.is_some() {
            return;
        }
        self.failed = None;
        self.asking = Some(fetch::spawn(resume(session)));
    }

    /// Everywhere the account has been, whether or not the graph is being cut back by it -- unlike
    /// [`Account::frontier`]. How much of the game someone has seen is worth saying either way,
    /// and is no reason to redraw anything.
    pub fn visited(&self) -> Option<&Visited> {
        self.visited.as_ref()
    }

    /// On, whether or not there is anything to show for it yet.
    pub fn frontier_wanted(&self) -> bool {
        self.frontier
    }

    pub fn set_frontier(&mut self, frontier: bool) {
        if frontier == self.frontier {
            return;
        }
        self.frontier = frontier;
        store::write(FRONTIER, frontier.then_some(""));
        // Turning the switch on before an account has been read changes nothing yet, and reading
        // one is what will say so.
        self.restated |= self.visited.is_some();
    }

    /// Pretends the account has been to a world, so the graph opens out past it: for looking at
    /// what a frontier would become without playing the game to it.
    ///
    /// Nothing is sent and nothing written down, so the pretence goes with the run, the sign-out,
    /// or the next answer from the server. Kept out of [`Account::visited`] so the completion this
    /// app reports still reads what the server said rather than the story.
    pub fn pretend(&mut self, title: String) {
        // The graph is already drawn as though this had happened.
        let known = self
            .visited
            .as_ref()
            .is_some_and(|visited| visited.contains(&title));
        if known || !self.pretended.insert(title.clone()) {
            return;
        }
        log::info!("pretending {title} has been visited");
        // As in [`Account::set_frontier`]: without an account there is no frontier for a pretence
        // to be part of, and nothing on screen would change.
        self.restated |= self.frontier && self.visited.is_some();
    }

    /// What the graph should be cut back to, or `None` where it should draw the whole game: the
    /// switch off, or nothing read to cut it back by. Owned only where a pretence has been laid
    /// over the account. See `world::Dump::showing`.
    pub fn frontier(&self) -> Option<std::borrow::Cow<'_, Visited>> {
        let visited = self.visited.as_ref().filter(|_| self.frontier)?;
        Some(match self.pretended.is_empty() {
            true => std::borrow::Cow::Borrowed(visited),
            false => std::borrow::Cow::Owned(visited.union(&self.pretended).cloned().collect()),
        })
    }

    /// Whether the graph in front of the person is built out of something this no longer says.
    /// Answered once: the asking is what clears it.
    pub fn restated(&mut self) -> bool {
        std::mem::take(&mut self.restated)
    }

    /// Whether what the graph should be built out of is settled, and so whether building one is
    /// worth it yet.
    ///
    /// A run resuming a session asks YNOproject about it while the dump is still on its way, and
    /// the two arrive in whichever order the network hands them over. Building on the dump alone
    /// would draw the whole game and then throw it away for the frontier.
    ///
    /// Only while the switch is on: with it off the answer changes nothing about what is built. A
    /// finished attempt is settled however it finished -- a failure answers the question too.
    pub fn settled(&self) -> bool {
        !self.frontier || self.asking.is_none()
    }

    pub fn state(&self) -> State<'_> {
        if self.asking.is_some() {
            return State::Working;
        }
        match (&self.visited, &self.failed) {
            (Some(_), _) => State::SignedIn,
            (None, Some(failed)) => State::Failed(failed),
            (None, None) => State::SignedOut,
        }
    }

    /// Also what settles whether the tab offers the fields or the sign-out button.
    pub fn signed_in(&self) -> bool {
        self.session.is_some()
    }
}

/// Built once and kept on the page, for the reason `world::proxied_images` is: these addresses go
/// to `reqwest`, which parses each one by itself and has no document to resolve a bare path
/// against, so the page's own origin has to be written in.
fn api() -> &'static str {
    #[cfg(not(target_family = "wasm"))]
    return API;
    #[cfg(target_family = "wasm")]
    {
        static API: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        API.get_or_init(|| format!("{}{API_PATH}", super::world::origin()))
    }
}

/// See [`api`].
fn auth() -> &'static str {
    #[cfg(not(target_family = "wasm"))]
    return AUTH;
    #[cfg(target_family = "wasm")]
    {
        static AUTH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        AUTH.get_or_init(|| format!("{}{AUTH_PATH}", super::world::origin()))
    }
}

/// The page's session is a cookie, so it is dropped by asking the host that handed it over -- the
/// only party that can, a script not being allowed to write it. Not waited on and its answer not
/// read: the sign-out has already happened here whatever the host says about it.
#[cfg(target_family = "wasm")]
fn forget() {
    let url = format!("{}/forget", auth());
    drop(fetch::spawn(async move {
        if let Err(error) = fetch::client().post(&url).send().await {
            log::warn!("cannot reach {url}: {error}");
        }
    }));
}

/// Nothing to ask anyone: a native run keeps the session itself, and the store is where it was
/// kept. See the page's [`forget`] above.
#[cfg(not(target_family = "wasm"))]
fn forget() {}

async fn sign_in(user: String, password: String) -> Result<(String, Visited), String> {
    let session = login(&user, &password).await?;
    let visited = visited(&session).await?;
    Ok((session, visited))
}

/// Hands the session back so the caller has the same pair either way. See [`Account::asking`].
async fn resume(session: String) -> Result<(String, Visited), String> {
    let visited = visited(&session).await?;
    Ok((session, visited))
}

/// The service answers by setting a cookie, which the two platforms read differently: a native run
/// takes it off the response and keeps it, having to send it itself, where the page is not told at
/// all -- the browser keeps `Set-Cookie` and sends it from here on by itself.
async fn login(user: &str, password: &str) -> Result<String, String> {
    let url = format!("{}/login", auth());
    let response = fetch::client()
        .post(&url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "user={}&password={}",
            encoded(user),
            encoded(password)
        ))
        .send()
        .await
        .map_err(|error| format!("cannot reach {url}: {error}"))?;
    let issued = session::issued(&response);
    let status = response.status();
    if !status.is_success() {
        // The service writes a short reason for the ordinary failures -- a wrong password, an
        // unregistered name -- which are worth reading out. Anything longer is a page rather than
        // a reason, and the status is all of it worth showing.
        let said = response.text().await.unwrap_or_default();
        return Err(match said.trim() {
            reason if !reason.is_empty() && reason.len() < 120 => reason.to_owned(),
            _ => format!("{url} answered {status}"),
        });
    }
    issued.ok_or_else(|| format!("{url} did not answer with a session"))
}

/// Where the sign-in is held, which is not the same place on the two platforms.
///
/// Natively it is the cookie's own value, kept by this app and set on every signed-in request. On
/// the page it is the browser's cookie, and what is kept here is only a marker that the browser
/// has one -- a script may neither read that cookie nor send it by hand, and does not have to. So
/// the one thing ever read out of the page's copy is whether it is there.
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

    /// There is nothing here for the page to read. See the native [`issued`] above.
    #[cfg(target_family = "wasm")]
    pub fn issued(_: &reqwest::Response) -> Option<String> {
        Some(HELD_BY_THE_BROWSER.to_owned())
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn sign(
        asking: super::fetch::RequestBuilder,
        session: &str,
    ) -> super::fetch::RequestBuilder {
        asking.header(
            reqwest::header::COOKIE,
            format!("{}={session}", super::COOKIE),
        )
    }

    /// The browser has already signed it, and will not let this add the header even to agree.
    #[cfg(target_family = "wasm")]
    pub fn sign(asking: super::fetch::RequestBuilder, _: &str) -> super::fetch::RequestBuilder {
        asking
    }

    /// The value of the header's first `name=value`, everything after the first `;` being how
    /// long to keep it and where.
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
        // The sign-in sets more than one cookie, and only one of them is the session.
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
    // The two lists are kept by different people out of the same wiki, so a place YNOproject
    // knows and the dump does not is ordinary rather than wrong. A run where nearly none of them
    // line up is drawing the wrong thing, and this is the only place that would show it.
    if visited.len() < seen.len() {
        log::info!(
            "{} of the {} places visited are not worlds this draws",
            seen.len() - visited.len(),
            seen.len()
        );
    }
    Ok(visited)
}

/// Only the places the account has been are read; the rest is who they are on the site, which
/// this app has nothing to do with.
#[derive(serde::Deserialize)]
struct Info {
    /// `None` for a request with no session, or one whose session has run out.
    #[serde(rename = "locationIds")]
    location_ids: Option<Vec<u64>>,
}

/// Only the id and the name are read: the rest of an entry is the wiki's own account of the place,
/// which the dump already carries.
#[derive(serde::Deserialize)]
struct Location {
    id: u64,
    title: String,
}

async fn seen(session: &str) -> Result<Vec<u64>, String> {
    let url = format!("{}/info", api());
    let said = session::sign(fetch::client().get(&url), session)
        .send()
        .await
        .map_err(|error| format!("cannot reach {url}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("{url} refused: {error}"))?
        .text()
        .await
        .map_err(|error| format!("cannot read {url}: {error}"))?;
    serde_json::from_str::<Info>(&said)
        .map_err(|error| format!("{url} is not an account: {error}"))?
        .location_ids
        // Answered, but with nothing about an account, which is what an expired session looks
        // like.
        .ok_or_else(|| t!("yno-signed-out"))
}

/// Every place YNOproject knows, by id. Public and the same for everyone, so it is asked for
/// without a session and left to the ordinary HTTP cache between runs.
async fn named() -> Result<std::collections::HashMap<u64, String>, String> {
    let url = format!("{}/gamelocations", api());
    let said = fetch::client()
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("cannot reach {url}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("{url} refused: {error}"))?
        .text()
        .await
        .map_err(|error| format!("cannot read {url}: {error}"))?;
    Ok(serde_json::from_str::<Vec<Location>>(&said)
        .map_err(|error| format!("{url} is not a list of places: {error}"))?
        .into_iter()
        .map(|location| (location.id, location.title))
        .collect())
}

/// Hand-rolled rather than a crate's: this app posts one form, of two fields, once.
fn encoded(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    for byte in field.bytes() {
        match byte {
            byte if byte.is_ascii_alphanumeric() => out.push(byte as char),
            b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            byte => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
