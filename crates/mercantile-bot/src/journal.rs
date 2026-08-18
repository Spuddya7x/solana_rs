//! An append-only JSONL record of everything the bot did.
//!
//! One line per event, flushed as it is written: a run that is killed mid-tick
//! still leaves a complete record up to that point, which is what makes
//! `mercbot report` trustworthy.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::execution::Fill;
use crate::risk::RejectReason;

/// One journal line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// A run began.
    Start {
        ts: i64,
        mode: String,
        markets: usize,
        strategies: Vec<String>,
        gp: f64,
    },
    /// A tick completed.
    Tick {
        ts: i64,
        markets: usize,
        signals: usize,
        fills: usize,
        gp: f64,
        equity: f64,
        realized_pnl: f64,
    },
    /// A market's price at a tick. Only written when `journal.record_snapshots` is on.
    Snapshot {
        ts: i64,
        market: String,
        price: f64,
        floor: f64,
        exit_depth_gp: f64,
    },
    /// A trade happened.
    Fill(Box<Fill>),
    /// The risk manager refused a signal.
    Reject { ts: i64, reason: RejectReason },
    /// Something went wrong that did not stop the run.
    Error {
        ts: i64,
        context: String,
        message: String,
    },
    /// The run ended.
    Stop {
        ts: i64,
        reason: String,
        realized_pnl: f64,
        trades: u64,
    },
}

/// Appends [`Event`]s to a file.
pub struct Journal {
    file: Option<File>,
    path: PathBuf,
}

impl Journal {
    /// Open (or create) a journal file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening journal {}", path.display()))?;
        Ok(Self {
            file: Some(file),
            path,
        })
    }

    /// A journal that discards everything, for tests and dry runs.
    pub fn none() -> Self {
        Self {
            file: None,
            path: PathBuf::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write one event. Journal failures are reported, never fatal: losing the
    /// record of a trade is bad, but stopping a running bot mid-position is worse.
    pub fn record(&mut self, event: &Event) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        match serde_json::to_string(event) {
            Ok(line) => {
                if let Err(err) = writeln!(file, "{line}").and_then(|()| file.flush()) {
                    tracing::error!(%err, path = %self.path.display(), "failed to write journal");
                }
            }
            Err(err) => tracing::error!(%err, "failed to serialise a journal event"),
        }
    }

    /// Read a journal back, skipping lines that cannot be parsed.
    pub fn read(path: impl AsRef<Path>) -> Result<Vec<Event>> {
        let path = path.as_ref();
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut events = Vec::new();
        for (number, line) in BufReader::new(file).lines().enumerate() {
            let line = line.with_context(|| format!("reading {}", path.display()))?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(event) => events.push(event),
                Err(err) => {
                    tracing::warn!(line = number + 1, %err, "skipping malformed journal line")
                }
            }
        }
        Ok(events)
    }
}

/// A summary of a journal, as printed by `mercbot report`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub fills: usize,
    pub buys: usize,
    pub sells: usize,
    pub gp_spent: f64,
    pub gp_received: f64,
    pub fees_gp: f64,
    pub realized_pnl: f64,
    pub rejects: usize,
    pub errors: usize,
    /// Rejections by rule, most frequent first.
    pub reject_rules: Vec<(String, usize)>,
    /// Fills by market, most active first.
    pub market_activity: Vec<(String, usize)>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
}

impl Report {
    /// Summarise a journal.
    pub fn from_events(events: &[Event]) -> Self {
        use std::collections::HashMap;
        let mut report = Report::default();
        let mut rules: HashMap<String, usize> = HashMap::new();
        let mut markets: HashMap<String, usize> = HashMap::new();

        for event in events {
            let ts = match event {
                Event::Start { ts, .. }
                | Event::Tick { ts, .. }
                | Event::Snapshot { ts, .. }
                | Event::Reject { ts, .. }
                | Event::Error { ts, .. }
                | Event::Stop { ts, .. } => *ts,
                Event::Fill(fill) => fill.ts,
            };
            report.first_ts = Some(report.first_ts.map_or(ts, |first| first.min(ts)));
            report.last_ts = Some(report.last_ts.map_or(ts, |last| last.max(ts)));

            match event {
                Event::Fill(fill) => {
                    report.fills += 1;
                    report.fees_gp += fill.fee_gp;
                    *markets.entry(fill.market.clone()).or_default() += 1;
                    match fill.side {
                        mercantile_dex::quote::Side::Buy => {
                            report.buys += 1;
                            report.gp_spent += fill.gp;
                        }
                        mercantile_dex::quote::Side::Sell => {
                            report.sells += 1;
                            report.gp_received += fill.gp;
                        }
                    }
                }
                Event::Reject { reason, .. } => {
                    report.rejects += 1;
                    *rules.entry(reason.rule.clone()).or_default() += 1;
                }
                Event::Error { .. } => report.errors += 1,
                // The last tick carries the authoritative running P&L.
                Event::Tick { realized_pnl, .. } => report.realized_pnl = *realized_pnl,
                Event::Stop { realized_pnl, .. } => report.realized_pnl = *realized_pnl,
                _ => {}
            }
        }

        report.reject_rules = sorted_by_count(rules);
        report.market_activity = sorted_by_count(markets);
        report
    }
}

fn sorted_by_count(map: std::collections::HashMap<String, usize>) -> Vec<(String, usize)> {
    let mut rows: Vec<_> = map.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use mercantile_dex::quote::Side;

    fn fill(market: &str, side: Side, gp: f64) -> Event {
        Event::Fill(Box::new(Fill {
            ts: 100,
            market: market.to_string(),
            side,
            items: 1,
            gp,
            price: gp,
            fee_gp: gp * 0.01,
            signature: None,
            paper: true,
            strategy: "alch-floor".to_string(),
        }))
    }

    #[test]
    fn events_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("mercbot-journal-{}", std::process::id()));
        let path = dir.join("journal.jsonl");
        let _ = std::fs::remove_dir_all(&dir);

        let mut journal = Journal::open(&path).unwrap();
        journal.record(&Event::Start {
            ts: 1,
            mode: "paper".to_string(),
            markets: 3,
            strategies: vec!["alch-floor".to_string()],
            gp: 1_000.0,
        });
        journal.record(&fill("lobster", Side::Buy, 250.0));
        drop(journal);

        let events = Journal::read(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Start { markets: 3, .. }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_discarding_journal_is_harmless() {
        let mut journal = Journal::none();
        journal.record(&fill("lobster", Side::Buy, 1.0));
    }

    #[test]
    fn reports_add_up_the_run() {
        let events = vec![
            fill("lobster", Side::Buy, 200.0),
            fill("lobster", Side::Sell, 260.0),
            fill("shark", Side::Buy, 100.0),
            Event::Reject {
                ts: 120,
                reason: RejectReason {
                    market: "shark".to_string(),
                    strategy: "grid".to_string(),
                    rule: "cooldown".to_string(),
                    detail: "too soon".to_string(),
                },
            },
            Event::Tick {
                ts: 130,
                markets: 2,
                signals: 3,
                fills: 3,
                gp: 900.0,
                equity: 1_010.0,
                realized_pnl: 60.0,
            },
        ];
        let report = Report::from_events(&events);
        assert_eq!(report.fills, 3);
        assert_eq!(report.buys, 2);
        assert_eq!(report.sells, 1);
        assert!((report.gp_spent - 300.0).abs() < 1e-9);
        assert!((report.gp_received - 260.0).abs() < 1e-9);
        assert!((report.realized_pnl - 60.0).abs() < 1e-9);
        assert_eq!(report.rejects, 1);
        assert_eq!(report.reject_rules, vec![("cooldown".to_string(), 1)]);
        assert_eq!(report.market_activity[0], ("lobster".to_string(), 2));
        assert_eq!(report.first_ts, Some(100));
        assert_eq!(report.last_ts, Some(130));
    }
}
