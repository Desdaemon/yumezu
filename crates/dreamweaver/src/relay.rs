//! YNOproject's account routes relayed for the web build, which YNOproject's origin rule blocks;
//! a fixed route list that rehosts the session cookie. Native builds call YNOproject directly.

use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

/// YNOproject's Yume 2kki API; one host per game.
const API: &str = "https://api.ynoproject.net/2kki/api";
/// The sign-in service, separate from [`API`].
const AUTH: &str = "https://auth.ynoproject.net";
/// The cookie YNOproject's sign-in issues, and the only one carried in either direction.
const SESSION: &str = "auth";

/// Merged into the server's own router; stateless.
pub fn routes<S: Clone + Send + Sync + 'static>() -> axum::Router<S> {
    axum::Router::new()
        .route("/yno/info", get(info))
        .route("/yno/gamelocations", get(locations))
        .route("/ynoauth/login", post(login))
        .route("/ynoauth/forget", post(forget))
}

/// `GET /yno/info` -- who the session belongs to and every location they have visited; the only
/// route that sends the session.
async fn info(headers: HeaderMap) -> Response {
    forward(&format!("{API}/info"), session(&headers)).await
}

/// `GET /yno/gamelocations` -- every location YNOproject knows, by id; fetched without a session.
async fn locations() -> Response {
    forward(&format!("{API}/gamelocations"), None).await
}

/// `POST /ynoauth/login` -- a name and a password for a session. Request and response pass through
/// unread, but for the session cookie: see [`resettled`].
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

/// `POST /ynoauth/forget` -- clears this origin's copy of the session. YNOproject's sign-out is
/// left alone, so the user stays signed in to the game in another browser tab.
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

/// Status and body passed through unchanged, so the page reports upstream failures.
async fn forward(url: &str, session: Option<HeaderValue>) -> Response {
    let mut request = client().get(url);
    if let Some(session) = session {
        request = request.header(header::COOKIE, session);
    }
    match request.send().await {
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

/// The [`SESSION`] cookie alone; the page's origin may carry unrelated cookies.
fn session(headers: &HeaderMap) -> Option<HeaderValue> {
    let sent = headers.get(header::COOKIE)?.to_str().ok()?;
    let session = sent
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| name.trim() == SESSION)
        .map(|(_, session)| session.trim())?;
    HeaderValue::from_str(&format!("{SESSION}={session}")).ok()
}

/// YNOproject's session `Set-Cookie`, rehosted: `Domain` dropped so the cookie belongs to this
/// origin, `Path` forced to `/`, the rest as written. `None` for any other cookie.
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

/// Built once and cloned, so every request shares one connection pool.
fn client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
            crate::http_client(builder)
        })
        .clone()
}

#[cfg(test)]
mod tests {
    /// A browser keeps a cookie only for the serving domain.
    #[test]
    fn a_session_is_resettled_on_whoever_served_it() {
        let resettled = super::resettled(
            "auth=s3cr3t; Path=/login; Domain=.ynoproject.net; Max-Age=2592000; HttpOnly; Secure; SameSite=None",
        )
        .expect("the session is passed on");
        assert_eq!(
            resettled.to_str().unwrap(),
            // No `Domain`: the serving origin owns the cookie.
            "auth=s3cr3t; Path=/; Max-Age=2592000; HttpOnly; Secure; SameSite=None"
        );
    }

    /// Cookies other than the session are dropped.
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
