//! Serves yumezu the world dump, built from yume.wiki's Semantic MediaWiki store, plus the
//! thumbnail atlas and a YNOproject relay; resyncs every `--sync-every` hours, fully once a week.

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;

mod atlas;
mod depth;
mod model;
mod next;
mod progress;
/// The web build's YNOproject account routes, relayed to YNOproject; see [`relay::routes`].
mod relay;
mod smw;
mod store;
mod sync;
mod versions;

const DEFAULT_DATA: &str = "data.json";

const DEFAULT_ATLAS: &str = "atlas";

const DEFAULT_LISTEN: &str = "127.0.0.1:5000";

/// A sync that finds no edits costs one small request, so this is how soon an edit shows. A
/// shorter interval splits the same wiki reading across more, smaller syncs.
const SYNC_EVERY: u64 = 2;

/// A soft sync trusts the wiki's recent-changes list, which misses edits to templates outside the
/// Yume 2kki namespace; the weekly full sync catches those.
const FULL_EVERY: time::Duration = time::Duration::weeks(1);

#[derive(Clone)]
pub struct Server {
    store: Arc<store::Store>,
    atlas: Arc<atlas::Atlas>,
    http: reqwest::Client,
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
        atlas: Arc::new(atlas::Atlas::open(&options.atlas)),
        http: http_client(reqwest::Client::builder()),
        progress: Arc::default(),
    };
    tokio::spawn(refresh(server.clone(), options.sync_every));
    serve(server, options).await;
    std::process::ExitCode::SUCCESS
}

/// reqwest panics without a process-wide provider, so every client is built here.
fn http_client(builder: reqwest::ClientBuilder) -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    builder.build().expect("cannot build an http client")
}

const USAGE: &str =
    "dreamweaver [--listen ADDR|PATH] [--data PATH] [--atlas DIR] [--sync-every HOURS]";

struct Options {
    /// Either `host:port` or a socket path; see [`Listen`].
    listen: String,
    data: String,
    /// Holds the packed atlas and the source wiki pictures.
    atlas: String,
    sync_every: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            listen: DEFAULT_LISTEN.to_owned(),
            data: DEFAULT_DATA.to_owned(),
            atlas: DEFAULT_ATLAS.to_owned(),
            sync_every: SYNC_EVERY,
        }
    }
}

impl Options {
    /// Hand-rolled: four switches do not justify a parser crate.
    fn read(&mut self, args: impl Iterator<Item = String>) -> Result<(), String> {
        let mut args = args;
        while let Some(switch) = args.next() {
            let mut value = || args.next().ok_or(format!("{switch} wants a value"));
            match switch.as_str() {
                "--listen" => self.listen = value()?,
                "--data" => self.data = value()?,
                "--atlas" => self.atlas = value()?,
                "--sync-every" => {
                    // Zero hours would sync forever and never serve.
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

/// One sync every `--sync-every` hours, each awaited so a slow sync cannot overwrite a newer one.
/// The first is timed from the dump on disk, so restarting the server does not re-read the wiki.
async fn refresh(server: Server, hours: u64) {
    let every = std::time::Duration::from_secs(hours * 3600);
    // Before the first sync: a restart is how a hand-set secret mark reaches the atlas.
    pack_atlas(&server).await;
    tokio::time::sleep(first_sync(&server.store.snapshot().dump, every)).await;
    let mut fetched = sync::Fetched::default();
    loop {
        match build(&server, &mut fetched).await {
            Ok(Some(worlds)) => {
                tracing::info!("published {worlds} worlds");
                pack_atlas(&server).await;
            }
            Ok(None) => tracing::info!("the wiki has not changed; the dump stands"),
            // Debug names the unreadable field; Display does not.
            Err(error) => tracing::error!("sync failed: {error:?}"),
        }
        server.progress.done();
        tokio::time::sleep(every).await;
    }
}

async fn pack_atlas(server: &Server) {
    let snapshot = server.store.snapshot();
    server.atlas.pack(&server.http, &snapshot.dump).await;
}

/// The rest of the interval since the dump on disk was built; zero when there is no dump.
fn first_sync(previous: &model::Dump, every: std::time::Duration) -> std::time::Duration {
    let Some(built) = previous.last_update.as_deref().and_then(sync::parse_iso) else {
        return std::time::Duration::ZERO;
    };
    let age = time::OffsetDateTime::now_utc() - built;
    // A stamp in the future waits the full interval, so restarts cannot each trigger a sync.
    every.saturating_sub(age.try_into().unwrap_or(every))
}

/// Split out so every outcome (no change, a failed fetch, a published dump) reaches one cleanup.
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

/// How much of the wiki this sync should read, or `None` when nothing has changed. Everything
/// without a previous dump, once a week, or when the wiki cannot say what changed.
async fn plan(server: &Server, previous: &model::Dump) -> Option<sync::RefreshScope> {
    server.progress.at(progress::CHANGES);
    let Some(built) = previous
        .last_update
        .as_deref()
        .filter(|_| !previous.worlds.is_empty())
    else {
        return Some(sync::RefreshScope::Everything);
    };
    if full_due(previous) {
        tracing::info!("the week is up; reading the whole wiki rather than querying what changed");
        return Some(sync::RefreshScope::Everything);
    }
    let Some(since) = sync::looking_from(built, time::OffsetDateTime::now_utc()) else {
        tracing::info!("the dump is older than the wiki's memory of what it changed; reading all");
        return Some(sync::RefreshScope::Everything);
    };
    match smw::changed_since(&server.http, &since).await {
        Ok(pages) if pages.is_empty() => None,
        Ok(pages) => {
            tracing::info!("{} pages edited since {since}", pages.len());
            Some(sync::RefreshScope::Pages(pages))
        }
        // A needless rebuild costs a minute; a missed one stops following the wiki.
        Err(error) => {
            tracing::warn!("cannot tell what the wiki has changed: {error}");
            Some(sync::RefreshScope::Everything)
        }
    }
}

/// `lastFullUpdate` is the dump's own record of when the whole wiki was last read, so the week
/// survives a restart.
fn full_due(previous: &model::Dump) -> bool {
    match previous
        .last_full_update
        .as_deref()
        .and_then(sync::parse_iso)
    {
        Some(last) => time::OffsetDateTime::now_utc() - last >= FULL_EVERY,
        None => true,
    }
}

/// A TCP `host:port`, or a Unix socket path for nginx's `proxy_pass http://unix:...`, told apart
/// by `/`, which no `host:port` contains, IPv6 literals included.
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

/// Runs until told to stop.
async fn serve(server: Server, options: Options) {
    let app = axum::Router::new()
        // `/data` is the reference implementation's route.
        .route("/data", get(data))
        .route("/data.json", get(data))
        .route("/pollUpdate", get(poll_update))
        // The app's own atlas paths, which shipped builds fetch from this host.
        .route("/static/thumbnails.jpg", get(atlas_jpeg))
        .route("/static/unknown_location.png", get(placeholder_png))
        // YNOproject's game client calls this.
        .route("/getNextLocations", get(next::get_next_locations))
        // The sign-in cookie must be rehosted for this origin.
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
            // A socket file outlives the process; `bind` clears one left by a crash.
            if let Err(error) = std::fs::remove_file(path) {
                tracing::warn!("cannot remove {}: {error}", path.display());
            }
        }
    }
}

/// Opens the `--listen` socket, clearing a stale one; mode `0666`, so the enclosing directory
/// decides who may connect.
fn bind(path: &Path) -> std::io::Result<tokio::net::UnixListener> {
    // Only a dead socket, so a mistyped `--listen` cannot delete a real file.
    if std::os::unix::net::UnixStream::connect(path).is_err()
        && std::fs::symlink_metadata(path).is_ok_and(|file| file.file_type().is_socket())
    {
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
    Ok(listener)
}

/// Serves until Ctrl-C, over TCP or a Unix socket.
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

/// `GET /data` -- the dump as on disk, served during syncs too; `503 needs update` while the dump
/// is empty, since a client cannot tell an empty dump from a wiki with no worlds.
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

/// `GET /static/thumbnails.jpg` -- the atlas, or `503` before the first pack.
async fn atlas_jpeg(State(server): State<Server>) -> axum::response::Response {
    match server.atlas.packed() {
        Some(packed) => picture("image/jpeg", packed.atlas_jpeg.clone()),
        None => not_packed(),
    }
}

/// `GET /static/unknown_location.png` -- the full-size placeholder, or `503` before the first pack.
async fn placeholder_png(State(server): State<Server>) -> axum::response::Response {
    match server.atlas.packed() {
        Some(packed) => picture("image/png", packed.placeholder_png.clone()),
        None => not_packed(),
    }
}

fn picture(content_type: &'static str, bytes: axum::body::Bytes) -> axum::response::Response {
    ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
}

fn not_packed() -> axum::response::Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, "60")],
        "not packed yet\n",
    )
        .into_response()
}

/// `GET /pollUpdate` -- what the sync is doing, in the reference implementation's own JSON.
/// `done` only means no sync is running, even on a server that has never built a dump.
async fn poll_update(State(server): State<Server>) -> impl IntoResponse {
    let task = server.progress.task();
    axum::Json(serde_json::json!({ "task": task, "done": task.is_none() }))
}

#[cfg(test)]
mod tests {
    /// `rustls-no-provider` moves the choice of provider out of reqwest's features and into
    /// [`super::http_client`], where the builder panics at runtime rather than failing to compile.
    #[test]
    fn the_client_has_a_crypto_provider() {
        super::http_client(reqwest::Client::builder());
    }
}
