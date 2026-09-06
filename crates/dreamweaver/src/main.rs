//! Serves yumezu its world dump.
//!
//! The wiki explorer this replaces keeps a MySQL database, a scraper for a dozen wiki pages and a
//! background worker to run them; this keeps one JSON document. It is smaller because it asks the
//! wiki's Semantic MediaWiki store for structured data instead of reading the wiki's HTML -- see
//! [`smw`] -- and because a dump that is rebuilt from scratch every time has nothing to reconcile.
//!
//! Not everything the reference dump carries can be had that way. Effects, menu themes, wallpapers
//! and the soundtrack are written in wiki prose, and the fields for them are published empty --
//! see [`model::Dump`]. Nothing here parses HTML, and nothing here should: none of those four says
//! anything about how the worlds join up, which is the whole of what this dump is read for.
//!
//! There is nothing to run but the server, and nothing to tell it to do. It keeps the dump current
//! out of the requests for it: a `GET /data` that arrives more than `--sync-every` hours after the
//! wiki was last asked about starts a sync and answers `503 needs update` instead of the dump, and
//! the client waits that sync out on `POST /pollUpdate` before asking again. Every other `GET
//! /data` is answered from the file. See [`Due`] and [`data`].
//!
//! A sync asks the wiki what has been edited since the dump was built and rebuilds out of the
//! answer, re-reading only the parts of the wiki those pages belong to -- see [`sync::Fetched`].
//! A run with no dump to compare against reads all of it.
//!
//! ```text
//! dreamweaver [--listen ADDR|PATH] [--data PATH] [--sync-every HOURS]
//! ```
//!
//! `--listen` takes either a `host:port` or, so that nginx can reach it the other way its
//! `proxy_pass` knows, the path of a Unix socket -- see [`Listen`].

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};

mod depth;
mod model;
mod progress;
mod smw;
mod store;
mod sync;
mod versions;

/// Where the dump is kept, and so what a run with no `--data` reads and writes.
const DATA: &str = "data.json";

/// Where the server listens with no `--listen`.
const LISTEN: &str = "127.0.0.1:5000";

/// How often a run looks for something to do, with no `--sync-every`.
///
/// A world appears every few days at most, so this is not a race. Six hours is the reference
/// implementation's own interval, and most of these passes cost one small request and stop --
/// see [`Sync::Soft`]. It is also the window the wiki is asked about, so a shorter interval means
/// more passes each covering less, not more of the wiki read.
const SYNC_EVERY: u64 = 6;

/// What the server hands to its handlers: the published dump, the HTTP client the wiki is asked
/// through, and what the last refresh already fetched.
///
/// The lock around that last part is also what keeps two refreshes from running at once, which
/// matters more than the cache does: a scheduled sync and a `POST /update` arriving together
/// would otherwise both build a dump and the slower one would publish over the newer.
#[derive(Clone)]
struct Server {
    store: Arc<store::Store>,
    http: reqwest::Client,
    fetched: Arc<tokio::sync::Mutex<sync::Fetched>>,
    /// Where the running sync has got to. See [`progress`].
    progress: Arc<progress::Progress>,
    /// Whether the wiki is worth asking about again. See [`Due`].
    due: Arc<Due>,
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

    let store = Arc::new(store::Store::open(&options.data));
    // A run that read a dump off disk is as up to date as that dump says it is, so it is not due
    // a sync the moment it comes up. One that read nothing is.
    let due = Arc::new(Due::new(options.sync_every, &store.snapshot().dump));
    let server = Server {
        store,
        http: reqwest::Client::new(),
        fetched: Arc::default(),
        progress: Arc::default(),
        due,
    };
    serve(server, options).await;
    std::process::ExitCode::SUCCESS
}

const USAGE: &str = "dreamweaver [--listen ADDR|PATH] [--data PATH] [--sync-every HOURS]";

/// The switches, and their defaults.
struct Options {
    /// A `host:port` or a socket path; [`Listen`] is which.
    listen: String,
    data: String,
    /// How long a sync stands for. See [`Due`].
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
    /// Reads `--switch value` pairs. Hand-rolled because there are three of them, and a parser
    /// crate would be more configuration than the thing it configures.
    fn read(&mut self, args: impl Iterator<Item = String>) -> Result<(), String> {
        let mut args = args;
        while let Some(switch) = args.next() {
            let mut value = || args.next().ok_or(format!("{switch} wants a value"));
            match switch.as_str() {
                "--listen" => self.listen = value()?,
                "--data" => self.data = value()?,
                "--sync-every" => {
                    // Refused rather than clamped, and refused here rather than found out later:
                    // zero hours is a sync that is due again the moment it ends, and a server
                    // whose dump is always due never serves it at all. See [`Due`] and [`data`].
                    self.sync_every = match value()?.parse() {
                        Ok(hours) if hours > 0 => hours,
                        _ => return Err("--sync-every wants a whole number of hours, at least one"
                            .to_owned()),
                    }
                }
                other => return Err(format!("no such switch: {other}")),
            }
        }
        Ok(())
    }
}

/// Whether the wiki is worth asking about again, and so whether the next `GET /data` is answered
/// with the dump or with `needs update`.
///
/// The mark is when a sync last *ran*, not when the dump last changed. Most syncs find the wiki
/// unmoved and publish nothing -- see [`plan`] -- so dating this from the dump's own stamp would
/// leave every one of those due again immediately, and a busy server would ask the wiki about
/// every request it got.
///
/// A run that came up with a dump on disk inherits that dump's stamp, so restarting the server is
/// not a way to make it re-read the wiki, and a host that restarts it hourly does not read the
/// wiki hourly.
struct Due {
    /// When the wiki was last asked, or `None` for a server that has never asked it.
    asked: std::sync::Mutex<Option<time::OffsetDateTime>>,
    /// How long that answer stands for: `--sync-every`.
    every: time::Duration,
}

impl Due {
    fn new(hours: u64, previous: &model::Dump) -> Self {
        Due {
            asked: std::sync::Mutex::new(previous.last_update.as_deref().and_then(sync::moment)),
            every: time::Duration::hours(hours as i64),
        }
    }

    /// Whether a sync is due now.
    fn now(&self) -> bool {
        match *self.asked.lock().unwrap() {
            Some(asked) => time::OffsetDateTime::now_utc() - asked >= self.every,
            None => true,
        }
    }

    /// Says the wiki has just been asked, whatever came of it.
    ///
    /// A failed sync counts. The wiki being unreachable is not something a tighter loop fixes,
    /// and a server that retried every request would answer `needs update` to all of them while
    /// hammering a host that is already having a bad day.
    fn met(&self) {
        *self.asked.lock().unwrap() = Some(time::OffsetDateTime::now_utc());
    }
}

/// Starts a sync, unless one is already running.
///
/// The lock the sync holds is taken here, so "is one running" and "what keeps two from running"
/// are one fact rather than two that can disagree. A caller that does not get it has nothing to
/// do: the sync already running is the one it wanted started.
///
/// `Ok(None)` from [`build`] means the wiki had nothing to say, which is not a failure and not a
/// dump: the one already published is still the right one, down to the byte.
fn start(server: &Server) {
    let Ok(mut fetched) = server.fetched.clone().try_lock_owned() else {
        return;
    };
    let server = server.clone();
    tokio::spawn(async move {
        let built = build(&server, &mut fetched).await;
        // However it went: the wiki has been asked, and nothing is being fetched any more.
        server.due.met();
        server.progress.done();
        match built {
            Ok(Some(worlds)) => tracing::info!("published {worlds} worlds"),
            Ok(None) => tracing::info!("the wiki has not changed; the dump stands"),
            // Debug rather than Display: `reqwest` names the request either way, but what it
            // will not put in a line is which field of the answer it could not read, and on a
            // failed sync that is the whole of what there is to go on.
            Err(error) => tracing::error!("sync failed: {error:?}"),
        }
    });
}

/// The work [`start`] reports on. Split out so that every way out of it -- a sync with nothing to
/// do, a failed fetch, a published dump -- passes back through the same clearing up.
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
/// A sync with a dump to compare against is the only one with a choice to make. It asks the wiki
/// which pages have moved since that dump was built -- a little before that, in fact, since the
/// store is indexed after the fact and the question is worth overlapping. Three answers stand the
/// sync down or widen it: nothing has changed, so there is nothing to build; the dump is old
/// enough that the wiki no longer remembers that far back, so the honest answer is to read all of
/// it; and the wiki cannot be asked at all, likewise.
async fn plan(server: &Server, previous: &model::Dump) -> Option<sync::Refresh> {
    server.progress.at(progress::CHANGES);
    // Nothing to compare against is a first sync, and a first sync reads all of it.
    let Some(built) = previous.last_update.as_deref() else {
        return Some(sync::Refresh::Everything);
    };
    if previous.worlds.is_empty() {
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
        // A gate that cannot be read is no reason to keep serving an old dump. The cost of
        // rebuilding one that did not need it is a minute of asking; the cost of the other
        // mistake is a dump that quietly stops following the wiki.
        Err(error) => {
            tracing::warn!("cannot tell what the wiki has changed: {error}");
            Some(sync::Refresh::Everything)
        }
    }
}

/// What `--listen` named.
///
/// nginx reaches an upstream by `proxy_pass http://127.0.0.1:5000` or by
/// `proxy_pass http://unix:/run/dreamweaver.sock:`, and which of the two a host wants is the
/// host's business: a socket in a directory only nginx and this program can enter needs no
/// loopback port left open, which for a server whose `/update` is unguarded is the safer half.
///
/// The two are told apart by the `/`, since no `host:port` has one -- not even an IPv6 literal,
/// which brackets its colons instead.
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

/// Runs the server until it is asked to stop.
async fn serve(server: Server, options: Options) {
    let app = axum::Router::new()
        // The two names the same document answers to: `/data` is what the reference
        // implementation serves it as, and `/data.json` is what a build script would rather save.
        .route("/data", get(data))
        .route("/data.json", get(data))
        .route("/pollUpdate", post(poll_update))
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
            // A socket outlives the process that bound it, and the one left behind is both a
            // door that answers nothing and the file the next run has to clear before it can
            // bind. Tidying up here covers the graceful stop; `bind` covers every other ending.
            if let Err(error) = std::fs::remove_file(path) {
                tracing::warn!("cannot remove {}: {error}", path.display());
            }
        }
    }
}

/// Opens the socket `--listen` named, clearing a stale one out of the way.
///
/// The socket is made world-reachable. The permissions on a socket that arrived with the default
/// umask would let nothing but this program's own user connect, which is not what a host putting
/// nginx in front of it is asking for, and nginx's own user is not something this program can
/// guess. What decides who may connect is the directory the socket sits in -- which is the usual
/// arrangement, and the one the host is already making when it chooses the path.
fn bind(path: &Path) -> std::io::Result<tokio::net::UnixListener> {
    // Only a socket, and only one nothing is listening on: `bind` fails with "address in use"
    // against a live one, which is the right answer for a second copy started by mistake, and
    // refusing to unlink anything else keeps a mistyped `--listen` from eating a real file.
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

/// `GET /data` -- the dump, exactly as it sits on disk, or `503 needs update`.
///
/// The second is what makes the server keep up at all: there is no clock in here, only requests,
/// and a request arriving after the last sync has gone stale is what starts the next one. The
/// client is told to wait rather than handed the old dump so that it has one story for both of
/// the waits it can meet -- the first sync of a server with nothing to serve, and a routine
/// refresh -- and `POST /pollUpdate` is how it waits: poll it until `done`, then ask again.
///
/// An empty dump is never served. A client cannot tell it from a wiki with no worlds in it and
/// would draw the second, so a server that has never built one answers `needs update` however
/// recently it last tried.
async fn data(State(server): State<Server>) -> axum::response::Response {
    let snapshot = server.store.snapshot();
    if server.due.now() || snapshot.dump.worlds.is_empty() {
        // Nothing is awaited: the sync outlives this request, which is the whole point of
        // answering rather than holding the connection open for the minute it takes.
        start(&server);
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

/// `POST /pollUpdate` -- what the sync is doing, in the reference implementation's own shape.
///
/// `done` is that no sync is running, not that there is a dump: a server between syncs answers
/// `{"task": null, "done": true}` whether or not it has ever built one. See [`progress`].
async fn poll_update(State(server): State<Server>) -> impl IntoResponse {
    let task = server.progress.task();
    axum::Json(serde_json::json!({ "task": task, "done": task.is_none() }))
}
