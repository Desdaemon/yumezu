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
/// One request being put together, which is the type a caller has to name to hand one on before
/// sending it. See [`Client`], and `yno`'s `session::sign`, which is what does the handing on.
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

/// Assembles it: the transport, the cache over the top, and [`judged`] over that.
///
/// Order is the whole of it. A builder runs its middleware outermost first, so `judged` is added
/// before the cache and therefore wraps it, which is the only position the cache's verdict can be
/// read from -- it is written onto the response on the way back out.
#[cfg(not(target_family = "wasm"))]
fn build() -> Client {
    let mut middleware = reqwest_middleware::ClientBuilder::new(transport()).with(judged);
    // Skipped rather than fatal where there is nowhere to keep it: a run with no cache fetches
    // everything it needs anyway, which is exactly what every run did before there was one.
    match cache() {
        Some(cache) => middleware = middleware.with(cache),
        None => log::warn!("downloads will not be cached between runs"),
    }
    middleware.build()
}

/// Logs what the cache made of every request, which is otherwise not observable at all: the store
/// is one opaque file, and the whole point of a hit is that no traffic leaves the device to watch.
///
/// Written at `info` because that is the level Android is set to -- see `lib`'s `android_main` --
/// and a phone is where this is hardest to answer any other way: there is no proxy to watch and no
/// cache directory to look in without a debug build. The volume is a few requests a run plus one
/// per picture the view comes close enough to sharpen, which is the traffic being asked about.
///
/// The two headers are the cache's own account of itself, stamped onto the response by
/// `http-cache` on the way out: `x-cache-lookup` says whether the store had anything for the
/// address at all and `x-cache` whether that thing was served. So `HIT`/`HIT` is a free answer,
/// `HIT`/`MISS` is one the store held but had to revalidate or could not use, and `MISS`/`MISS`
/// went to the network cold. Neither header means there is no cache in this client, which is what
/// [`build`] warns about. Named here rather than imported: the crate keeps its own constants for
/// them behind a private re-export.
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
    Some(http_cache_reqwest::Cache(http_cache_reqwest::HttpCache {
        mode: http_cache_reqwest::CacheMode::Default,
        manager: store()?.clone(),
        options: http_cache_reqwest::HttpCacheOptions::default(),
    }))
}

/// The open store itself, kept apart from [`cache`] so that [`clear`] can reach the same one.
///
/// Opened once. Two handles on one redb file is a lock the second would fail on, and a second
/// store would in any case be a second answer: emptying it would leave the client still serving
/// out of the first.
///
/// `None` if there is nowhere to keep it or it cannot be opened -- a directory that cannot be
/// made, or a store left corrupt by a run that died mid-write. Losing the cache is not worth
/// failing a run over.
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

/// How far the cache has got with emptying itself, which is the whole of what the button in the
/// settings tab draws. See [`clear`].
///
/// Not on the page: there is no store of this app's own there. The browser keeps the HTTP cache,
/// and only the person reading can empty that.
#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cleared {
    /// Nobody has asked this run.
    Never,
    /// Asked, and still going.
    Clearing,
    /// Emptied. Every later request goes to the network cold, and fills the store again as it
    /// comes back.
    Done,
    /// Could not be. The store's own complaint is logged rather than shown: there is nothing a
    /// person can do about a redb error, and the log is where the rest of the cache's account of
    /// itself already goes -- see [`judged`].
    Failed,
}

/// What [`clear`] has come to. Global, because the cache is: one store behind one client, so one
/// answer however many places ask for it.
#[cfg(not(target_family = "wasm"))]
static CLEARED: Mutex<Cleared> = Mutex::new(Cleared::Never);

/// See [`Cleared`].
#[cfg(not(target_family = "wasm"))]
pub fn cleared() -> Cleared {
    *CLEARED.lock().unwrap()
}

/// Empties the store, off the drawing thread.
///
/// Started rather than awaited, like every other load here: the work is one redb transaction, but
/// it is a transaction over a file that may hold every picture the run has fetched, and the frame
/// that asked cannot wait on a disk. [`cleared`] is how the asking frame's successors find out.
///
/// The client is left alone. It holds a clone of the same [`store`], so it is serving out of the
/// store this empties and needs no rebuilding: the next request finds nothing, goes to the
/// network, and fills it again.
///
/// What this frees is the cache, not the disk. redb hands the emptied pages back to its own free
/// list and leaves the file the size it had grown to, and shrinking it is a compaction the manager
/// keeps no way to ask for. So the room comes back as the next downloads are written into it
/// rather than at the press, and a device short of space empties the whole directory itself -- see
/// [`super::store::cache_directory`].
#[cfg(not(target_family = "wasm"))]
pub fn clear() {
    *CLEARED.lock().unwrap() = Cleared::Clearing;
    spawn(async {
        let outcome = match store() {
            Some(store) => store.clear().await.map_err(|error| error.to_string()),
            // Not a failure of the clearing so much as of the cache, which said so when it could
            // not be opened. Reported the same way regardless: there is still nothing kept.
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
