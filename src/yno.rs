//! The player's own account on YNOproject, and the worlds it says they have been to.
//!
//! The game is played at <https://ynoproject.net>, which records every place its client has seen
//! the player standing in. That record is what turns this visualization from a map of the whole
//! game into a map of one person's game: see [`Account::frontier`] and `world::Dump::showing`.
//!
//! Two documents make it up, and they have to be read together. `GET /api/info` answers with the
//! ids of the places the account has been, and those ids are YNOproject's own database keys, which
//! mean nothing to a dump the wiki numbers by the game's maps. `GET /api/gamelocations` is what
//! turns them into names, which is the one thing the two sides do write the same way.
//!
//! The credential is a cookie. YNOproject's sign-in issues one, its own edge is what turns it into
//! the header its server reads, and a request carrying that header instead is answered as if
//! nobody had signed in at all. That is what splits the two platforms here: a native run may set a
//! cookie by hand and so keeps the session itself, while on the page the cookie is the browser's --
//! neither readable nor writable by a script, and sent on every same-origin request whether this
//! asks for it or not. See [`session`].

use super::i18n::t;
use super::{fetch, store};
use std::collections::HashSet;

/// Where a native run asks. YNOproject serves one host per game and this app draws exactly one:
/// Yume 2kki.
#[cfg(not(target_family = "wasm"))]
const API: &str = "https://api.ynoproject.net/2kki/api";
/// Where a native run signs in, which is a service of its own rather than a route of [`API`]'s.
#[cfg(not(target_family = "wasm"))]
const AUTH: &str = "https://auth.ynoproject.net";

/// The path the page asks under instead of [`API`], and which its own host puts through.
///
/// The page cannot ask YNOproject directly: it allows exactly one origin -- its own site -- and a
/// browser will not send the request from anywhere else. So the page asks its own host, which
/// makes the request same-origin, for the same reason and by the same arrangement as the wiki's
/// pictures in `world`. It is also what makes the sign-in's cookie keepable at all, since a
/// browser only keeps one for the origin that served it. See `dreamweaver`'s `relay`, which is
/// what answers these, and `Trunk.toml`, which is how a development page reaches it.
///
/// What this costs is that the host relays a signed-in request, and so sees the session it
/// carries. That is the price of the feature existing on the page at all; a run that would rather
/// not pay it is a native run, where nothing leaves the machine.
#[cfg(target_family = "wasm")]
const API_PATH: &str = "/yno";
/// See [`API_PATH`]. Its own path because signing in is its own host.
#[cfg(target_family = "wasm")]
const AUTH_PATH: &str = "/ynoauth";

/// What the sign-in is kept under between runs. See [`session`], which is what is kept there and
/// is not the same thing on both platforms.
const SESSION: &str = "yno-session";
/// The cookie YNOproject's sign-in issues, which is the credential every signed-in request
/// carries. Named here only where this app is the one carrying it: on the page it is the
/// browser's, and nothing here ever writes it out. See [`session`].
#[cfg(not(target_family = "wasm"))]
const COOKIE: &str = "auth";
/// What the frontier switch is kept under. Present for on, absent for off.
const FRONTIER: &str = "frontier";

/// The titles of the worlds an account has stood in, as the wiki writes them.
///
/// Names rather than either side's numbering, because a name is the only thing YNOproject's
/// locations and this app's dump have in common. See [`Account::visited`].
pub type Visited = HashSet<String>;

/// Where a sign-in or a resumption got to, which is the whole of what the settings tab draws.
pub enum State<'a> {
    /// Nobody has signed in this run and no earlier run left a session behind.
    SignedOut,
    /// Signing in, or fetching what the account has seen. Neither is worth telling apart on
    /// screen: both are the same wait for the same answer.
    Working,
    /// Signed in, and what the account has been to is in hand. How much of the game that is,
    /// is measured against the dump rather than counted here -- see `world::Dump::visited`.
    SignedIn,
    /// Why the last attempt came to nothing, in the server's own words where it wrote any.
    Failed(&'a str),
}

/// The account this run is signed in to, and what it has seen.
///
/// Held by the app rather than by the overlay, because it is what the graph is built out of: a
/// sign-in rebuilds the graph, the same way the dump landing does. See `app`'s `App::build`.
#[derive(Default)]
pub struct Account {
    /// The session this run signs its requests with, kept between runs so signing in is something
    /// a person does once rather than every time they open the app.
    session: Option<String>,
    /// What the account has been to, once it has arrived.
    visited: Option<Visited>,
    /// The sign-in or resumption in flight. It answers with both, because a resumption hands back
    /// the session it was given and a sign-in hands back the one it was issued: the caller has the
    /// same two things to write down either way.
    asking: Option<fetch::Pending<Result<(String, Visited), String>>>,
    /// Why the last attempt came to nothing, until the next one is started.
    failed: Option<String>,
    /// Whether the person wants the graph cut back to what they have seen. Kept apart from
    /// [`Account::visited`] so that signing out of the frontier does not sign out of the account,
    /// and so the switch answers instantly once the visits are in hand.
    frontier: bool,
    /// Whether the graph is now built out of something other than what this says, and so has to be
    /// built again. See [`Account::restated`].
    restated: bool,
    /// Worlds this run is pretending the account has been to. See [`Account::pretend`].
    pretended: Visited,
}

impl Account {
    /// Picks up whatever an earlier run left: the session, the switch, and — if there is a session
    /// — what that account has seen.
    ///
    /// Fetched on the way in rather than when the switch is turned on, because the switch is
    /// meant to answer at once and the fetch is two documents off someone else's server. A run
    /// that never turns it on has spent two requests; a run that does has spent none at the press.
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

    /// Reads whatever the attempt in flight came back with. Called once a frame; does nothing on
    /// the frames there is nothing to read.
    pub fn poll(&mut self) {
        let Some(answer) = self.asking.as_ref().and_then(fetch::Pending::take) else {
            return;
        };
        self.asking = None;
        // Whether what the graph would be built out of is now different, which a refresh that
        // turned nothing up is not: rebuilding then would throw away a layout and a selection to
        // arrive back at the picture already on screen. An answer that lands on top of a pretence
        // always is: it is putting back a world the pretence had taken the graph past. See
        // [`Account::pretend`].
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
                // The session is dropped along with the attempt: the one thing that fails this
                // way in ordinary use is a session that has run out, and keeping it would only
                // fail again on the next run. Signing in again is the whole of the fix.
                store::write(SESSION, None);
                self.session = None;
                self.visited = None;
                self.failed = Some(error);
            }
        }
        // Whether it worked or not: a failure is as much a reason to build the graph again as an
        // arrival is, because it may be what took a frontier away. Only while the switch is on,
        // though -- with it off the graph is the whole game either way, and rebuilding it would
        // throw away a layout and a selection to arrive at the same picture.
        self.restated |= self.frontier && changed;
    }

    /// Signs in and reads what the account has seen, in one go: the session is only ever wanted
    /// for the reading, so there is no state between them worth showing.
    pub fn sign_in(&mut self, user: String, password: String) {
        self.failed = None;
        self.asking = Some(fetch::spawn(sign_in(user, password)));
    }

    /// Forgets the session and everything read with it, here and between runs.
    ///
    /// YNOproject is not told: it has a route for ending a session, and using it would end the one
    /// the person is playing the game with as well. What is dropped is this app's copy -- the
    /// value a native run wrote down, and, on the page, the cookie the page's own host handed back
    /// for its origin. See [`forget`] and `dreamweaver`'s `relay`.
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

    /// Asks YNOproject again what this account has been to.
    ///
    /// The one thing that goes stale here: the app reads the account when it starts and then not
    /// again, while the person is playing the game in another window and walking into places all
    /// the while. So there is a button, rather than a poll -- the answer only matters when someone
    /// wants to look at it, and asking on a timer would be asking someone else's server for
    /// something nobody is reading.
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

    /// Everywhere the account has been, whether or not the graph is being cut back by it.
    ///
    /// Unlike [`Account::frontier`], which answers only where the switch is on: how much of the
    /// game someone has seen is worth saying either way, and is not a reason to redraw anything.
    pub fn visited(&self) -> Option<&Visited> {
        self.visited.as_ref()
    }

    /// Whether the switch is on, whether or not there is anything to show for it yet.
    pub fn frontier_wanted(&self) -> bool {
        self.frontier
    }

    /// Turns the switch, and remembers which way it was left.
    pub fn set_frontier(&mut self, frontier: bool) {
        if frontier == self.frontier {
            return;
        }
        self.frontier = frontier;
        store::write(FRONTIER, frontier.then_some(""));
        // Only where there is something to cut the graph back by. Turning the switch on before an
        // account has been read changes nothing yet, and reading one is what will say so.
        self.restated |= self.visited.is_some();
    }

    /// Pretends the account has been to a world, so the graph opens out past it.
    ///
    /// For looking at what a frontier would become without having to go and play the game to it,
    /// which is the only other way to see the graph do this. Nothing is sent to YNOproject and
    /// nothing is written down: the pretence lives in this field, and it is gone with the run, the
    /// sign-out, or the next answer from the server -- a refresh is how the real frontier is put
    /// back. It is kept out of [`Account::visited`] for the same reason: what the server said is
    /// what it said, and the completion this app reports goes on reading it rather than the story.
    pub fn pretend(&mut self, title: String) {
        // Nothing to pretend about a world the account has really been to, or one already being
        // pretended about: either way the graph is already drawn as though this had happened, and
        // building it again would cost a layout and a camera to arrive back at it.
        let known = self
            .visited
            .as_ref()
            .is_some_and(|visited| visited.contains(&title));
        if known || !self.pretended.insert(title.clone()) {
            return;
        }
        log::info!("pretending {title} has been visited");
        // The same reading [`Account::set_frontier`] makes: without an account there is no
        // frontier for a pretence to be part of, and nothing on screen would change.
        self.restated |= self.frontier && self.visited.is_some();
    }

    /// What the graph should be cut back to, or `None` for a run that should draw the whole game:
    /// the switch off, or nothing read to cut it back by. See `world::Dump::showing`.
    ///
    /// Borrowed where the account is being taken at its word, which is every ordinary run, and
    /// owned only where a pretence has been laid over it. See [`Account::pretend`].
    pub fn frontier(&self) -> Option<std::borrow::Cow<'_, Visited>> {
        let visited = self.visited.as_ref().filter(|_| self.frontier)?;
        Some(match self.pretended.is_empty() {
            true => std::borrow::Cow::Borrowed(visited),
            false => std::borrow::Cow::Owned(visited.union(&self.pretended).cloned().collect()),
        })
    }

    /// Whether the graph in front of the person is built out of something this no longer says, and
    /// so has to be built again. Answered once: the asking is what clears it.
    pub fn restated(&mut self) -> bool {
        std::mem::take(&mut self.restated)
    }

    /// Whether what the graph should be built out of is settled, and so whether it is worth
    /// building one at all yet.
    ///
    /// A run that resumed a session asks YNOproject about it while the dump is still on its way,
    /// and the two arrive in whichever order the network hands them over. Building on the dump
    /// alone would mean drawing the whole game and then, a moment later, throwing it away for the
    /// frontier -- so the graph waits for both, and is drawn once. See `app`'s `App::build`.
    ///
    /// Only while the switch is on: with it off the answer changes nothing about what is built,
    /// and there is nothing to wait for. Nor is an attempt that has finished worth waiting on,
    /// however it finished -- a failure settles the question as much as an answer does.
    pub fn settled(&self) -> bool {
        !self.frontier || self.asking.is_none()
    }

    /// Where the sign-in has got to, for the settings tab to say.
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

    /// Whether there is a session to sign out of, which is also what settles whether the tab
    /// offers the fields or the button.
    pub fn signed_in(&self) -> bool {
        self.session.is_some()
    }
}

/// Where this run asks for what an account has seen.
///
/// Built once and kept on the page, for the reason `world::proxied_images` is: these addresses are
/// handed to `reqwest`, which parses each one by itself and has no document to resolve a bare path
/// against, so the page's own origin has to be written into them.
fn api() -> &'static str {
    #[cfg(not(target_family = "wasm"))]
    return API;
    #[cfg(target_family = "wasm")]
    {
        static API: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        API.get_or_init(|| format!("{}{API_PATH}", super::world::origin()))
    }
}

/// Where this run signs in. See [`api`].
fn auth() -> &'static str {
    #[cfg(not(target_family = "wasm"))]
    return AUTH;
    #[cfg(target_family = "wasm")]
    {
        static AUTH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        AUTH.get_or_init(|| format!("{}{AUTH_PATH}", super::world::origin()))
    }
}

/// Drops the page's own copy of the session, which is a cookie and so is the browser's to drop.
///
/// Asked of the host that handed it over, since that is the only party that can take it back: a
/// script may not write the cookie, and the host set it for this origin on the way past. Started
/// and not waited on, and its answer is not read -- a sign-out has already happened here whatever
/// the host says about it.
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

/// Signs in, then reads what the account has seen.
async fn sign_in(user: String, password: String) -> Result<(String, Visited), String> {
    let session = login(&user, &password).await?;
    let visited = visited(&session).await?;
    Ok((session, visited))
}

/// Reads what the account a session belongs to has seen, handing the session back so that the
/// caller has the same pair either way. See [`Account::asking`].
async fn resume(session: String) -> Result<(String, Visited), String> {
    let visited = visited(&session).await?;
    Ok((session, visited))
}

/// Exchanges a name and a password for a session.
///
/// The service answers by setting a cookie, which the two platforms are told by in different ways.
/// A native run reads it off the response and keeps it, because it is going to have to send it
/// itself. The page is not told at all: the browser keeps `Set-Cookie` from the script that asked
/// for it, and there is nothing for the page to read or to do -- its host handed the cookie back
/// for this origin on the way past, and the browser will send it from here on by itself. See
/// `dreamweaver`'s `relay`.
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
        // The service writes a short reason for the ordinary failures -- a wrong password, a name
        // that is not registered -- and those are worth reading out. Anything longer is a page
        // rather than a reason, and the status is all of it worth showing.
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
/// Natively it is the cookie's own value, kept by this app and set on every signed-in request.
/// On the page it is the browser's cookie, and what is kept here is only a note that the browser
/// has one: a script may neither read that cookie nor send it by hand, and does not have to --
/// every same-origin request carries it already. So the page's copy is a marker, and the one thing
/// read out of it is whether it is there. See [`Account::session`].
mod session {
    /// What the page writes down instead of a session, since it never sees one.
    #[cfg(target_family = "wasm")]
    const HELD_BY_THE_BROWSER: &str = "browser";

    /// The session a sign-in issued, out of its answer.
    #[cfg(not(target_family = "wasm"))]
    pub fn issued(response: &reqwest::Response) -> Option<String> {
        response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|cookie| cookie.to_str().ok())
            .find_map(value)
    }

    /// See the native [`issued`] above: there is nothing here for the page to read.
    #[cfg(target_family = "wasm")]
    pub fn issued(_: &reqwest::Response) -> Option<String> {
        Some(HELD_BY_THE_BROWSER.to_owned())
    }

    /// Signs a request with the session, where this platform is the one doing the signing.
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

    /// The session out of one `Set-Cookie`, if that is the cookie it carries: the value of its
    /// first `name=value`, everything after the first `;` being how long to keep it and where.
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
        /// The sign-in sets more than one cookie, and only one of them is the session.
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

/// The titles of the worlds a session's account has stood in.
async fn visited(session: &str) -> Result<Visited, String> {
    let seen = seen(session).await?;
    let named = named().await?;
    let visited: Visited = seen
        .iter()
        .filter_map(|id| named.get(id).cloned())
        .collect();
    // The two lists are kept by different people out of the same wiki, so a place YNOproject knows
    // and the dump does not is ordinary rather than wrong -- but a run where nearly none of them
    // line up is a run drawing the wrong thing, and this is the only place that would show it.
    if visited.len() < seen.len() {
        log::info!(
            "{} of the {} places visited are not worlds this draws",
            seen.len() - visited.len(),
            seen.len()
        );
    }
    Ok(visited)
}

/// What YNOproject says about the account a session belongs to. Only the places it has been are
/// read; the rest is who they are on the site, which this app has nothing to do with.
#[derive(serde::Deserialize)]
struct Info {
    /// `None` for a request with no session, or one whose session has run out.
    #[serde(rename = "locationIds")]
    location_ids: Option<Vec<u64>>,
}

/// One place YNOproject knows, as its own catalog lists it. Only the id and the name are read: the
/// rest of the entry is the wiki's own account of the place, which the dump already carries.
#[derive(serde::Deserialize)]
struct Location {
    id: u64,
    title: String,
}

/// The ids of the places a session's account has been.
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
        // Answered, but with nothing about an account: the session is not one YNOproject knows
        // any more, which is what an expired one looks like.
        .ok_or_else(|| t!("yno-signed-out"))
}

/// Every place YNOproject knows, by id. Public, and the same for everyone, so it is asked for
/// without a session and left to the ordinary HTTP cache between runs -- see `fetch`.
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

/// One field of a form-encoded body. Its own rather than a crate's: this app posts one form, of
/// two fields, once.
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
