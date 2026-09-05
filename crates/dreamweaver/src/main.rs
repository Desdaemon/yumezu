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
//! There is nothing to run but the server, and nothing to tell it to do. It builds the dump when
//! it comes up and keeps it current by itself: every few hours it asks the wiki what has been
//! edited since the dump was built, and rebuilds out of the answer -- reading again only the parts
//! of the wiki those pages belong to. See [`Sync`] and [`sync::Fetched`]. Rebuilding all of it
//! regardless is the one thing it waits to be asked for, and `POST /update` is the asking.
//!
//! ```text
//! dreamweaver [--listen ADDR] [--data PATH] [--sync-every HOURS]
//! ```

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};

mod depth;
mod model;
mod smw;
mod store;
mod sync;
mod versions;
mod wiki;

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

/// What the server hands to its handlers: the published dump, the client that refreshes it, and
/// what the last refresh already fetched.
///
/// The lock around that last part is also what keeps two refreshes from running at once, which
/// matters more than the cache does: a scheduled sync and a `POST /update` arriving together
/// would otherwise both build a dump and the slower one would publish over the newer.
#[derive(Clone)]
struct Server {
    store: Arc<store::Store>,
    wiki: wiki::Client,
    fetched: Arc<tokio::sync::Mutex<sync::Fetched>>,
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
        wiki: wiki::Client::new(),
        fetched: Arc::default(),
    };
    serve(server, options).await;
    std::process::ExitCode::SUCCESS
}

const USAGE: &str = "dreamweaver [--listen ADDR] [--data PATH] [--sync-every HOURS]";

/// The switches, and their defaults.
struct Options {
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
                    self.sync_every = value()?
                        .parse()
                        .map_err(|_| "--sync-every wants a whole number of hours".to_owned())?
                }
                other => return Err(format!("no such switch: {other}")),
            }
        }
        Ok(())
    }
}

/// How hard a refresh tries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sync {
    /// Ask the wiki what has changed, and rebuild out of the answer.
    ///
    /// Most passes end at that first request with nothing to do, which on a wiki whose worlds
    /// arrive weekly is most of them. A pass that does have something to do re-reads the parts of
    /// the wiki the edited pages belong to and keeps the rest of the last fetch -- see
    /// [`sync::Fetched`] -- so a week's editing costs a couple of requests rather than thirty.
    Soft,
    /// Rebuild, whatever the wiki says about itself.
    ///
    /// What a run does when it comes up, since there may be no dump at all to keep, and what
    /// `POST /update` asks for.
    Full,
}

/// Fetches a new dump and publishes it.
///
/// `Ok(None)` means a soft sync found nothing to do, which is not a failure and not a dump: the
/// one already published is still the right one, down to the byte.
async fn refresh(server: &Server, sync: Sync) -> wiki::Result<Option<usize>> {
    let mut fetched = server.fetched.lock().await;
    let previous = server.store.snapshot();
    let plan = match plan(server, sync, &previous.dump).await {
        Some(plan) => plan,
        None => return Ok(None),
    };
    let dump = sync::run(&server.wiki, &previous.dump, plan, &mut fetched).await?;
    Ok(Some(server.store.publish(dump).dump.worlds.len()))
}

/// How much of the wiki this refresh should read, or `None` for one that need not run at all.
///
/// A soft sync is the only one with a choice to make. It asks the wiki which pages have moved
/// since the dump was built -- a little before that, in fact, since the store is indexed after the
/// fact and the question is worth overlapping. Three answers stand the sync down or widen it:
/// nothing has changed, so there is nothing to build; the dump is old enough that the wiki no
/// longer remembers that far back, so the honest answer is to read all of it; and the wiki cannot
/// be asked at all, likewise.
async fn plan(server: &Server, sync: Sync, previous: &model::Dump) -> Option<sync::Refresh> {
    // Nothing to compare against is a first sync, and a first sync is a full one however it was
    // asked for.
    let (Sync::Soft, Some(built)) = (sync, previous.last_update.as_deref()) else {
        return Some(sync::Refresh::Everything);
    };
    if previous.worlds.is_empty() {
        return Some(sync::Refresh::Everything);
    }
    let Some(since) = sync::asked_from(built, time::OffsetDateTime::now_utc()) else {
        tracing::info!("the dump is older than the wiki's memory of what it changed; reading all");
        return Some(sync::Refresh::Everything);
    };
    match server.wiki.changed_since(&since).await {
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

/// Runs the server until it is asked to stop.
async fn serve(server: Server, options: Options) {
    let app = axum::Router::new()
        // The two names the same document answers to: `/data` is what the reference
        // implementation serves it as, and `/data.json` is what a build script would rather save.
        .route("/data", get(data))
        .route("/data.json", get(data))
        .route("/update", post(update))
        .with_state(server.clone());

    let listener = match tokio::net::TcpListener::bind(&options.listen).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!("cannot listen on {}: {error}", options.listen);
            return;
        }
    };
    tracing::info!("listening on http://{}", options.listen);

    // The refresher runs alongside rather than inside a request, so a client is never made to
    // wait for the wiki and a wiki that is down costs nothing but a stale dump. It is also why
    // the port is open before the first sync: a run that comes up with a dump on disk serves it
    // straight away rather than after several minutes of paging.
    tokio::spawn(refresher(
        server,
        Duration::from_secs(options.sync_every * 3600),
    ));

    if let Err(error) = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            _ = tokio::signal::ctrl_c().await;
        })
        .await
    {
        tracing::error!("stopped serving: {error}");
    }
}

/// Keeps the dump current, for as long as the server runs.
///
/// The first pass is a full one, because a run that has just come up does not know how old what it
/// read off disk is, and may have read nothing at all. Every pass after it is soft, and most of
/// them end at the first request with the wiki saying nothing has changed.
///
/// A failed refresh is logged and waited out rather than retried straight away: the wiki being
/// unreachable is not something a tighter loop fixes, and the dump already being served stays
/// perfectly good in the meantime.
async fn refresher(server: Server, every: Duration) {
    let mut sync = Sync::Full;
    loop {
        match refresh(&server, sync).await {
            Ok(Some(worlds)) => tracing::info!("published {worlds} worlds"),
            Ok(None) => tracing::info!("the wiki has not changed; the dump stands"),
            Err(error) => tracing::error!("sync failed: {error}"),
        }
        sync = Sync::Soft;
        tokio::time::sleep(every).await;
    }
}

/// `GET /data` -- the dump, exactly as it sits on disk.
async fn data(State(server): State<Server>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        server.store.snapshot().json.to_string(),
    )
}

/// `POST /update` -- rebuild now, and say how it went.
///
/// A full sync, not a soft one: this is the one way to have the dump rebuilt without the wiki
/// first agreeing that it needs to be, which is what makes it worth having at all. Everything else
/// the server does to keep up it does on its own.
///
/// Unguarded, and so meant for a host that is not exposing this port to the world. It costs the
/// wiki a few dozen requests and cannot damage anything: the worst a caller can do is make the
/// server fetch a dump it was going to fetch anyway.
async fn update(State(server): State<Server>) -> impl IntoResponse {
    match refresh(&server, Sync::Full).await {
        Ok(worlds) => (
            StatusCode::OK,
            format!("published {} worlds\n", worlds.unwrap_or_default()),
        ),
        Err(error) => (StatusCode::BAD_GATEWAY, format!("sync failed: {error}\n")),
    }
}
