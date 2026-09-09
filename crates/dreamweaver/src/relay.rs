//! What the page asks about a player's own YNOproject account, put through to YNOproject.
//!
//! The page cannot ask YNOproject itself: that host allows exactly one origin -- its own site --
//! and the credential it answers to is a cookie, which a script may neither read nor set by hand.
//!
//! The one thing done beyond forwarding is the sign-in's cookie. YNOproject issues it for its own
//! domain, which a browser reading this page would refuse to keep, so [`resettled`] hands it back
//! for whatever origin served the page.
//!
//! The cost is that a signed-in request goes through this host, so this host sees the session. That
//! is why the native builds ask YNOproject directly instead.
//!
//! Only the routes the app uses, rather than a path forwarding whatever it is given: an open relay
//! to someone else's API is not something to run by accident.

use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

/// One host per game, and this app draws exactly one.
const API: &str = "https://api.ynoproject.net/2kki/api";
/// Where signing in goes, which is a service of its own rather than a route of [`API`]'s.
const AUTH: &str = "https://auth.ynoproject.net";
/// The cookie YNOproject's sign-in issues, and the only one carried in either direction.
const SESSION: &str = "auth";

/// Merged into the server's own router. No state of its own: a relay keeps nothing.
pub fn routes<S: Clone + Send + Sync + 'static>() -> axum::Router<S> {
    axum::Router::new()
        .route("/yno/info", get(info))
        .route("/yno/gamelocations", get(locations))
        .route("/ynoauth/login", post(login))
        .route("/ynoauth/forget", post(forget))
}

/// `GET /yno/info` -- who the session belongs to, and everywhere they have been. The one route
/// that carries the session, being the one that wants it.
async fn info(headers: HeaderMap) -> Response {
    forward(&format!("{API}/info"), session(&headers)).await
}

/// `GET /yno/gamelocations` -- every place YNOproject knows, by id.
///
/// The same document for everybody, so it is asked for without a session.
async fn locations() -> Response {
    forward(&format!("{API}/gamelocations"), None).await
}

/// `POST /ynoauth/login` -- a name and a password for a session.
///
/// The body is passed through as it arrived, being a form this host has no business reading, and
/// so is the answer but for the cookie: see [`resettled`].
async fn login(headers: HeaderMap, body: Bytes) -> Response {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("application/x-www-form-urlencoded"));
    let sent = client()
        .post(format!("{AUTH}/login"))
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .send()
        .await;
    let answer = match sent {
        Ok(answer) => answer,
        Err(error) => {
            tracing::warn!("cannot reach {AUTH}/login: {error}");
            return (StatusCode::BAD_GATEWAY, "cannot reach ynoproject\n").into_response();
        }
    };
    let status = answer.status();
    let cookies: Vec<HeaderValue> = answer
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|cookie| cookie.to_str().ok())
        .filter_map(resettled)
        .collect();
    let said = answer.bytes().await.unwrap_or_default();

    let mut response = (status, said).into_response();
    for cookie in cookies {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

/// `POST /ynoauth/forget` -- takes back the copy of the session this origin was given.
///
/// Not YNOproject's own sign-out: signing out of a map should not sign someone out of the game
/// they are playing in another browser tab.
async fn forget() -> Response {
    (
        [(
            header::SET_COOKIE,
            format!("{SESSION}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"),
        )],
        "ok\n",
    )
        .into_response()
}

/// Status and body as they came back: a failure upstream is for the page to read and say.
async fn forward(url: &str, session: Option<HeaderValue>) -> Response {
    let mut asking = client().get(url);
    if let Some(session) = session {
        asking = asking.header(header::COOKIE, session);
    }
    match asking.send().await {
        Ok(answer) => {
            let status = answer.status();
            let content_type = answer
                .headers()
                .get(header::CONTENT_TYPE)
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_static("text/plain; charset=utf-8"));
            let said = answer.bytes().await.unwrap_or_default();
            (status, [(header::CONTENT_TYPE, content_type)], said).into_response()
        }
        Err(error) => {
            tracing::warn!("cannot reach {url}: {error}");
            (StatusCode::BAD_GATEWAY, "cannot reach ynoproject\n").into_response()
        }
    }
}

/// [`SESSION`] and nothing else: the page's origin may carry cookies of its own, and none of them
/// are YNOproject's business.
fn session(headers: &HeaderMap) -> Option<HeaderValue> {
    let sent = headers.get(header::COOKIE)?.to_str().ok()?;
    let session = sent
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| name.trim() == SESSION)
        .map(|(_, session)| session.trim())?;
    HeaderValue::from_str(&format!("{SESSION}={session}")).ok()
}

/// One `Set-Cookie` from YNOproject, made keepable by whatever browser is reading this page.
///
/// `Domain` is dropped, which is the whole point: a browser refuses a cookie for a domain that is
/// not the one that served the response, and without the attribute the cookie belongs to whichever
/// origin served it. `Path` is forced to the root, the page asking under paths of its own.
/// Everything else is left as YNOproject wrote it.
///
/// `None` for a cookie that is not the session.
fn resettled(cookie: &str) -> Option<HeaderValue> {
    let mut parts = cookie.split(';');
    let pair = parts.next()?.trim();
    let (name, _) = pair.split_once('=')?;
    if name.trim() != SESSION {
        return None;
    }
    let mut resettled = String::from(pair);
    resettled.push_str("; Path=/");
    for attribute in parts {
        let named = attribute.trim();
        let key = named.split('=').next().unwrap_or_default().trim();
        if key.eq_ignore_ascii_case("domain") || key.eq_ignore_ascii_case("path") {
            continue;
        }
        resettled.push_str("; ");
        resettled.push_str(named);
    }
    HeaderValue::from_str(&resettled).ok()
}

/// Built once and handed out by the clone: it keeps a connection pool, so the second question of a
/// sign-in reuses the first one's socket.
fn client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            // `rustls-no-provider` leaves this to the process, and reqwest panics without it.
            let _ = rustls::crypto::ring::default_provider().install_default();
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("cannot build an http client")
        })
        .clone()
}

#[cfg(test)]
mod tests {
    /// A browser keeps a cookie only for the domain that served it.
    #[test]
    fn a_session_is_resettled_on_whoever_served_it() {
        let resettled = super::resettled(
            "auth=s3cr3t; Path=/login; Domain=.ynoproject.net; Max-Age=2592000; HttpOnly; Secure; SameSite=None",
        )
        .expect("the session is passed on");
        assert_eq!(
            resettled.to_str().unwrap(),
            // The root, because the page asks under paths of its own; no domain, so it belongs to
            // whoever served it.
            "auth=s3cr3t; Path=/; Max-Age=2592000; HttpOnly; Secure; SameSite=None"
        );
    }

    /// Whatever else the sign-in sets is not the session and is nothing to hand on.
    #[test]
    fn nothing_but_the_session_is_passed_on() {
        assert!(super::resettled("othercookie=whatever; Path=/").is_none());
    }

    /// The page's origin may carry cookies that have nothing to do with YNOproject.
    #[test]
    fn only_the_session_is_sent_upstream() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "theme=dark; auth=s3cr3t; language=ja".parse().unwrap(),
        );
        assert_eq!(
            super::session(&headers).unwrap().to_str().unwrap(),
            "auth=s3cr3t"
        );
    }
}
