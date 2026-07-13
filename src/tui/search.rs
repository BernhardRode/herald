//! Fuzzy search engine wrapping nucleo.
//!
//! Provides a `Matcher` that manages injecting items, updating the search pattern,
//! ticking the background workers, and extracting ranked results with match indices.

use parking_lot::Mutex;
use std::ops::DerefMut;
use std::sync::Arc;
use std::thread::available_parallelism;

/// Timeout (ms) for each `nucleo::Nucleo::tick` call.
const MATCHER_TICK_TIMEOUT: u64 = 2;

// ---------------------------------------------------------------------------
// LazyMutex — lazily-initialised global nucleo::Matcher for index computation
// ---------------------------------------------------------------------------

/// A lazily-initialised mutex, avoiding expensive upfront allocation.
pub struct LazyMutex<T> {
    inner: Mutex<Option<T>>,
    init: fn() -> T,
}

impl<T> LazyMutex<T> {
    pub const fn new(init: fn() -> T) -> Self {
        Self {
            inner: Mutex::new(None),
            init,
        }
    }

    pub fn lock(&self) -> impl DerefMut<Target = T> + '_ {
        parking_lot::MutexGuard::map(self.inner.lock(), |val| val.get_or_insert_with(self.init))
    }
}

/// Global lazy matcher used for computing per-character match indices.
static INDEX_MATCHER: LazyMutex<nucleo::Matcher> = LazyMutex::new(nucleo::Matcher::default);

// ---------------------------------------------------------------------------
// MatchedItem
// ---------------------------------------------------------------------------

/// A single matched result with its display string and character-level match indices.
#[derive(Debug, Clone)]
pub struct MatchedItem<I: Clone> {
    /// The original item data.
    pub inner: I,
    /// The string that was matched against.
    pub matched_string: String,
    /// Sorted, deduplicated character indices that matched.
    pub match_indices: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Matcher
// ---------------------------------------------------------------------------

/// High-performance fuzzy matcher wrapping `nucleo::Nucleo`.
pub struct Matcher<I>
where
    I: Sync + Send + Clone + 'static,
{
    inner: nucleo::Nucleo<I>,
    pub total_item_count: u32,
    pub matched_item_count: u32,
    pub running: bool,
    last_pattern: String,
    col_indices_buf: Vec<u32>,
}

fn matcher_threads() -> usize {
    available_parallelism()
        .map(|n| n.get().saturating_sub(3).clamp(1, 32))
        .unwrap_or(4)
}

impl<I> Matcher<I>
where
    I: Sync + Send + Clone + 'static,
{
    /// Create a new matcher with score-based sorting.
    pub fn new() -> Self {
        let n_threads = matcher_threads();
        let inner =
            nucleo::Nucleo::new(nucleo::Config::DEFAULT, Arc::new(|| {}), Some(n_threads), 1);
        Self {
            inner,
            total_item_count: 0,
            matched_item_count: 0,
            running: false,
            // Sentinel value ensures first find() always triggers reparse
            last_pattern: "\x00".to_string(),
            col_indices_buf: Vec::with_capacity(128),
        }
    }

    /// Get an injector handle (thread-safe, cloneable) to push items into the matcher.
    pub fn injector(&self) -> nucleo::Injector<I> {
        self.inner.injector()
    }

    /// Tick the background matcher workers. Call this each frame.
    pub fn tick(&mut self) {
        let status = self.inner.tick(MATCHER_TICK_TIMEOUT);
        self.running = status.running;
    }

    /// Update the search pattern. Only reparses if the pattern has changed.
    pub fn find(&mut self, pattern: &str) {
        if pattern != self.last_pattern {
            self.inner.pattern.reparse(
                0,
                pattern,
                nucleo::pattern::CaseMatching::Smart,
                nucleo::pattern::Normalization::Smart,
                pattern.starts_with(&self.last_pattern),
            );
            self.last_pattern = pattern.to_string();
        }
    }

    /// Extract the top `num_entries` results starting from `offset`.
    pub fn results(&mut self, num_entries: u32, offset: u32) -> Vec<MatchedItem<I>> {
        let snapshot = self.inner.snapshot();
        self.total_item_count = snapshot.item_count();
        self.matched_item_count = snapshot.matched_item_count();

        if offset >= self.matched_item_count {
            return Vec::new();
        }

        let count = num_entries.min(self.matched_item_count - offset);
        let mut matcher = INDEX_MATCHER.lock();
        let mut results = Vec::with_capacity(count as usize);

        for item in snapshot.matched_items(offset..(offset + count).min(self.matched_item_count)) {
            self.col_indices_buf.clear();
            snapshot.pattern().column_pattern(0).indices(
                item.matcher_columns[0].slice(..),
                &mut matcher,
                &mut self.col_indices_buf,
            );

            if self.col_indices_buf.len() > 1 {
                self.col_indices_buf.sort_unstable();
                self.col_indices_buf.dedup();
            }

            let indices: Vec<u32> = self.col_indices_buf.drain(..).collect();
            let matched_string = item.matcher_columns[0].to_string();

            results.push(MatchedItem {
                inner: item.data.clone(),
                matched_string,
                match_indices: indices,
            });
        }

        results
    }
}
