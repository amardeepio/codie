//! Per-path serialization for file mutations.
//!
//! Concurrent `write` and `edit` calls that target the same canonical path must
//! not interleave. Different paths remain independent.

use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use tokio::sync::Mutex as AsyncMutex;

type LockMap = HashMap<PathBuf, Arc<AsyncMutex<()>>>;
type SharedLockMap = Mutex<LockMap>;

static LOCKS: OnceLock<SharedLockMap> = OnceLock::new();

/// Runs `operation` while holding the mutation lock for `path`.
pub(crate) async fn with_file_mutation<T, F>(path: &Path, operation: F) -> T
where
    F: Future<Output = T>,
{
    let key = canonical_key(path).await;
    let lock = {
        let mut locks = LOCKS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(locks.entry(key).or_default())
    };
    let _guard = lock.lock().await;
    operation.await
}

/// Returns a stable key for `path`, following symlinks when possible.
async fn canonical_key(path: &Path) -> PathBuf {
    if let Ok(canonical) = tokio::fs::canonicalize(path).await {
        return canonical;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(canonical_parent) = tokio::fs::canonicalize(parent).await
    {
        return canonical_parent.join(name);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn mutations_to_the_same_path_serialize() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("file.txt");
        let order = Arc::new(Mutex::new(Vec::new()));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));

        let first = {
            let path = path.clone();
            let order = Arc::clone(&order);
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            tokio::spawn(async move {
                with_file_mutation(&path, async {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(now, Ordering::SeqCst);
                    order.lock().expect("order").push("first-start");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    order.lock().expect("order").push("first-end");
                    active.fetch_sub(1, Ordering::SeqCst);
                })
                .await;
            })
        };
        let second = {
            let path = path.clone();
            let order = Arc::clone(&order);
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            tokio::spawn(async move {
                with_file_mutation(&path, async {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(now, Ordering::SeqCst);
                    order.lock().expect("order").push("second-start");
                    order.lock().expect("order").push("second-end");
                    active.fetch_sub(1, Ordering::SeqCst);
                })
                .await;
            })
        };

        first.await.expect("first task");
        second.await.expect("second task");

        assert_eq!(maximum.load(Ordering::SeqCst), 1);
        let order = order.lock().expect("order");
        assert_eq!(order.len(), 4);
        let first_prefix = order[0].trim_end_matches("-start").trim_end_matches("-end");
        assert!(order[0].ends_with("-start"));
        assert!(order[1].ends_with("-end"));
        assert!(order[1].starts_with(first_prefix));
        assert!(order[2].ends_with("-start"));
        assert!(order[3].ends_with("-end"));
        assert!(order[3].starts_with(order[2].trim_end_matches("-start")));
        assert!(!order[2].starts_with(first_prefix));
    }
}
