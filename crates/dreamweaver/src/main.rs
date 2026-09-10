//! Serves yumezu its world dump.
//!
//! The wiki explorer this replaces keeps a MySQL database, a scraper for a dozen wiki pages and a
//! background worker; this keeps one JSON document. It asks the wiki's Semantic MediaWiki store for
//! structured data instead of reading HTML -- see [`smw`] -- and a dump rebuilt from scratch every
//! time has nothing to reconcile.
//!
//! Effects, menu themes, wallpapers and the soundtrack are written in wiki prose, so their fields
//! are published empty. Nothing here parses HTML, and nothing here should: none of those four says
//! anything about how the worlds join up.
//!
//! The dump is kept current on the server's own clock: a sync runs every `--sync-every` hours, and
//! `GET /data` is answered from the file whether or not one is running. A sync re-reads only the
//! parts of the wiki whose pages have been edited; once a week one reads the whole of it instead
//! of asking what has changed -- see [`FULL_EVERY`]. Until the first sync lands there is nothing to
//! serve, so `/data` answers `503 needs update` and the client waits on `GET /pollUpdate`.
//!
//! ```text
//! dreamweaver [--listen ADDR|PATH] [--data PATH] [--sync-every HOURS]
//! ```
//!
//! Besides the dump it answers four routes carrying what YNOproject knows about the player reading
//! the page, put through because the page may not ask YNOproject directly. See [`relay`].
//!
//! `--listen` takes either a `host:port` or, so that nginx can reach it the other way its
//! `proxy_pass` knows, the path of a Unix socket -- see [`Listen`].

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;

mod depth;
mod model;
mod progress;
/// What the page asks about a player's own YNOproject account, put through to YNOproject. See
/// [`relay::routes`].
mod relay;
mod smw;
mod store;
mod sync;
mod versions;

/// Where the dump is kept, and so what a run with no `--data` reads and writes.
const DATA: &str = "data.json";

/// Where the server listens with no `--listen`.
const LISTEN: &str = "127.0.0.1:5000";

/// A pass that finds the wiki unmoved costs one small request, so what sets this is how soon an edit
/// should show rather than what the asking costs. It is also the window the wiki is asked about: a
/// shorter interval means more passes each covering less, not more of the wiki read.
const SYNC_EVERY: u64 = 2;

/// A soft sync takes the wiki's account of its own edits at its word, and an edit that account
/// misses -- a template the worlds are built out of, outside the one namespace that is watched --
/// is missed for good: nothing later asks about that week again.
const FULL_EVERY: time::Duration = time::Duration::weeks(1);

#[derive(Clone)]
struct Server {
    store: Arc<store::Store>,
    http: reqwest::Client,
    /// Where the running sync has got to. See [`progress`].
    progress: Arc<progress::Progress>,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dreamweaver=info".into()),
        )
        .init();

    let mut options = Options::default();
    if let Err(complaint) = options.read(std::env::args().skip(1)) {
        eprintln!("{complaint}\n\n{USAGE}");
        return std::process::ExitCode::FAILURE;
    }

    let server = Server {
        store: Arc::new(store::Store::open(&options.data)),
        http: http(),
        progress: Arc::default(),
    };
    tokio::spawn(refresh(server.clone(), options.sync_every));
    serve(server, options).await;
    std::process::ExitCode::SUCCESS
}

/// The client the wiki is asked over.
fn http() -> reqwest::Client {
    // `rustls-no-provider` leaves the provider to the process, and reqwest panics building a
    // client without one.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
}

const USAGE: &str = "dreamweaver [--listen ADDR|PATH] [--data PATH] [--sync-every HOURS]";

/// The switches, and their defaults.
struct Options {
    /// A `host:port` or a socket path; [`Listen`] is which.
    listen: String,
    data: String,
    sync_every: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            listen: LISTEN.to_owned(),
            data: DATA.to_owned(),
            sync_every: SYNC_EVERY,
        }
    }
}

impl Options {
    /// Hand-rolled: three switches would take more configuring through a parser crate than they
    /// configure.
    fn read(&mut self, args: impl Iterator<Item = String>) -> Result<(), String> {
        let mut args = args;
        while let Some(switch) = args.next() {
            let mut value = || args.next().ok_or(format!("{switch} wants a value"));
            match switch.as_str() {
                "--listen" => self.listen = value()?,
                "--data" => self.data = value()?,
                "--sync-every" => {
                    // Zero hours is a sync due again the moment it ends, and a server whose dump
                    // is always due never serves it at all.
                    self.sync_every = match value()?.parse() {
                        Ok(hours) if hours > 0 => hours,
                        _ => {
                            return Err("--sync-every wants a whole number of hours, at least one"
                                .to_owned());
                        }
                    }
                }
                other => return Err(format!("no such switch: {other}")),
            }
        }
        Ok(())
    }
}

/// Keeps the dump current, for as long as the server runs: one sync every `--sync-every` hours.
///
/// Each sync is awaited before the next interval begins. Two at once would both build a dump, and
/// the slower would publish over the newer.
///
/// The first is timed from the dump on disk rather than from startup, so restarting the server is
/// not a way to make it re-read the wiki.
async fn refresh(server: Server, hours: u64) {
    let every = std::time::Duration::from_secs(hours * 3600);
    tokio::time::sleep(first_sync(&server.store.snapshot().dump, every)).await;
    let mut fetched = sync::Fetched::default();
    loop {
        match build(&server, &mut fetched).await {
            Ok(Some(worlds)) => tracing::info!("published {worlds} worlds"),
            Ok(None) => tracing::info!("the wiki has not changed; the dump stands"),
            // Debug rather than Display: only Debug says which field of the answer it could not
            // read, which on a failed sync is the whole of what there is to go on.
            Err(error) => tracing::error!("sync failed: {error:?}"),
        }
        server.progress.done();
        tokio::time::sleep(every).await;
    }
}

/// How long a run waits before its first sync: whatever is left of the interval the dump it came up
/// with was built in, and nothing at all for a run that came up with no dump.
fn first_sync(previous: &model::Dump, every: std::time::Duration) -> std::time::Duration {
    let Some(built) = previous.last_update.as_deref().and_then(sync::moment) else {
        return std::time::Duration::ZERO;
    };
    let age = time::OffsetDateTime::now_utc() - built;
    // A stamp in the future has no age. Waiting the interval out is the reading that cannot become
    // a sync per restart.
    every.saturating_sub(age.try_into().unwrap_or(every))
}

/// Split out so every way out -- nothing to do, a failed fetch, a published dump -- passes back
/// through the same clearing up.
async fn build(server: &Server, fetched: &mut sync::Fetched) -> smw::Result<Option<usize>> {
    let previous = server.store.snapshot();
    let plan = match plan(server, &previous.dump).await {
        Some(plan) => plan,
        None => return Ok(None),
    };
    let dump = sync::run(
        &server.http,
        &previous.dump,
        plan,
        fetched,
        &server.progress,
    )
    .await?;
    Ok(Some(server.store.publish(dump).dump.worlds.len()))
}

/// How much of the wiki this refresh should read, or `None` for one that need not run at all.
///
/// Only a sync with a dump to compare against, and a week not yet up, has a choice to make. Three
/// answers stand it down or widen it: nothing has changed; the dump is older than the wiki
/// remembers, so all of it is read; and the wiki cannot be asked at all, which reads all of it too.
async fn plan(server: &Server, previous: &model::Dump) -> Option<sync::Refresh> {
    server.progress.at(progress::CHANGES);
    // Nothing to compare against is a first sync, and a first sync reads all of it.
    let Some(built) = previous
        .last_update
        .as_deref()
        .filter(|_| !previous.worlds.is_empty())
    else {
        return Some(sync::Refresh::Everything);
    };
    if full_due(previous) {
        tracing::info!("the week is up; reading the whole wiki rather than asking what changed");
        return Some(sync::Refresh::Everything);
    }
    let Some(since) = sync::asked_from(built, time::OffsetDateTime::now_utc()) else {
        tracing::info!("the dump is older than the wiki's memory of what it changed; reading all");
        return Some(sync::Refresh::Everything);
    };
    match smw::changed_since(&server.http, &since).await {
        Ok(pages) if pages.is_empty() => None,
        Ok(pages) => {
            tracing::info!("{} pages edited since {since}", pages.len());
            Some(sync::Refresh::Pages(pages))
        }
        // Rebuilding a dump that did not need it costs a minute of asking; the other mistake is a
        // dump that quietly stops following the wiki.
        Err(error) => {
            tracing::warn!("cannot tell what the wiki has changed: {error}");
            Some(sync::Refresh::Everything)
        }
    }
}

/// `lastFullUpdate` is the dump's own record of when the whole wiki was last read, so the week
/// survives a restart.
fn full_due(previous: &model::Dump) -> bool {
    match previous.last_full_update.as_deref().and_then(sync::moment) {
        Some(last) => time::OffsetDateTime::now_utc() - last >= FULL_EVERY,
        None => true,
    }
}

/// nginx reaches an upstream by `proxy_pass http://127.0.0.1:5000` or by
/// `proxy_pass http://unix:/run/dreamweaver.sock:`. A socket in a directory only nginx and this
/// program can enter needs no loopback port left open.
///
/// The two are told apart by the `/`, no `host:port` having one -- not even an IPv6 literal, which
/// brackets its colons instead.
enum Listen<'a> {
    Tcp(&'a str),
    Unix(&'a Path),
}

impl<'a> Listen<'a> {
    fn read(listen: &'a str) -> Self {
        match listen.contains('/') {
            true => Listen::Unix(Path::new(listen)),
            false => Listen::Tcp(listen),
        }
    }
}

/// Runs until asked to stop.
async fn serve(server: Server, options: Options) {
    let app = axum::Router::new()
        // `/data` is what the reference implementation serves it as; `/data.json` is what a build
        // script would rather save.
        .route("/data", get(data))
        .route("/data.json", get(data))
        .route("/pollUpdate", get(poll_update))
        // Kept here rather than left to whatever serves the page: the sign-in's cookie has to be
        // handed back for this origin to be keepable at all.
        .merge(relay::routes())
        .with_state(server);

    match Listen::read(&options.listen) {
        Listen::Tcp(address) => {
            let listener = match tokio::net::TcpListener::bind(address).await {
                Ok(listener) => listener,
                Err(error) => {
                    tracing::error!("cannot listen on {address}: {error}");
                    return;
                }
            };
            tracing::info!("listening on http://{address}");
            run(listener, app).await;
        }
        Listen::Unix(path) => {
            let listener = match bind(path) {
                Ok(listener) => listener,
                Err(error) => {
                    tracing::error!("cannot listen on {}: {error}", path.display());
                    return;
                }
            };
            tracing::info!("listening on unix:{}", path.display());
            run(listener, app).await;
            // A socket outlives the process that bound it, and the one left behind is both a door
            // that answers nothing and the file the next run must clear. This covers the graceful
            // stop; `bind` covers every other ending.
            if let Err(error) = std::fs::remove_file(path) {
                tracing::warn!("cannot remove {}: {error}", path.display());
            }
        }
    }
}

/// Opens the socket `--listen` named, clearing a stale one out of the way.
///
/// Made world-reachable: the default umask would let nothing but this program's own user connect,
/// and nginx's user is not something this program can guess. What decides who may connect is the
/// directory the socket sits in, which the host chooses along with the path.
fn bind(path: &Path) -> std::io::Result<tokio::net::UnixListener> {
    // Only a socket, and only one nothing is listening on: `bind` fails with "address in use"
    // against a live one, and refusing to unlink anything else keeps a mistyped `--listen` from
    // eating a real file.
    if std::os::unix::net::UnixStream::connect(path).is_err()
        && std::fs::symlink_metadata(path).is_ok_and(|file| file.file_type().is_socket())
    {
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
    Ok(listener)
}

/// Serves until Ctrl-C, whichever kind of door the requests come through.
async fn run<L>(listener: L, app: axum::Router)
where
    L: axum::serve::Listener,
    L::Addr: std::fmt::Debug,
{
    if let Err(error) = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            _ = tokio::signal::ctrl_c().await;
        })
        .await
    {
        tracing::error!("stopped serving: {error}");
    }
}

/// `GET /data` -- the dump, exactly as it sits on disk, or `503 needs update` before there is one.
///
/// An empty dump is never served: a client cannot tell it from a wiki with no worlds in it and
/// would draw the second. A sync under way is no reason to withhold the dump standing: on a pass
/// that publishes nothing, that is the same document the client would be handed a minute later.
async fn data(State(server): State<Server>) -> axum::response::Response {
    let snapshot = server.store.snapshot();
    if snapshot.dump.worlds.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, "5")],
            "needs update\n",
        )
            .into_response();
    }

    (
        [(header::CONTENT_TYPE, "application/json")],
        snapshot.json.to_string(),
    )
        .into_response()
}

/// `GET /pollUpdate` -- what the sync is doing, in the reference implementation's own JSON.
///
/// `done` means no sync is running, not that there is a dump: a server between syncs answers
/// `{"task": null, "done": true}` whether or not it has ever built one.
async fn poll_update(State(server): State<Server>) -> impl IntoResponse {
    let task = server.progress.task();
    axum::Json(serde_json::json!({ "task": task, "done": task.is_none() }))
}

#[cfg(test)]
mod tests {
    /// `rustls-no-provider` moves the choice of provider out of reqwest's features and into
    /// [`super::http`], where nothing but a run can prove it was made: the builder panics instead
    /// of failing to compile.
    #[test]
    fn the_client_has_a_crypto_provider() {
        super::http();
    }
}
