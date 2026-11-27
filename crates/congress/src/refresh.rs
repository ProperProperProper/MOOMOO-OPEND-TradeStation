//! Incremental ingest: discover new PTRs on both chambers' official sites,
//! parse them, normalise into SQLite and raise alerts for tracked politicians.

use std::future::Future;
use anyhow::{Context, Result, bail};
use chrono::{Datelike, NaiveDate, Utc};
use futures::{StreamExt, stream};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::db::{Db, FilingRecord, FilingStatus, NewPolitician};
use crate::house::{self, IndexEntry, PdfError};
pub use crate::model::NewTradeAlert;
use crate::model::{Chamber, Party};
use crate::roster::{self, Roster};
use crate::senate::{self, ReportKind, ReportRef};

/// Where filings come from. [`HttpSource`] talks to the official sites; tests
/// substitute fixtures.
pub trait Source: Sync {
    fn rosters(&self) -> impl Future<Output = Result<Roster>> + Send;
    fn house_index(&self, year: i32) -> impl Future<Output = Result<Vec<IndexEntry>>> + Send;
    fn house_pdf(&self, entry: &IndexEntry) -> impl Future<Output = Result<Vec<u8>>> + Send;
    fn senate_reports(&self, since: Option<NaiveDate>) -> impl Future<Output = Result<Vec<ReportRef>>> + Send;
    fn senate_ptr(&self, report: &ReportRef) -> impl Future<Output = Result<String>> + Send;
}

#[derive(Debug, Clone)]
pub struct RefreshConfig {
    /// House index years to scan (current year, plus last year on first run).
    pub house_years: Vec<i32>,
    /// Only Senate reports submitted on or after this date are listed.
    pub senate_since: Option<NaiveDate>,
    /// Cap on new filings fetched per chamber per run (keeps first runs polite).
    pub max_new_per_chamber: usize,
    pub concurrency: usize,
}

impl RefreshConfig {
    /// Sensible defaults derived from the database's last successful run.
    pub fn incremental(db: &Db) -> Self {
        let today = Utc::now().date_naive();
        let last: Option<NaiveDate> = db.meta("last_refresh_date").ok().flatten().and_then(|s| s.parse().ok());
        let (house_years, senate_since) = match last {
            // Re-scan a week back: filings can appear in the index late.
            Some(d) => {
                let since = d - chrono::Duration::days(7);
                let mut years = vec![today.year()];
                if since.year() != today.year() {
                    years.insert(0, since.year());
                }
                (years, Some(since))
            }
            None => (vec![today.year() - 1, today.year()], NaiveDate::from_ymd_opt(today.year() - 1, 1, 1)),
        };
        Self { house_years, senate_since, max_new_per_chamber: 2_000, concurrency: 4 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RefreshReport {
    pub new_filings: usize,
    pub new_trades: usize,
    pub scanned: usize,
    pub errors: Vec<String>,
    pub alerts: Vec<NewTradeAlert>,
}

impl RefreshReport {
    pub fn summary(&self) -> String {
        format!(
            "{} new filings, {} trades, {} scanned/paper, {} errors, {} alerts",
            self.new_filings,
            self.new_trades,
            self.scanned,
            self.errors.len(),
            self.alerts.len()
        )
    }
}

struct Parsed {
    record_paper: bool,
    status: FilingStatus,
    error: Option<String>,
    filed: Option<NaiveDate>,
    trades: Vec<crate::model::ParsedTrade>,
}

/// Run one refresh cycle against `src`, writing into `db`.
pub async fn refresh<S: Source>(src: &S, db: &Mutex<Db>, cfg: &RefreshConfig) -> Result<RefreshReport> {
    let mut report = RefreshReport::default();
    let roster = match src.rosters().await {
        Ok(r) => r,
        Err(e) => {
            warn!("roster fetch failed, parties will be unknown: {e:#}");
            report.errors.push(format!("roster: {e:#}"));
            Roster::default()
        }
    };

    if !roster.is_empty() {
        let members: Vec<_> = roster
            .house
            .iter()
            .chain(&roster.senate)
            .filter_map(|m| m.bioguide.clone().map(|b| (b, m.name.clone(), m.party, Some(m.state.clone()))))
            .collect();
        match db.lock().await.apply_roster(&members) {
            Ok(n) if n > 0 => info!("merged {n} duplicate politician rows via roster"),
            Ok(_) => {}
            Err(e) => warn!("roster sync failed: {e:#}"),
        }
    }
    if let Err(e) = refresh_house(src, db, cfg, &roster, &mut report).await {
        warn!("house refresh failed: {e:#}");
        report.errors.push(format!("house: {e:#}"));
    }
    if let Err(e) = refresh_senate(src, db, cfg, &roster, &mut report).await {
        warn!("senate refresh failed: {e:#}");
        report.errors.push(format!("senate: {e:#}"));
    }

    let db = db.lock().await;
    db.set_meta("last_refresh_date", &Utc::now().date_naive().to_string())?;
    db.set_meta("last_refresh_summary", &report.summary())?;
    info!("congress refresh: {}", report.summary());
    Ok(report)
}

async fn store(
    db: &Mutex<Db>,
    report: &mut RefreshReport,
    politician: NewPolitician,
    record: impl FnOnce(i64) -> FilingRecord,
    parsed: Parsed,
) -> Result<()> {
    let mut db = db.lock().await;
    let pid = db.upsert_politician(&politician)?;
    let mut rec = record(pid);
    rec.paper = parsed.record_paper;
    rec.status = parsed.status;
    rec.error = parsed.error.clone();
    if let Some(f) = parsed.filed {
        rec.filed_date = f;
    }
    let n = db.insert_filing(&rec, &parsed.trades)?;
    report.new_filings += 1;
    report.new_trades += n;
    match parsed.status {
        FilingStatus::Scanned => report.scanned += 1,
        FilingStatus::Error => report.errors.push(format!("{}: {}", rec.id, parsed.error.unwrap_or_default())),
        FilingStatus::Parsed => {}
    }
    if db.is_tracked(pid)? && parsed.status != FilingStatus::Error {
        let tickers: Vec<String> = {
            let mut t: Vec<String> = parsed
                .trades
                .iter()
                .map(|t| {
                    let what = t.ticker.clone().unwrap_or_else(|| t.asset_name.chars().take(18).collect());
                    format!("{} {what}", t.tx_type.marker())
                })
                .collect();
            t.dedup();
            t.truncate(6);
            t
        };
        let message = if parsed.trades.is_empty() {
            format!(
                "{} filed a new PTR ({}) on {}",
                politician.name,
                if rec.paper { "paper" } else { "scanned" },
                rec.filed_date
            )
        } else {
            format!("{} filed {} trade(s): {}", politician.name, parsed.trades.len(), tickers.join(", "))
        };
        if db.insert_alert(pid, &rec.id, &message)?.is_some() {
            report.alerts.push(NewTradeAlert {
                politician_id: pid,
                politician: politician.name.clone(),
                filing_id: rec.id.clone(),
                message,
            });
        }
    }
    Ok(())
}

async fn refresh_house<S: Source>(
    src: &S,
    db: &Mutex<Db>,
    cfg: &RefreshConfig,
    roster: &Roster,
    report: &mut RefreshReport,
) -> Result<()> {
    let known = db.lock().await.known_filing_ids(Chamber::House)?;
    let mut todo: Vec<IndexEntry> = Vec::new();
    for &year in &cfg.house_years {
        let entries = src.house_index(year).await.with_context(|| format!("house index {year}"))?;
        todo.extend(entries.into_iter().filter(|e| e.is_ptr() && !known.contains(&format!("house:{}", e.doc_id))));
    }
    // Newest first so a capped run still gets the freshest filings.
    todo.sort_by_key(|e| std::cmp::Reverse(e.filing_date));
    todo.truncate(cfg.max_new_per_chamber);
    debug!("house: {} new PTRs to fetch", todo.len());

    let mut results = stream::iter(todo.into_iter().map(|e| async move {
        let res = src.house_pdf(&e).await;
        (e, res)
    }))
    .buffer_unordered(cfg.concurrency.max(1));

    while let Some((entry, pdf)) = results.next().await {
        let parsed = match pdf {
            Err(e) => {
                // Network errors are not stored, so the filing is retried next run.
                report.errors.push(format!("house:{} fetch: {e:#}", entry.doc_id));
                continue;
            }
            Ok(bytes) => {
                let bytes_owned = bytes;
                match tokio::task::spawn_blocking(move || house::parse_ptr_pdf(&bytes_owned)).await? {
                    Ok(ptr) => Parsed {
                        record_paper: false,
                        status: FilingStatus::Parsed,
                        error: None,
                        filed: entry.filing_date.or(ptr.signed_date),
                        trades: ptr.trades,
                    },
                    Err(PdfError::Scanned) => Parsed {
                        record_paper: true,
                        status: FilingStatus::Scanned,
                        error: None,
                        filed: entry.filing_date,
                        trades: vec![],
                    },
                    Err(e) => Parsed {
                        record_paper: false,
                        status: FilingStatus::Error,
                        error: Some(e.to_string()),
                        filed: entry.filing_date,
                        trades: vec![],
                    },
                }
            }
        };
        let member = roster
            .house_by_district(&entry.state_district)
            .filter(|m| roster::normalize_name(&m.last).last() == roster::normalize_name(&entry.last).last())
            .or_else(|| roster.find_by_name(Chamber::House, &entry.first, &entry.last));
        let politician = NewPolitician {
            // The filing index has typos ("Scott Scott Franklin"); prefer the roster's name.
            name: member.map(|m| m.name.clone()).unwrap_or_else(|| entry.display_name()),
            chamber: Chamber::House,
            party: member.map(|m| m.party).unwrap_or(Party::Unknown),
            state: entry.state(),
            district: Some(entry.state_district.clone()).filter(|s| !s.is_empty()),
            bioguide: member.and_then(|m| m.bioguide.clone()),
        };
        let filed = entry.filing_date.unwrap_or_else(|| Utc::now().date_naive());
        let url = entry.pdf_url();
        let id = format!("house:{}", entry.doc_id);
        store(
            db,
            report,
            politician,
            |pid| FilingRecord {
                id,
                chamber: Chamber::House,
                politician_id: pid,
                filed_date: filed,
                url,
                paper: false,
                status: FilingStatus::Parsed,
                error: None,
                is_amendment: false,
            },
            parsed,
        )
        .await?;
    }
    Ok(())
}

async fn refresh_senate<S: Source>(
    src: &S,
    db: &Mutex<Db>,
    cfg: &RefreshConfig,
    roster: &Roster,
    report: &mut RefreshReport,
) -> Result<()> {
    let known = db.lock().await.known_filing_ids(Chamber::Senate)?;
    let mut todo: Vec<ReportRef> = src
        .senate_reports(cfg.senate_since)
        .await?
        .into_iter()
        .filter(|r| !known.contains(&format!("senate:{}", r.id)))
        .collect();
    todo.sort_by_key(|r| std::cmp::Reverse(r.date_received));
    todo.truncate(cfg.max_new_per_chamber);
    debug!("senate: {} new PTRs to fetch", todo.len());

    let mut results = stream::iter(todo.into_iter().map(|r| async move {
        let res = match r.kind {
            ReportKind::Electronic => src.senate_ptr(&r).await.map(Some),
            ReportKind::Paper => Ok(None),
        };
        (r, res)
    }))
    .buffer_unordered(cfg.concurrency.max(1));

    while let Some((r, html)) = results.next().await {
        let parsed = match html {
            Err(e) => {
                report.errors.push(format!("senate:{} fetch: {e:#}", r.id));
                continue;
            }
            Ok(None) => Parsed {
                record_paper: true,
                status: FilingStatus::Scanned,
                error: None,
                filed: r.date_received,
                trades: vec![],
            },
            Ok(Some(html)) => match senate::parse_ptr_html(&html) {
                Ok(p) => Parsed {
                    record_paper: false,
                    status: FilingStatus::Parsed,
                    error: None,
                    filed: p.filed_date.or(r.date_received),
                    trades: p.trades,
                },
                Err(e) => Parsed {
                    record_paper: false,
                    status: FilingStatus::Error,
                    error: Some(format!("{e:#}")),
                    filed: r.date_received,
                    trades: vec![],
                },
            },
        };
        let member = roster.find_by_name(Chamber::Senate, &r.first, &r.last);
        let politician = NewPolitician {
            name: member.map(|m| m.name.clone()).unwrap_or_else(|| r.display_name()),
            chamber: Chamber::Senate,
            party: member.map(|m| m.party).unwrap_or(Party::Unknown),
            state: member.map(|m| m.state.clone()),
            district: None,
            bioguide: member.and_then(|m| m.bioguide.clone()),
        };
        let filed = r.date_received.unwrap_or_else(|| Utc::now().date_naive());
        let (id, url, amend) = (format!("senate:{}", r.id), r.url.clone(), r.is_amendment());
        store(
            db,
            report,
            politician,
            |pid| FilingRecord {
                id,
                chamber: Chamber::Senate,
                politician_id: pid,
                filed_date: filed,
                url,
                paper: false,
                status: FilingStatus::Parsed,
                error: None,
                is_amendment: amend,
            },
            parsed,
        )
        .await?;
    }
    Ok(())
}

/// Live source backed by the official websites.
#[derive(Default)]
pub struct HttpSource;

impl HttpSource {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl Source for HttpSource {
    async fn rosters(&self) -> Result<Roster> {
        // Disabled external network requests: application is restricted to local OpenD only.
        Ok(Roster::default())
    }

    async fn house_index(&self, _year: i32) -> Result<Vec<IndexEntry>> {
        Ok(Vec::new())
    }

    async fn house_pdf(&self, _entry: &IndexEntry) -> Result<Vec<u8>> {
        bail!("External PDF downloading disabled")
    }

    async fn senate_reports(&self, _since: Option<NaiveDate>) -> Result<Vec<ReportRef>> {
        Ok(Vec::new())
    }

    async fn senate_ptr(&self, _report: &ReportRef) -> Result<String> {
        bail!("External Senate scraping disabled")
    }
}
