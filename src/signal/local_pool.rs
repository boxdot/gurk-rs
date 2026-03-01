use std::pin::Pin;
use std::{future::Future, thread};

use tokio::{sync::mpsc, task::LocalSet};

type TaskFn = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + 'static>> + Send + 'static>;

/// A handle for spawning `!Send` futures on a background thread's [`tokio::task::LocalSet`].
///
/// Note: We could have used `tokio_util::task::LocalPoolHandle` instead, however it does not allow
/// to set the stack size of the background thread(s).
#[derive(Clone)]
pub struct LocalPool {
    tx: mpsc::UnboundedSender<TaskFn>,
}

impl LocalPool {
    pub fn new() -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<TaskFn>();
        thread::Builder::new()
            // Signal uses quite some stack space for post quantum crypto.
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let local_set = LocalSet::new();
                runtime.block_on(local_set.run_until(async move {
                    while let Some(task_fn) = rx.recv().await {
                        tokio::task::spawn_local(task_fn());
                    }
                }));
            })
            .unwrap();
        Self { tx }
    }

    /// Spawns a `!Send` future on the background `LocalSet`.
    ///
    /// The factory closure must be `Send` (so it can cross the channel), but the future it
    /// produces does not need to be `Send`.
    pub fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let _ = self.tx.send(Box::new(move || Box::pin(f())));
    }
}

impl Default for LocalPool {
    fn default() -> Self {
        Self::new()
    }
}
