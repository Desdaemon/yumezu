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
