//! Loading something off the network without stalling the frame that asked for it.
//!
//! The app draws on one thread and never blocks, so a load cannot be awaited where it is wanted.
//! It is started instead, and the caller keeps a [`Pending`] to look in on each frame until the
//! value turns up. The two platforms have different executors and no shared way to reach one, so
//! [`spawn`] is the seam: everything above it is the same on both.

use std::sync::{Arc, Mutex};

/// Something being loaded, which the draw loop polls until it arrives.
///
/// A slot rather than a channel: nothing here waits, and nothing needs the load's history — only
/// whether it has finished.
pub struct Pending<T>(Arc<Mutex<Option<T>>>);

impl<T> Pending<T> {
    /// The loaded value, the once it is there. `None` while the load is still running, and `None`
    /// forever after it has been taken, so a caller can poll this every frame and act once.
    pub fn take(&self) -> Option<T> {
        self.0.lock().unwrap().take()
    }
}

/// Starts `work` and hands back the slot its result will land in.
///
/// The future is dropped along with its result if the [`Pending`] outlives the app, which is the
/// whole of the cancellation this needs: a load nobody is waiting for costs a wasted download and
/// nothing else.
#[cfg(not(target_family = "wasm"))]
pub fn spawn<T: Send + 'static>(
    work: impl std::future::Future<Output = T> + Send + 'static,
) -> Pending<T> {
    // Downloads drive their sockets through a tokio reactor, so the future needs one running
    // under it. One runtime serves every load the app ever starts; it is built on the first,
    // because most runs never fetch anything at all.
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    let slot = Pending(Arc::default());
    let into = slot.0.clone();
    RUNTIME
        .get_or_init(|| tokio::runtime::Runtime::new().expect("cannot start an async runtime"))
        .spawn(async move { *into.lock().unwrap() = Some(work.await) });
    slot
}

/// Starts `work` and hands back the slot its result will land in.
///
/// On the page there is only ever the one thread, and the browser is already running the executor
/// the future needs, so this hands it over rather than starting one. [`Send`] is therefore not
/// asked for, which is the only way the signature differs from the native one.
#[cfg(target_family = "wasm")]
pub fn spawn<T: 'static>(work: impl std::future::Future<Output = T> + 'static) -> Pending<T> {
    let slot = Pending(Arc::default());
    let into = slot.0.clone();
    wasm_bindgen_futures::spawn_local(async move { *into.lock().unwrap() = Some(work.await) });
    slot
}

/// What a download is made with, and what one fails with.
///
/// The middleware carrying the cache wraps the client in a type of its own, so the two platforms
/// no longer name the same one. Aliases rather than the types themselves, so everything above
/// this is written once: the surface a caller touches -- `get`, `header`, `send` -- is identical,
/// and [`Error`] converts from `reqwest`'s own, so `?` still reaches it from a `send` or an
/// `error_for_status`.
#[cfg(not(target_family = "wasm"))]
pub type Client = reqwest_middleware::ClientWithMiddleware;
/// See [`Client`].
#[cfg(target_family = "wasm")]
pub type Client = reqwest::Client;
/// See [`Client`].
#[cfg(not(target_family = "wasm"))]
pub type Error = reqwest_middleware::Error;
/// See [`Client`].
#[cfg(target_family = "wasm")]
pub type Error = reqwest::Error;

/// The client every download goes through, built once and handed out by the clone.
///
/// The clone is cheap -- everything behind it is shared -- and sharing is the point: one client
/// keeps a connection pool, so the second picture off the wiki reuses the first one's socket and
/// its TLS session rather than starting a handshake of its own. Built lazily, and so on the
/// executor [`spawn`] put the first request on, which is where the native one has to be.
///
/// On the page this is [`reqwest::Client`] unchanged. There it is a wrapper over the browser's
/// `fetch`, which pools connections and keeps an HTTP cache without being asked, so there is
/// nothing here worth building once and nothing to add.
#[cfg(target_family = "wasm")]
pub fn client() -> Client {
    reqwest::Client::new()
}

/// The client every download goes through. See the page's [`client`] above.
#[cfg(not(target_family = "wasm"))]
pub fn client() -> Client {
    static CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(build).clone()
}

/// Assembles it: the transport, then the cache over the top.
#[cfg(not(target_family = "wasm"))]
fn build() -> Client {
    let mut middleware = reqwest_middleware::ClientBuilder::new(transport());
    // Skipped rather than fatal where there is nowhere to keep it: a run with no cache fetches
    // everything it needs anyway, which is exactly what every run did before there was one.
    match cache() {
        Some(cache) => middleware = middleware.with(cache),
        None => log::warn!("downloads will not be cached between runs"),
    }
    middleware.build()
}

/// The client underneath, which is the whole of it on every platform but Android.
#[cfg(not(target_family = "wasm"))]
fn transport() -> reqwest::Client {
    let builder = reqwest::Client::builder();
    // Android alone, and only since reqwest 0.13
    // (<https://github.com/seanmonstar/reqwest/pull/2891>) made `rustls-platform-verifier` the
    // one way it checks a certificate. That verifier reaches the system trust store through a
    // Java class which has to be in the apk and initialised over JNI before the first request;
    // this apk is native the whole way down and has no Java in it at all -- see
    // `android/build.sh` -- so the call would panic on the first picture the app fetches.
    // Compiled-in roots instead: the same Mozilla set the page's own browser would use, which is
    // enough for the one host this build ever asks for anything (see `detail::ORIGIN`). The cost
    // is that they only change when a build does, so a root withdrawn or added between releases
    // is missed. Every other platform is left with the OS verifier, which knows more than a fixed
    // list can.
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(
        webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .map(|root| reqwest::Certificate::from_der(root).expect("a compiled-in root is a cert")),
    );
    builder.build().expect("cannot build an http client")
}

/// The store the cache is kept in, and the rules it is kept under.
///
/// [`CacheMode::Default`] rather than anything of this app's own devising, which is to say the
/// ordinary HTTP rules: the wiki serves its pictures with a `max-age` and an `ETag`, so a picture
/// fetched once is reused without a request until it goes stale and revalidated with a
/// conditional one after that, which comes back empty unless the picture really did change. A
/// single file, and the system empties the directory holding it when the device wants the room --
/// see [`super::store::cache_directory`], which is the whole of the size policy.
///
/// `None` if there is nowhere to keep it or it cannot be opened -- a directory that cannot be
/// made, or a store left corrupt by a run that died mid-write. Losing the cache is not worth
/// failing a run over.
#[cfg(not(target_family = "wasm"))]
fn cache() -> Option<http_cache_reqwest::Cache<http_cache_reqwest::RedbManager>> {
    let file = super::store::cache_directory()?.join("downloads.redb");
    let manager = http_cache_reqwest::RedbManager::new(&file)
        .inspect_err(|error| log::warn!("cannot open {}: {error}", file.display()))
        .ok()?;
    Some(http_cache_reqwest::Cache(http_cache_reqwest::HttpCache {
        mode: http_cache_reqwest::CacheMode::Default,
        manager,
        options: http_cache_reqwest::HttpCacheOptions::default(),
    }))
}
