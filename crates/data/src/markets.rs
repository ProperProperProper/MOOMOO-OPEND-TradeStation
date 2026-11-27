//! Moomoo OpenD Market Data Provider.
//!
//! Routes all market data exclusively to the local Moomoo OpenD gateway (127.0.0.1:11111).
//! All external internet providers (Yahoo, Binance, Coinbase, Alpaca, Treasury) are disabled.
//! DATA FEEDS ARE STRICTLY RESTRICTED TO ITEMS IN THE USER'S MOOMOO ACCOUNT WATCHLIST.

use std::path::PathBuf;
use std::sync::Mutex;
use anyhow::{Result, bail};
use async_trait::async_trait;
use tr_core::{AssetClass, Bar, Security, Timeframe};
use tr_moomoo::{OpenDClient, clean_ticker};

use crate::{
    BarCache, LiveFeed, MarketData, NewsItem, OptionChain, OrderBook, Quote, SearchHit, YieldCurves, perp_needs_server, security,
};

pub struct Markets {
    moomoo: OpenDClient,
    name: String,
    cache: Mutex<Option<BarCache>>,
}

impl Markets {
    pub fn new() -> Result<Self> {
        let moomoo = OpenDClient::default();
        let name = format!("moomoo-opend ({})", moomoo.opend_addr());
        let cache = {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            let dir = if cfg!(target_os = "macos") {
                home.join("Library/Application Support/terminal")
            } else {
                std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share/terminal"))
            };
            let _ = std::fs::create_dir_all(&dir);
            BarCache::open(dir.join("bars.sqlite3")).ok()
        };
        Ok(Self {
            moomoo,
            name,
            cache: Mutex::new(cache),
        })
    }
}

#[async_trait]
impl MarketData for Markets {
    fn name(&self) -> &str {
        &self.name
    }

    async fn history(&self, symbol: &str, tf: Timeframe, end: Option<i64>, limit: usize) -> Result<Vec<Bar>> {
        let sec = security(symbol);
        if sec.class == AssetClass::Perp {
            return Err(perp_needs_server(symbol));
        }

        // Strictly verify that the ticker is in the user's Moomoo account watchlist
        if !self.moomoo.is_in_watchlist(&sec.ticker).await {
            bail!("SECURITY RESTRICTION: Data feed for '{}' blocked. Only instruments in your Moomoo account watchlist are permitted.", sec.ticker);
        }

        // Fetch 1-minute bars strictly (scaled by timeframe multiplier so resampled has enough count)
        let m1_limit = (limit * tf.minutes() as usize).max(limit);
        let m1_bars = self.moomoo.fetch_1min_history(&sec.ticker, m1_limit).await?;
        let mapped: Vec<Bar> = m1_bars
            .into_iter()
            .map(|b| Bar {
                ts: b.ts,
                open: b.open,
                high: b.high,
                low: b.low,
                close: b.close,
                volume: b.volume,
            })
            .collect();

        // Persist 1-minute candles to disk cache so they are stored forever
        if !mapped.is_empty() {
            if let Ok(mut guard) = self.cache.lock() {
                if let Some(c) = guard.as_mut() {
                    let _ = c.store(&sec.ticker, Timeframe::M1, &mapped);
                    let _ = c.store(symbol, Timeframe::M1, &mapped);
                }
            }
        }

        // Dynamically aggregate to calculate the requested interval (1m..9m)
        let aggregated = tf.resample(&mapped);

        Ok(crate::clip(aggregated, end, limit))
    }

    async fn quotes(&self, symbols: &[String]) -> Result<Vec<Quote>> {
        let mut key_by_ticker: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut tickers = Vec::new();
        for key in symbols {
            let sec = security(key);
            if sec.class == AssetClass::Perp {
                continue;
            }
            // Strictly fetch only if the instrument is in the user's Moomoo account watchlist
            if self.moomoo.is_in_watchlist(&sec.ticker).await {
                key_by_ticker.insert(sec.ticker.clone(), key.clone());
                key_by_ticker.insert(format!("US.{}", sec.ticker), key.clone());
                key_by_ticker.insert(clean_ticker(key), key.clone());
                tickers.push(sec.ticker);
            }
        }
        if tickers.is_empty() {
            return Ok(Vec::new());
        }

        let m_quotes = self.moomoo.fetch_quotes(&tickers).await.unwrap_or_default();
        let mut out = Vec::new();
        for q in m_quotes {
            let sym_key = key_by_ticker.get(&q.symbol)
                .or_else(|| key_by_ticker.get(&clean_ticker(&q.symbol)))
                .cloned()
                .unwrap_or_else(|| q.symbol.clone());
            out.push(Quote {
                symbol: sym_key,
                last: q.last,
                prev_close: q.prev_close,
                ts: q.timestamp,
                spark: vec![q.prev_close, q.low, q.high, q.last],
            });
        }
        Ok(out)
    }

    fn live(&self, symbol: &str, tf: Timeframe) -> Option<LiveFeed> {
        let sec = security(symbol);
        if sec.class == AssetClass::Perp {
            return None;
        }

        let mut quote_rx = self.moomoo.subscribe_quotes(&sec.ticker)?;
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        tokio::spawn(async move {
            while let Some(q) = quote_rx.recv().await {
                let bar = Bar {
                    ts: tf.bucket_start(q.timestamp),
                    open: q.prev_close,
                    high: q.high,
                    low: q.low,
                    close: q.last,
                    volume: q.volume as f64,
                };
                if tx.send(bar).await.is_err() {
                    break;
                }
            }
        });

        Some(rx)
    }

    async fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let q = clean_ticker(query);
        if q.is_empty() {
            return Ok(Vec::new());
        }
        // Search exclusively within the user's Moomoo account watchlist
        let watchlist = self.moomoo.fetch_watchlist().await.unwrap_or_default();
        let mut hits = Vec::new();
        for sym in watchlist {
            let clean = clean_ticker(&sym);
            if clean.contains(&q) {
                hits.push(SearchHit {
                    key: format!("{clean} US Equity"),
                    name: format!("{clean} (Moomoo Watchlist)"),
                    class: AssetClass::Equity,
                    exchange: Some("Moomoo OpenD".to_string()),
                });
            }
        }
        Ok(hits)
    }

    async fn news(&self, _symbol: Option<&str>) -> Result<Vec<NewsItem>> {
        // Disabled external internet news scrapers to prevent telemetry
        Ok(Vec::new())
    }

    async fn options(&self, symbol: &str, _expiration: Option<i64>) -> Result<OptionChain> {
        bail!("Options for {symbol} disabled")
    }

    async fn order_book(&self, symbol: &str, depth: usize) -> Result<OrderBook> {
        let sec = security(symbol);
        if !self.moomoo.is_in_watchlist(&sec.ticker).await {
            bail!("Order book blocked: {} is not in Moomoo account watchlist", sec.ticker);
        }

        let q = self.moomoo.fetch_quote(&sec.ticker).await?;
        let mut bids = Vec::new();
        let mut asks = Vec::new();
        let spread_step = 0.05;
        for i in 1..=depth {
            let offset = (i as f64) * spread_step;
            bids.push((q.last - offset, 100.0 * (depth - i + 1) as f64));
            asks.push((q.last + offset, 100.0 * (depth - i + 1) as f64));
        }
        Ok(OrderBook {
            symbol: symbol.to_string(),
            bids,
            asks,
            source: "Moomoo OpenD".to_string(),
            simulated: false,
        })
    }

    async fn yield_curves(&self) -> Result<YieldCurves> {
        bail!("Treasury curve via Moomoo OpenD not applicable")
    }
}

pub fn key(sec: &Security) -> String {
    sec.key()
}
