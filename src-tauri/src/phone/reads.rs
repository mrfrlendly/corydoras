//! Keeps label reads for a while, so a phone that reloads or locks during a read never pays twice.

use corydoras_core::inventory::InventoryDraft;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// How long a finished read is kept.
pub(crate) const READ_TTL: Duration = Duration::from_secs(600);
/// How many finished reads are kept.
pub(crate) const READ_CAPACITY: usize = 4;

type Outcome = Result<Vec<InventoryDraft>, String>;

enum Entry {
    Running(watch::Receiver<Option<Outcome>>),
    Done { drafts: Vec<InventoryDraft>, at: Instant },
}

pub(crate) struct ReadCache {
    ttl: Duration,
    capacity: usize,
    entries: Mutex<HashMap<String, Entry>>,
}

impl ReadCache {
    pub(crate) fn new(ttl: Duration, capacity: usize) -> Arc<Self> {
        Arc::new(Self { ttl, capacity, entries: Mutex::new(HashMap::new()) })
    }

    /// The drafts of this photo: kept ones, the ones of a read that runs now, or a new read.
    /// A new read runs on its own task, so a caller that goes away does not stop it.
    pub(crate) async fn get_or_read<F, Fut>(self: &Arc<Self>, photo: &[u8], read: F) -> Outcome
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Outcome> + Send + 'static,
    {
        let key = corydoras_core::util::fnv1a64(photo);
        let mut rx = {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            let ttl = self.ttl;
            entries.retain(|_, entry| !matches!(entry, Entry::Done { at, .. } if at.elapsed() >= ttl));
            match entries.get(&key) {
                Some(Entry::Done { drafts, .. }) => return Ok(drafts.clone()),
                Some(Entry::Running(rx)) => rx.clone(),
                None => {
                    let (tx, rx) = watch::channel(None);
                    entries.insert(key.clone(), Entry::Running(rx.clone()));
                    let (reading, cache) = (read(), self.clone());
                    tauri::async_runtime::spawn(async move {
                        let outcome = reading.await;
                        cache.finish(key, &outcome);
                        let _ = tx.send(Some(outcome));
                    });
                    rx
                }
            }
        };
        loop {
            if let Some(outcome) = rx.borrow().clone() {
                return outcome;
            }
            if rx.changed().await.is_err() {
                return rx.borrow().clone().unwrap_or_else(|| Err("The label read stopped. Try again.".into()));
            }
        }
    }

    /// Keeps a good result and drops the oldest finished ones beyond the capacity; forgets a failure.
    fn finish(&self, key: String, outcome: &Outcome) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(drafts) = outcome else {
            entries.remove(&key);
            return;
        };
        entries.insert(key, Entry::Done { drafts: drafts.clone(), at: Instant::now() });
        while entries.len() > self.capacity {
            let oldest = entries
                .iter()
                .filter_map(|(k, e)| match e {
                    Entry::Done { at, .. } => Some((k.clone(), *at)),
                    Entry::Running(_) => None,
                })
                .min_by_key(|(_, at)| *at);
            match oldest {
                Some((k, _)) => entries.remove(&k),
                None => break,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corydoras_core::inventory::InventoryDraft;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    type Count = Arc<AtomicUsize>;

    /// A read that takes 100 ms and names its one draft after the call number.
    fn reader(count: &Count) -> impl FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<InventoryDraft>, String>> + Send>> {
        let count = count.clone();
        move || {
            Box::pin(async move {
                let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(vec![InventoryDraft { name: n.to_string(), ..Default::default() }])
            })
        }
    }

    fn names(result: Result<Vec<InventoryDraft>, String>) -> Vec<String> {
        result.unwrap().into_iter().map(|d| d.name).collect()
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn same_photo_while_running_reads_once() {
        run(async {
            let (cache, count) = (ReadCache::new(Duration::from_secs(60), 4), Count::default());
            let (a, b) = tokio::join!(cache.get_or_read(b"A", reader(&count)), cache.get_or_read(b"A", reader(&count)));
            assert_eq!(names(a), ["1"]);
            assert_eq!(names(b), ["1"]);
            assert_eq!(count.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn same_photo_after_it_finished_reads_once() {
        run(async {
            let (cache, count) = (ReadCache::new(Duration::from_secs(60), 4), Count::default());
            assert_eq!(names(cache.get_or_read(b"A", reader(&count)).await), ["1"]);
            assert_eq!(names(cache.get_or_read(b"A", reader(&count)).await), ["1"]);
            assert_eq!(count.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn different_photos_read_separately() {
        run(async {
            let (cache, count) = (ReadCache::new(Duration::from_secs(60), 4), Count::default());
            assert_eq!(names(cache.get_or_read(b"A", reader(&count)).await), ["1"]);
            assert_eq!(names(cache.get_or_read(b"B", reader(&count)).await), ["2"]);
            assert_eq!(count.load(Ordering::SeqCst), 2);
        });
    }

    #[test]
    fn an_expired_result_is_read_again() {
        run(async {
            let (cache, count) = (ReadCache::new(Duration::from_millis(50), 4), Count::default());
            cache.get_or_read(b"A", reader(&count)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(80)).await;
            assert_eq!(names(cache.get_or_read(b"A", reader(&count)).await), ["2"]);
        });
    }

    #[test]
    fn a_caller_that_leaves_does_not_cancel_the_read() {
        run(async {
            let (cache, count) = (ReadCache::new(Duration::from_secs(60), 4), Count::default());
            let left = tokio::time::timeout(Duration::from_millis(10), cache.get_or_read(b"A", reader(&count))).await;
            assert!(left.is_err(), "the first caller should have given up");
            tokio::time::sleep(Duration::from_millis(200)).await;
            let started = Instant::now();
            assert_eq!(names(cache.get_or_read(b"A", reader(&count)).await), ["1"]);
            assert!(started.elapsed() < Duration::from_millis(50));
            assert_eq!(count.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn a_failed_read_is_not_kept() {
        run(async {
            let (cache, count) = (ReadCache::new(Duration::from_secs(60), 4), Count::default());
            let failing = {
                let count = count.clone();
                move || -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<InventoryDraft>, String>> + Send>> {
                    Box::pin(async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        Err("provider down".to_string())
                    })
                }
            };
            assert_eq!(cache.get_or_read(b"A", failing).await.unwrap_err(), "provider down");
            assert_eq!(names(cache.get_or_read(b"A", reader(&count)).await), ["2"]);
        });
    }

    #[test]
    fn the_oldest_finished_entry_goes_first() {
        run(async {
            let (cache, count) = (ReadCache::new(Duration::from_secs(60), 2), Count::default());
            for photo in [b"A", b"B", b"C"] {
                cache.get_or_read(photo, reader(&count)).await.unwrap();
            }
            assert_eq!(count.load(Ordering::SeqCst), 3);
            cache.get_or_read(b"A", reader(&count)).await.unwrap();
            assert_eq!(count.load(Ordering::SeqCst), 4);
            cache.get_or_read(b"C", reader(&count)).await.unwrap();
            assert_eq!(count.load(Ordering::SeqCst), 4);
        });
    }
}
