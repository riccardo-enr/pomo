/*
 * Read-only roll-up over the append-only session log.
 *
 * Streams `log.jsonl`, accumulates per-`kind` totals (completed and aborted
 * counts plus summed `planned_secs`), and produces a 7-row per-day breakdown
 * for the trailing 7-day window ending today.
 *
 * Dates are bucketed by the UTC calendar day of `started_at`. Local-timezone
 * boundaries are deferred to #11 (range filters), where the timezone handling
 * lands in one place.
 *
 * Malformed lines (bad JSON, unknown `kind`, unparseable date) are skipped
 * silently and counted; the count is rendered as a footer so users notice
 * corruption without losing the summary.
 */

use crate::journal::default_log_path;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
struct EntryRecord {
    started_at: String,
    kind: String,
    planned_secs: u64,
    completed: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KindTotals {
    pub completed: u64,
    pub aborted: u64,
    pub completed_secs: u64,
    pub aborted_secs: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DayRow {
    pub date: String,
    pub work_completed: u64,
    pub work_secs: u64,
}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub work: KindTotals,
    pub short_break: KindTotals,
    pub long_break: KindTotals,
    pub timer: KindTotals,
    pub last_7_days: Vec<DayRow>,
    pub malformed: u64,
    pub total_entries: u64,
}

pub fn report_from_default() -> io::Result<Option<Report>> {
    let Some(path) = default_log_path() else {
        return Ok(Some(Report::default()));
    };
    report_from_path(&path, SystemTime::now())
}

pub fn report_from_path(path: &Path, now: SystemTime) -> io::Result<Option<Report>> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(Some(report_from_reader(BufReader::new(f), now)))
}

pub fn report_from_reader<R: BufRead>(reader: R, now: SystemTime) -> Report {
    let today = utc_day(now);
    let mut by_day: HashMap<i64, (u64, u64)> = HashMap::new(); // day -> (count, secs)
    let mut report = Report::default();

    for line in reader.lines() {
        let Ok(line) = line else {
            report.malformed += 1;
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<EntryRecord>(trimmed) else {
            report.malformed += 1;
            continue;
        };

        let bucket = match rec.kind.as_str() {
            "work" => &mut report.work,
            "short_break" | "break" => &mut report.short_break,
            "long_break" => &mut report.long_break,
            "timer" => &mut report.timer,
            _ => {
                report.malformed += 1;
                continue;
            }
        };
        report.total_entries += 1;
        if rec.completed {
            bucket.completed += 1;
            bucket.completed_secs += rec.planned_secs;
        } else {
            bucket.aborted += 1;
            bucket.aborted_secs += rec.planned_secs;
        }

        if rec.kind == "work" && rec.completed {
            match parse_utc_day(&rec.started_at) {
                Some(day) => {
                    let delta = today - day;
                    if (0..7).contains(&delta) {
                        let e = by_day.entry(day).or_insert((0, 0));
                        e.0 += 1;
                        e.1 += rec.planned_secs;
                    }
                }
                None => report.malformed += 1,
            }
        }
    }

    report.last_7_days = (0..7)
        .rev()
        .map(|offset| {
            let day = today - offset;
            let (count, secs) = by_day.get(&day).copied().unwrap_or((0, 0));
            DayRow {
                date: format_utc_day(day),
                work_completed: count,
                work_secs: secs,
            }
        })
        .collect();

    report
}

pub fn render(report: &Report) -> String {
    let mut out = String::new();

    if report.total_entries == 0 && report.malformed == 0 {
        out.push_str("no sessions yet\n");
        return out;
    }

    out.push_str("All-time totals:\n");
    push_row(&mut out, "work", &report.work);
    push_row(&mut out, "short_break", &report.short_break);
    push_row(&mut out, "long_break", &report.long_break);
    push_row(&mut out, "timer", &report.timer);

    out.push_str("\nLast 7 days (UTC, completed work):\n");
    for row in &report.last_7_days {
        out.push_str(&format!(
            "  {}  {:>3} done  {:>4} min\n",
            row.date, row.work_completed, row.work_secs / 60,
        ));
    }

    if report.malformed > 0 {
        out.push_str(&format!(
            "\n({} malformed lines skipped)\n",
            report.malformed
        ));
    }

    out
}

fn push_row(out: &mut String, label: &str, t: &KindTotals) {
    out.push_str(&format!(
        "  {:<12} {:>4} done ({:>4} min) / {:>3} aborted ({:>3} min)\n",
        label,
        t.completed,
        t.completed_secs / 60,
        t.aborted,
        t.aborted_secs / 60,
    ));
}

fn utc_day(t: SystemTime) -> i64 {
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs() as i64;
    secs.div_euclid(86_400)
}

fn parse_utc_day(rfc3339: &str) -> Option<i64> {
    // Expects YYYY-MM-DDTHH:MM:SSZ as produced by humantime::format_rfc3339_seconds.
    if rfc3339.len() < 10 {
        return None;
    }
    let year: i64 = rfc3339.get(0..4)?.parse().ok()?;
    let month: u32 = rfc3339.get(5..7)?.parse().ok()?;
    let day: u32 = rfc3339.get(8..10)?.parse().ok()?;
    Some(days_from_civil(year, month, day))
}

fn format_utc_day(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/*
 * Howard Hinnant's days_from_civil / civil_from_days algorithms (proleptic
 * Gregorian, day 0 = 1970-01-01). Avoids pulling in `chrono` or `time` just
 * for date math. Reference: http://howardhinnant.github.io/date_algorithms.html
 */
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn at(rfc: &str) -> SystemTime {
        humantime::parse_rfc3339(rfc).unwrap()
    }

    fn line(started: &str, kind: &str, planned: u64, completed: bool) -> String {
        format!(
            r#"{{"started_at":"{}","ended_at":"{}","kind":"{}","label":"x","planned_secs":{},"completed":{}}}"#,
            started, started, kind, planned, completed
        )
    }

    #[test]
    fn empty_input_yields_empty_report() {
        let r = report_from_reader(Cursor::new(""), at("2026-05-19T12:00:00Z"));
        assert_eq!(r.total_entries, 0);
        assert_eq!(r.malformed, 0);
        assert_eq!(r.last_7_days.len(), 7);
        assert!(r.last_7_days.iter().all(|row| row.work_secs == 0));
    }

    #[test]
    fn aggregates_kinds_and_completion() {
        let input = [
            line("2026-05-19T08:00:00Z", "work", 1500, true),
            line("2026-05-19T09:00:00Z", "work", 600, false),
            line("2026-05-19T09:30:00Z", "break", 300, true),
            line("2026-05-19T10:00:00Z", "timer", 90, true),
        ]
        .join("\n");
        let r = report_from_reader(Cursor::new(input), at("2026-05-19T12:00:00Z"));
        assert_eq!(r.total_entries, 4);
        assert_eq!(r.work.completed, 1);
        assert_eq!(r.work.aborted, 1);
        assert_eq!(r.work.completed_secs, 1500);
        assert_eq!(r.work.aborted_secs, 600);
        assert_eq!(r.short_break.completed, 1);
        assert_eq!(r.timer.completed, 1);
        assert_eq!(r.malformed, 0);
    }

    #[test]
    fn last_7_days_groups_completed_work_only() {
        let input = [
            line("2026-05-19T08:00:00Z", "work", 1500, true),
            line("2026-05-19T09:00:00Z", "work", 1500, true),
            line("2026-05-19T10:00:00Z", "work", 1500, false), // aborted - ignored
            line("2026-05-13T10:00:00Z", "work", 1500, true),  // 6 days ago - included
            line("2026-05-12T10:00:00Z", "work", 1500, true),  // 7 days ago - excluded
        ]
        .join("\n");
        let r = report_from_reader(Cursor::new(input), at("2026-05-19T12:00:00Z"));
        let today = r.last_7_days.iter().find(|d| d.date == "2026-05-19").unwrap();
        assert_eq!(today.work_completed, 2);
        assert_eq!(today.work_secs, 3000);
        let six_days_ago = r.last_7_days.iter().find(|d| d.date == "2026-05-13").unwrap();
        assert_eq!(six_days_ago.work_completed, 1);
        assert!(r.last_7_days.iter().all(|d| d.date != "2026-05-12"));
    }

    #[test]
    fn malformed_lines_are_counted_and_skipped() {
        let input = [
            line("2026-05-19T08:00:00Z", "work", 1500, true),
            "this is not json".to_string(),
            String::new(),
            r#"{"started_at":"2026-05-19T09:00:00Z","ended_at":"x","kind":"unknown","label":"x","planned_secs":60,"completed":true}"#.to_string(),
        ]
        .join("\n");
        let r = report_from_reader(Cursor::new(input), at("2026-05-19T12:00:00Z"));
        assert_eq!(r.total_entries, 1);
        assert_eq!(r.malformed, 2);
        assert_eq!(r.work.completed, 1);
    }

    #[test]
    fn missing_file_returns_none() {
        let path = std::path::PathBuf::from("/tmp/pomo-stats-does-not-exist-xyz/log.jsonl");
        let _ = std::fs::remove_file(&path);
        let r = report_from_path(&path, SystemTime::now()).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn render_no_sessions_says_so() {
        let s = render(&Report::default());
        assert!(s.contains("no sessions yet"));
    }

    #[test]
    fn civil_roundtrip() {
        for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 5, 19), (1999, 12, 31), (2100, 3, 1)] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d));
        }
    }
}
