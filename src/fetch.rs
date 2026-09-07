//! Loading something off the network without stalling the frame that asked for it.
//!
//! The app draws on one thread and never blocks, so a load is started rather than awaited and the
//! caller keeps a [`Pending`] to look in on each frame. The two platforms have different
//! executors and no shared way to reach one, so [`spawn`] is the seam: everything above it is the
//! same on both.

use std::sync::{Arc, Mutex};

/// A slot rather than a channel: nothing here waits, and nothing needs the load's history.
pub struct Pending<T>(Arc<Mutex<Option<T>>>);

impl<T> Pending<T> {
    /// `None` while the load is still running, and `None` forever after it has been taken, so a
    /// caller can poll every frame and act once.
    pub fn take(&self) -> Option<T> {
        self.0.lock().unwrap().take()
    }
}

/// No cancellation: a load nobody is waiting for costs a wasted download and nothing else.
#[cfg(not(target_family = "wasm"))]
pub fn spawn<T: Send + 'static>(
    work: impl std::future::Future<Output = T> + Send + 'static,
) -> Pending<T> {
    // One runtime serves every load, built on the first because most runs fetch nothing at all.
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    let slot = Pending(Arc::default());
    let into = slot.0.clone();
    RUNTIME
        .get_or_init(|| tokio::runtime::Runtime::new().expect("cannot start an async runtime"))
        .spawn(async move { *into.lock().unwrap() = Some(work.await) });
    slot
}

/// The browser already runs the executor the future needs, so this hands it over rather than
/// starting one. [`Send`] is therefore not asked for, the only way this differs from the native
/// signature.
#[cfg(target_family = "wasm")]
pub fn spawn<T: 'static>(work: impl std::future::Future<Output = T> + 'static) -> Pending<T> {
    let slot = Pending(Arc::default());
    let into = slot.0.clone();
    wasm_bindgen_futures::spawn_local(async move { *into.lock().unwrap() = Some(work.await) });
    slot
}

/// The middleware carrying the cache wraps the client in a type of its own, so the two platforms
/// no longer name the same one. Aliased so everything above is written once: the surface a caller
/// touches -- `get`, `header`, `send` -- is identical, and [`Error`] converts from `reqwest`'s
/// own, so `?` still reaches it.
#[cfg(not(target_family = "wasm"))]
pub type Client = reqwest_middleware::ClientWithMiddleware;
/// See [`Client`].
#[cfg(target_family = "wasm")]
pub type Client = reqwest::Client;
/// Named so a caller can hand a half-built request on before sending it -- see `yno`'s
/// `session::sign`. Aliased for the reason in [`Client`].
#[cfg(not(target_family = "wasm"))]
pub type RequestBuilder = reqwest_middleware::RequestBuilder;
/// See [`RequestBuilder`].
#[cfg(target_family = "wasm")]
pub type RequestBuilder = reqwest::RequestBuilder;
/// See [`Client`].
#[cfg(not(target_family = "wasm"))]
pub type Error = reqwest_middleware::Error;
/// See [`Client`].
#[cfg(target_family = "wasm")]
pub type Error = reqwest::Error;

/// Unbuilt and unwrapped on the page: this is a thin cover over the browser's `fetch`, which
/// pools connections and keeps an HTTP cache without being asked.
#[cfg(target_family = "wasm")]
pub fn client() -> Client {
    reqwest::Client::new()
}

/// Built once and handed out by the clone, which is cheap because everything behind it is shared.
/// Sharing is the point: one connection pool, so the second picture off the wiki reuses the
/// first's socket and TLS session. Built lazily, and so on the executor [`spawn`] put the first
/// request on, which is where it has to be.
#[cfg(not(target_family = "wasm"))]
pub fn client() -> Client {
    static CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(build).clone()
}

/// Order is the whole of it. A builder runs its middleware outermost first, so [`judged`] is added
/// before the cache and therefore wraps it, the only position the cache's verdict can be read
/// from -- it is written onto the response on the way back out.
#[cfg(not(target_family = "wasm"))]
fn build() -> Client {
    let mut middleware = reqwest_middleware::ClientBuilder::new(transport()).with(judged);
    // Skipped rather than fatal: a run with no cache fetches everything it needs anyway.
    match cache() {
        Some(cache) => middleware = middleware.with(cache),
        None => log::warn!("downloads will not be cached between runs"),
    }
    middleware.build()
}

/// Logs what the cache made of every request, which is otherwise unobservable: the store is one
/// opaque file, and the point of a hit is that no traffic leaves the device to watch. At `info`
/// because that is the level Android is set to, and a phone has no proxy to watch and no cache
/// directory to look in without a debug build.
///
/// `x-cache-lookup` says whether the store had anything for the address at all and `x-cache`
/// whether that thing was served, so `HIT`/`MISS` was held but had to be revalidated or could not
/// be used. Neither header means this client has no cache, which is what [`build`] warns about.
/// Named here rather than imported: the crate keeps its own constants behind a private re-export.
#[cfg(not(target_family = "wasm"))]
fn judged<'a>(
    request: reqwest::Request,
    extensions: &'a mut http::Extensions,
    next: reqwest_middleware::Next<'a>,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<reqwest::Response, Error>> + Send + 'a>,
> {
    Box::pin(async move {
        let url = request.url().clone();
        let response = next.run(request, extensions).await?;
        let said = |header: &str| {
            response
                .headers()
                .get(header)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("none")
                .to_owned()
        };
        log::info!(
            "{} {}: cache lookup {}, served {}",
            response.status().as_u16(),
            url,
            said("x-cache-lookup"),
            said("x-cache"),
        );
        Ok(response)
    })
}

#[cfg(not(target_family = "wasm"))]
fn transport() -> reqwest::Client {
    let builder = reqwest::Client::builder();
    // reqwest 0.13 made `rustls-platform-verifier` its one way to check a certificate, and that
    // verifier reaches the system trust store through a Java class which must be in the apk and
    // initialised over JNI before the first request. This apk has no Java in it at all -- see
    // `android/build.sh` -- so the call would panic on the first picture fetched. Compiled-in
    // roots instead, at the cost of only changing when a build does. Every other platform keeps
    // the OS verifier. <https://github.com/seanmonstar/reqwest/pull/2891>
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(
        webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .map(|root| reqwest::Certificate::from_der(root).expect("a compiled-in root is a cert")),
    );
    builder.build().expect("cannot build an http client")
}

/// Ordinary HTTP rules, which is all the wiki needs: it serves its pictures with a `max-age` and
/// an `ETag`, so a picture fetched once is reused without a request until it goes stale, then
/// revalidated conditionally. Nothing here caps the size -- the system empties the directory
/// holding the store when the device wants the room, and that is the whole of the policy.
///
/// `None` if there is nowhere to keep it or it cannot be opened, which is not worth failing a run
/// over.
#[cfg(not(target_family = "wasm"))]
fn cache() -> Option<http_cache_reqwest::Cache<http_cache_reqwest::RedbManager>> {
    Some(http_cache_reqwest::Cache(http_cache_reqwest::HttpCache {
        mode: http_cache_reqwest::CacheMode::Default,
        manager: store()?.clone(),
        options: http_cache_reqwest::HttpCacheOptions::default(),
    }))
}

/// Kept apart from [`cache`] so [`clear`] reaches the same store. Opened once: two handles on one
/// redb file is a lock the second would fail on, and emptying a second store would leave the
/// client serving out of the first.
#[cfg(not(target_family = "wasm"))]
fn store() -> Option<&'static http_cache_reqwest::RedbManager> {
    static STORE: std::sync::OnceLock<Option<http_cache_reqwest::RedbManager>> =
        std::sync::OnceLock::new();
    STORE
        .get_or_init(|| {
            let file = super::store::cache_directory()?.join("downloads.redb");
            http_cache_reqwest::RedbManager::new(&file)
                .inspect_err(|error| log::warn!("cannot open {}: {error}", file.display()))
                .ok()
        })
        .as_ref()
}

/// How far [`clear`] has got, which is what the button in the settings tab draws. Absent on the
/// page, which has no store of this app's own -- the browser keeps that HTTP cache, and only the
/// person reading can empty it.
#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cleared {
    /// Nobody has asked this run.
    Never,
    Clearing,
    Done,
    /// The store's own complaint is logged rather than shown: there is nothing a person can do
    /// about a redb error.
    Failed,
}

/// Global because the cache is: one store behind one client, so one answer however many places
/// ask.
#[cfg(not(target_family = "wasm"))]
static CLEARED: Mutex<Cleared> = Mutex::new(Cleared::Never);

#[cfg(not(target_family = "wasm"))]
pub fn cleared() -> Cleared {
    *CLEARED.lock().unwrap()
}

/// Started rather than awaited, like every other load here: one redb transaction, but over a file
/// that may hold every picture the run has fetched. [`cleared`] is how later frames find out.
///
/// The client is left alone -- it holds a clone of the same [`store`], so the next request finds
/// nothing and fills it again.
///
/// What this frees is the cache, not the disk: redb hands the emptied pages back to its own free
/// list, leaves the file the size it had grown to, and the manager offers no compaction. The room
/// comes back as later downloads are written into it.
#[cfg(not(target_family = "wasm"))]
pub fn clear() {
    *CLEARED.lock().unwrap() = Cleared::Clearing;
    spawn(async {
        let outcome = match store() {
            Some(store) => store.clear().await.map_err(|error| error.to_string()),
            // Really a failure of [`cache`], which said so when it could not be opened, but
            // reported the same way: there is still nothing kept.
            None => Err("there is no cache to clear".to_owned()),
        };
        *CLEARED.lock().unwrap() = match outcome {
            Ok(()) => {
                log::info!("the download cache has been emptied");
                Cleared::Done
            }
            Err(error) => {
                log::warn!("cannot empty the download cache: {error}");
                Cleared::Failed
            }
        };
    });
}
