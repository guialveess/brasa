//! [`requests_csv`], the request table of a run as CSV: one row per sample.

use std::fmt::Write as _;
use std::ops::Range;

use crate::csv_text;
use crate::history::status_text;
use crate::progress::{Progress, Ticker};
use crate::samples::{SampleLog, SampleRecord};

/// The header line of the exported CSV.
const HEADER: &str = "id,time_ms,vu,method,request,url,status,latency_ms,size_bytes,error";

/// Every sample of `log` inside `range` (whole seconds `[start, end)` of `elapsed_ms / 1000`,
/// `None` for the whole run) with its `id`, the 1-based position in the log. The id is the
/// position in the whole log, so it does not change when the window does.
///
/// Every sample read, inside the window or not, counts on `ticker`, and the rows end once its job
/// is cancelled.
pub(crate) fn window_rows<'a>(
    log: &'a SampleLog,
    range: Option<&'a Range<u32>>,
    ticker: &'a mut Ticker<'_>,
) -> impl Iterator<Item = (usize, &'a SampleRecord)> {
    log.samples()
        .iter()
        .enumerate()
        .take_while(move |_| ticker.tick())
        .filter(move |(_, sample)| {
            range.is_none_or(|range| range.contains(&(sample.elapsed_ms / 1_000)))
        })
        .map(|(index, sample)| (index + 1, sample))
}

/// `latency_us` in milliseconds with 3 decimals (`1500` is `"1.500"`), without going through a
/// float.
pub(crate) fn latency_ms_text(latency_us: u32) -> String {
    format!("{}.{:03}", latency_us / 1_000, latency_us % 1_000)
}

/// The method and the name of `target` from `targets`, empty when the target is not in the list.
pub(crate) fn target_label(targets: &[(String, String)], target: u32) -> (&str, &str) {
    targets
        .get(target as usize)
        .map_or(("", ""), |(method, name)| (method.as_str(), name.as_str()))
}

/// The requests of a run as CSV, one row per sample in `range` (whole seconds `[start, end)`,
/// `None` for the whole run), in the order of the log.
///
/// The header is `id,time_ms,vu,method,request,url,status,latency_ms,size_bytes,error`:
///
/// - `id` is the 1-based position of the sample in the whole log.
/// - `vu` is 1-based (the log's own index is 0-based).
/// - `method` and `request` come from `targets`, one `(method, name)` pair per target.
/// - `status` is the HTTP code, `timeout` or `failed`.
/// - `latency_ms` has 3 decimals.
/// - `error` is `1` for an error and `0` otherwise.
///
/// Fields with a comma, a quote or a line break are quoted. Lines end with `\n`.
#[must_use]
pub fn requests_csv(
    log: &SampleLog,
    targets: &[(String, String)],
    range: Option<Range<u32>>,
) -> String {
    build(log, targets, range, &Progress::default())
}

/// [`requests_csv`] reporting each sample of the log to `progress`, `None` when it is cancelled.
#[must_use]
pub fn requests_csv_with(
    log: &SampleLog,
    targets: &[(String, String)],
    range: Option<Range<u32>>,
    progress: &Progress,
) -> Option<String> {
    let csv = build(log, targets, range, progress);
    (!progress.is_cancelled()).then_some(csv)
}

/// [`requests_csv_with`], cut short when cancelled.
fn build(
    log: &SampleLog,
    targets: &[(String, String)],
    range: Option<Range<u32>>,
    progress: &Progress,
) -> String {
    let mut out = String::with_capacity(HEADER.len() + 1 + log.len() * 96);
    out.push_str(HEADER);
    out.push('\n');
    let mut ticker = Ticker::new(progress);
    for (id, sample) in window_rows(log, range.as_ref(), &mut ticker) {
        let (method, name) = target_label(targets, sample.target);
        let _ = write!(out, "{id},{},{},", sample.elapsed_ms, sample.vu + 1);
        csv_text::push_field(&mut out, method);
        out.push(',');
        csv_text::push_field(&mut out, name);
        out.push(',');
        csv_text::push_field(&mut out, log.url(sample.url));
        let _ = writeln!(
            out,
            ",{},{},{},{}",
            status_text(sample.status),
            latency_ms_text(sample.latency_us),
            sample.size,
            u8::from(sample.is_error),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::StatusKey;
    use pretty_assertions::assert_eq;

    fn log() -> SampleLog {
        let mut log = SampleLog::new();
        let plain = log.intern("http://t/users");
        let tricky = log.intern("http://t/s?q=a,b&n=\"x\"");
        let rows = [
            (100, 0, 0, plain, StatusKey::Code(201), 1_500, 312),
            (1_200, 4, 1, tricky, StatusKey::Code(500), 40_007, 64),
            (2_500, 1, 1, plain, StatusKey::Timeout, 5_000_000, 0),
            (3_100, 1, 7, plain, StatusKey::Failed, 12, 0),
        ];
        for (elapsed_ms, vu, target, url, status, latency_us, size) in rows {
            log.push(SampleRecord {
                elapsed_ms,
                vu,
                target,
                url,
                status,
                latency_us,
                size,
                is_error: status.is_error(),
            });
        }
        log
    }

    fn targets() -> Vec<(String, String)> {
        vec![
            ("POST".to_string(), "create".to_string()),
            ("GET".to_string(), "list, all".to_string()),
        ]
    }

    #[test]
    fn the_whole_run_has_a_header_and_one_row_per_sample() {
        let csv = requests_csv(&log(), &targets(), None);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines,
            vec![
                "id,time_ms,vu,method,request,url,status,latency_ms,size_bytes,error",
                "1,100,1,POST,create,http://t/users,201,1.500,312,0",
                "2,1200,5,GET,\"list, all\",\"http://t/s?q=a,b&n=\"\"x\"\"\",500,40.007,64,1",
                "3,2500,2,GET,\"list, all\",http://t/users,timeout,5000.000,0,1",
                // A target outside the list has no method or name.
                "4,3100,2,,,http://t/users,failed,0.012,0,1",
            ]
        );
        assert!(csv.ends_with('\n'));
    }

    #[test]
    fn a_range_keeps_the_ids_of_the_whole_log() {
        let csv = requests_csv(&log(), &targets(), Some(1..3));
        let ids: Vec<&str> = csv
            .lines()
            .skip(1)
            .filter_map(|line| line.split(',').next())
            .collect();
        assert_eq!(ids, vec!["2", "3"]);
    }

    #[test]
    fn an_empty_log_is_just_the_header() {
        let csv = requests_csv(&SampleLog::new(), &[], None);
        assert_eq!(
            csv,
            "id,time_ms,vu,method,request,url,status,latency_ms,size_bytes,error\n"
        );
    }

    #[test]
    fn latency_has_three_decimals() {
        assert_eq!(latency_ms_text(0), "0.000");
        assert_eq!(latency_ms_text(999), "0.999");
        assert_eq!(latency_ms_text(1_000), "1.000");
        assert_eq!(latency_ms_text(123_456), "123.456");
    }
}
