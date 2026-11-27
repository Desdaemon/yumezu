//! Network requests that each frame polls through a [`Pending`] without blocking; [`spawn`] is
//! the per-platform executor seam.

use std::sync::{Arc, Mutex};

/// A one-value slot: nothing waits on a load, and only the result is read.
pub struct Pending<T>(Arc<Mutex<Option<T>>>);

impl<T> Pending<T> {
    /// `None` while loading and after the result is taken, so a caller can poll every frame and act
    /// once.
    pub fn take(&self) -> Option<T> {
        self.0.lock().unwrap().take()
    }
}

/// No cancellation: a load nobody is waiting for costs a wasted download and nothing else.
#[cfg(not(target_family = "wasm"))]
pub fn spawn<T: Send + 'static>(
    work: impl std::future::Future<Output = T> + Send + 'static,
) -> Pending<T> {
    // Built lazily: most sessions fetch nothing.
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    let slot = Pending(Arc::default());
    let into = slot.0.clone();
    RUNTIME
        .get_or_init(|| tokio::runtime::Runtime::new().expect("cannot start an async runtime"))
        .spawn(async move { *into.lock().unwrap() = Some(work.await) });
    slot
}

/// Spawned on the browser's executor, so [`Send`] is not required.
#[cfg(target_family = "wasm")]
pub fn spawn<T: 'static>(work: impl std::future::Future<Output = T> + 'static) -> Pending<T> {
    let slot = Pending(Arc::default());
    let into = slot.0.clone();
    wasm_bindgen_futures::spawn_local(async move { *into.lock().unwrap() = Some(work.await) });
    slot
}

/// On the native build the cache middleware wraps the client in another type. The surface is
/// identical, and [`Error`] converts from `reqwest`'s so `?` still works.
#[cfg(not(target_family = "wasm"))]
pub type Client = reqwest_middleware::ClientWithMiddleware;
#[cfg(target_family = "wasm")]
pub type Client = reqwest::Client;
/// Named so a caller can amend a request before sending, as `yno`'s `session::sign` does. Aliased
/// as [`Client`] is.
#[cfg(not(target_family = "wasm"))]
pub type RequestBuilder = reqwest_middleware::RequestBuilder;
#[cfg(target_family = "wasm")]
pub type RequestBuilder = reqwest::RequestBuilder;
#[cfg(not(target_family = "wasm"))]
pub type Error = reqwest_middleware::Error;
#[cfg(target_family = "wasm")]
pub type Error = reqwest::Error;

/// On the web build, a thin wrapper over the browser's `fetch`, which pools connections and keeps
/// an HTTP cache by default.
#[cfg(target_family = "wasm")]
pub fn client() -> Client {
    reqwest::Client::new()
}

/// Built once and cloned, the clones sharing one connection pool and TLS sessions. Built lazily, so
/// the client lands on the executor [`spawn`] put the first request on, as required.
#[cfg(not(target_family = "wasm"))]
pub fn client() -> Client {
    static CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(build).clone()
}

/// Sends a finished request and reads the whole body, treating an error status as an error.
pub trait FetchExt {
    fn fetch_bytes(self) -> impl Future<Output = Result<Vec<u8>, Error>>;
    fn fetch_text(self) -> impl Future<Output = Result<String, Error>>;
}

impl FetchExt for RequestBuilder {
    async fn fetch_bytes(self) -> Result<Vec<u8>, Error> {
        let body = self.send().await?.error_for_status()?.bytes().await?;
        Ok(body.into())
    }

    async fn fetch_text(self) -> Result<String, Error> {
        let body = self.send().await?.error_for_status()?.text().await?;
        Ok(body)
    }
}

/// Middleware runs outermost first, so [`judged`] is added before the cache to read the headers
/// the cache writes on the response.
#[cfg(not(target_family = "wasm"))]
fn build() -> Client {
    let mut middleware = reqwest_middleware::ClientBuilder::new(transport()).with(judged);
    // A session with no cache fetches everything anyway.
    match cache() {
        Some(cache) => middleware = middleware.with(cache),
        None => log::warn!("downloads will not be cached between runs"),
    }
    middleware.build()
}

/// Logs at `debug` how the cache answered each request, which is otherwise unobservable.
///
/// `x-cache-lookup` says whether the store held the URL, and `x-cache` whether the store served
/// the response: `HIT`/`MISS` means held but revalidated, and `none` means no cache. The crate
/// keeps the header names private.
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
        log::debug!(
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
    // reqwest panics without a process-wide provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let builder = reqwest::Client::builder();
    // The platform verifier needs a Java class this apk lacks.
    // <https://github.com/seanmonstar/reqwest/pull/2891>
    #[cfg(target_os = "android")]
    let builder =
        builder.tls_certs_only(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().map(|root| {
            reqwest::Certificate::from_der(root).expect("a compiled-in root is a cert")
        }));
    builder.build().expect("cannot build an http client")
}

/// Standard HTTP caching, enough for the wiki's `max-age` and `ETag` headers. No size cap: the
/// system empties the cache directory when the device needs space.
///
/// `None` if the store has no directory or cannot be opened.
#[cfg(not(target_family = "wasm"))]
fn cache() -> Option<http_cache_reqwest::Cache<http_cache_reqwest::RedbManager>> {
    Some(http_cache_reqwest::Cache(http_cache_reqwest::HttpCache {
        mode: http_cache_reqwest::CacheMode::Default,
        manager: store()?.clone(),
        options: http_cache_reqwest::HttpCacheOptions::default(),
    }))
}

/// Opened once, since redb locks the file against a second handle, and shared with [`clear`].
/// Flushed on every write: a partial batch is flushed only on drop, and the manager never drops.
#[cfg(not(target_family = "wasm"))]
fn store() -> Option<&'static http_cache_reqwest::RedbManager> {
    static STORE: std::sync::OnceLock<Option<http_cache_reqwest::RedbManager>> =
        std::sync::OnceLock::new();
    STORE
        .get_or_init(|| {
            let file = super::store::cache_directory()?.join("downloads.redb");
            redb::Database::create(&file)
                .map_err(|error| error.to_string())
                .and_then(|database| {
                    http_cache_reqwest::RedbManager::from_database_with_flush_interval(
                        std::sync::Arc::new(database),
                        1,
                    )
                    .map_err(|error| error.to_string())
                })
                .inspect_err(|error| log::warn!("cannot open {}: {error}", file.display()))
                .ok()
        })
        .as_ref()
}

/// How far [`clear`] has got, for the settings tab's button. Absent on the web build, where only
/// the user can empty the browser's HTTP cache.
#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cleared {
    Never,
    Clearing,
    Done,
    /// The redb error goes to the log only, as the user cannot act on one.
    Failed,
}

/// Global like the cache: one store behind one client.
#[cfg(not(target_family = "wasm"))]
static CLEARED: Mutex<Cleared> = Mutex::new(Cleared::Never);

#[cfg(not(target_family = "wasm"))]
pub fn cleared() -> Cleared {
    *CLEARED.lock().unwrap()
}

/// Spawned like every other load, the file possibly holding every picture fetched; later frames
/// read [`cleared`]. The client shares [`store`], so later requests refill the store.
///
/// Disk space stays allocated: redb has no compaction and reuses freed pages for later downloads.
#[cfg(not(target_family = "wasm"))]
pub fn clear() {
    *CLEARED.lock().unwrap() = Cleared::Clearing;
    spawn(async {
        let outcome = match store() {
            Some(store) => store.clear().await.map_err(|error| error.to_string()),
            // [`store`] already logged the failure.
            None => Err("there is no cache to clear".to_owned()),
        };
        *CLEARED.lock().unwrap() = match outcome {
            Ok(()) => {
                log::warn!("the download cache has been emptied");
                Cleared::Done
            }
            Err(error) => {
                log::warn!("cannot empty the download cache: {error}");
                Cleared::Failed
            }
        };
    });
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    /// `rustls-no-provider` moves the provider choice from reqwest's features into
    /// [`super::transport`], where a missing provider compiles and panics at runtime.
    #[test]
    fn transport_has_a_crypto_provider() {
        super::transport();
    }
}
