//! File span lists, cached per inode (R6). TernFS files are immutable once linked, so a
//! span list never goes stale. There is no block cache: the kernel page cache is the only
//! data cache (FR-011).

use std::sync::Arc;

use moka::future::Cache;
use rustern_core::span::Span;
use rustern_msgs::InodeId;

use crate::ClientError;

/// Most spans held in total.
pub const MAX_SPANS: u64 = 1 << 20;

/// Span lists keyed by inode, sorted by offset; concurrent loads of one inode coalesce.
#[derive(Debug, Clone)]
pub struct SpanCache {
    cache: Cache<InodeId, Arc<[Span]>>,
}

impl Default for SpanCache {
    fn default() -> Self {
        Self::new(MAX_SPANS)
    }
}

impl SpanCache {
    /// A cache holding at most `max_spans` spans.
    pub fn new(max_spans: u64) -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(max_spans)
                .weigher(|_, v: &Arc<[Span]>| u32::try_from(v.len().max(1)).unwrap_or(u32::MAX))
                .build(),
        }
    }

    /// The span list of `id`, loaded by `load` at most once for concurrent callers. A
    /// failed load is not cached.
    pub async fn get_or_load<F>(&self, id: InodeId, load: F) -> Result<Arc<[Span]>, ClientError>
    where
        F: Future<Output = Result<Vec<Span>, ClientError>>,
    {
        self.cache
            .try_get_with(id, async move {
                let mut spans = load.await?;
                spans.sort_by_key(|s| s.byte_offset);
                Ok(Arc::from(spans))
            })
            .await
            .map_err(ClientError::shared)
    }

    /// Cached lists (tests).
    #[doc(hidden)]
    pub async fn entries(&self) -> u64 {
        self.cache.run_pending_tasks().await;
        self.cache.entry_count()
    }
}

/// The span containing file byte `offset`; `None` in a gap or past the end.
pub fn span_at(spans: &[Span], offset: u64) -> Option<&Span> {
    let i = spans.partition_point(|s| s.end() <= offset);
    spans.get(i).filter(|s| s.byte_offset <= offset)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use bytes::Bytes;
    use rustern_core::span::SpanBody;
    use rustern_msgs::Crc;

    use super::*;

    fn span(byte_offset: u64, size: u32) -> Span {
        Span {
            byte_offset,
            size,
            crc: Crc(0),
            body: SpanBody::Inline { data: Bytes::new() },
        }
    }

    #[test]
    fn span_at_binary_search() {
        // [0, 100), [100, 300), gap, [400, 401)
        let spans = [span(0, 100), span(100, 200), span(400, 1)];
        for (o, want) in [
            (0, Some(0)),
            (99, Some(0)),
            (100, Some(100)),
            (299, Some(100)),
            (400, Some(400)),
        ] {
            assert_eq!(span_at(&spans, o).map(|s| s.byte_offset), want, "{o}");
        }
        for o in [300, 399, 401, u64::MAX] {
            assert!(span_at(&spans, o).is_none(), "{o}");
        }
        assert!(span_at(&[], 0).is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_loads_coalesce() {
        let cache = SpanCache::default();
        let loads = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..64)
            .map(|_| {
                let (cache, loads) = (cache.clone(), Arc::clone(&loads));
                tokio::spawn(async move {
                    cache
                        .get_or_load(InodeId(7), async {
                            loads.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            Ok(vec![span(100, 5), span(0, 100)])
                        })
                        .await
                        .unwrap()
                })
            })
            .collect();
        let mut first: Option<Arc<[Span]>> = None;
        for t in tasks {
            let v = t.await.unwrap();
            assert_eq!(v[0].byte_offset, 0, "sorted by offset");
            if let Some(f) = &first {
                assert!(Arc::ptr_eq(f, &v), "same map for everyone");
            }
            first = Some(v);
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failures_are_not_cached() {
        let cache = SpanCache::default();
        let r = cache
            .get_or_load(InodeId(1), async {
                Err(ClientError::Malformed { what: "x" })
            })
            .await;
        assert!(r.is_err());
        let v = cache
            .get_or_load(InodeId(1), async { Ok(vec![span(0, 1)]) })
            .await
            .unwrap();
        assert_eq!(v.len(), 1);
    }

    #[tokio::test]
    async fn bounded_by_span_count() {
        let cache = SpanCache::new(100);
        for i in 0..20 {
            cache
                .get_or_load(InodeId(i), async {
                    Ok((0..10).map(|k| span(k * 10, 10)).collect())
                })
                .await
                .unwrap();
        }
        assert!(cache.entries().await <= 10, "{}", cache.entries().await);
    }
}
