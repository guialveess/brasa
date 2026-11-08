//! [`SampleLog`], the per request record of a load test: one [`SampleRecord`] for every request
//! a virtual user completed, in recording order.
//!
//! The aggregates of [`crate::LoadSnapshot`] answer "how did the run go"; the log answers "what
//! happened to each request", which the results view needs for its request table, its error
//! groups and the statistics of a selected time window (see [`crate::window`]).
//!
//! A [`SampleRecord`] is small and `Copy` (28 bytes) so a run of a few million requests stays in
//! memory. The URL of a request does not live in the record: URLs repeat a lot (the same request
//! thousands of times), so the log keeps one interned table of them and each record holds an
//! index into it, see [`SampleLog::url`].
//!
//! # Ordering
//!
//! Samples are stored in the order the aggregator recorded them. Each virtual user measures its
//! `elapsed_ms` before taking the aggregator lock, so two threads can record samples whose
//! `elapsed_ms` differ by a few milliseconds from their position in the log: the log is only
//! approximately sorted by time. Anything that needs a time order should sort a copy, anything
//! that groups by whole seconds (the time window of [`crate::window::window_stats`]) is not
//! affected beyond a request near a second boundary landing in the neighbouring second.

use std::collections::HashMap;

use crate::status::StatusKey;

/// One completed request of a load test run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleRecord {
    /// When the request completed, in milliseconds since the run started. Can be a few
    /// milliseconds out of order with respect to the position in the [`SampleLog`], see the
    /// module docs.
    pub elapsed_ms: u32,
    /// The virtual user that sent it, 0-based.
    pub vu: u32,
    /// The target it belongs to, an index into [`crate::LoadConfig::targets`].
    pub target: u32,
    /// The index of its URL in the log's table, read with [`SampleLog::url`].
    pub url: u32,
    /// How the sample is classified: an HTTP status code, a timeout or another failure.
    pub status: StatusKey,
    /// The latency in microseconds, the same value that feeds the percentiles.
    pub latency_us: u32,
    /// The size of the response body in bytes, `0` when there was no response.
    pub size: u32,
    /// Whether the sample counts as an error ([`StatusKey::is_error`]).
    pub is_error: bool,
}

/// Every [`SampleRecord`] of a run in recording order, plus the table of the URLs they point to.
#[derive(Debug, Clone, Default)]
pub struct SampleLog {
    samples: Vec<SampleRecord>,
    urls: Vec<String>,
    url_index: HashMap<String, u32>,
}

impl SampleLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The index of `url` in the table, adding it the first time it is seen. Repeated URLs cost
    /// a hash lookup and no allocation.
    pub fn intern(&mut self, url: &str) -> u32 {
        if let Some(&index) = self.url_index.get(url) {
            return index;
        }
        // A run would need more than four billion distinct URLs to overflow; saturating keeps
        // the function total instead of panicking.
        let index = u32::try_from(self.urls.len()).unwrap_or(u32::MAX);
        self.urls.push(url.to_string());
        self.url_index.insert(url.to_string(), index);
        index
    }

    /// Appends `record`. Its [`SampleRecord::url`] must come from [`Self::intern`] on this log.
    pub fn push(&mut self, record: SampleRecord) {
        self.samples.push(record);
    }

    /// Every sample, in recording order.
    #[must_use]
    pub fn samples(&self) -> &[SampleRecord] {
        &self.samples
    }

    /// The URL table, indexed by [`SampleRecord::url`].
    #[must_use]
    pub fn urls(&self) -> &[String] {
        &self.urls
    }

    /// The URL at `index`, or an empty string when the index is not in the table.
    #[must_use]
    pub fn url(&self, index: u32) -> &str {
        self.urls.get(index as usize).map_or("", String::as_str)
    }

    /// How many samples the log holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether the log holds no sample.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Appends the samples and URLs of `source` that `self` does not have yet, decided by length:
    /// `self` must be a prefix of `source` (an earlier copy of it, or empty). `O(new)`.
    pub(crate) fn append_missing_from(&mut self, source: &SampleLog) {
        for url in source.urls.iter().skip(self.urls.len()) {
            // Index and position agree because `self` is a prefix of `source`.
            self.intern(url);
        }
        self.samples
            .extend_from_slice(source.samples.get(self.samples.len()..).unwrap_or(&[]));
    }
}

impl PartialEq for SampleLog {
    /// Two logs are equal when they hold the same samples and the same URL table; the interning
    /// map is derived from the table and not compared.
    fn eq(&self, other: &Self) -> bool {
        self.samples == other.samples && self.urls == other.urls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn record(elapsed_ms: u32, url: u32) -> SampleRecord {
        SampleRecord {
            elapsed_ms,
            vu: 0,
            target: 0,
            url,
            status: StatusKey::Code(200),
            latency_us: 1_000,
            size: 10,
            is_error: false,
        }
    }

    #[test]
    fn intern_returns_the_same_index_for_the_same_url() {
        let mut log = SampleLog::new();
        let a = log.intern("http://x/a");
        let b = log.intern("http://x/b");
        assert_eq!((a, b), (0, 1));
        assert_eq!(log.intern("http://x/a"), 0);
        assert_eq!(log.urls().len(), 2);
        assert_eq!(log.url(1), "http://x/b");
        assert_eq!(log.url(9), "");
    }

    #[test]
    fn append_missing_from_copies_only_what_the_target_lacks() {
        let mut source = SampleLog::new();
        let a = source.intern("http://x/a");
        source.push(record(1, a));
        source.push(record(2, a));

        let mut copy = SampleLog::new();
        copy.append_missing_from(&source);
        assert_eq!(copy, source);

        let b = source.intern("http://x/b");
        source.push(record(3, b));
        copy.append_missing_from(&source);
        assert_eq!(copy, source);
        assert_eq!(copy.url(copy.samples()[2].url), "http://x/b");

        // Nothing new: nothing changes.
        copy.append_missing_from(&source);
        assert_eq!(copy.len(), 3);
        assert!(!copy.is_empty());
    }
}
