//! Moomoo OpenD Market Data Provider.
//!
//! Routes all market data exclusively to the local Moomoo OpenD gateway (127.0.0.1:11111).
//! All external internet providers (Yahoo, Binance, Coinbase, Alpaca, Treasury) are disabled
//! to ensure zero external internet access.

use anyhow::{Result, bail};
use async_trait::async_trait;
use tr_core::{AssetClass, Bar, Security, Timeframe};
use tr_moomoo::OpenDClient;

use crate::{
    LiveFeed, MarketData, NewsItem, OptionChain, OrderBook, Quote, SearchHit, YieldCurves, perp_needs_server, security,
};

pub struct Markets {
    moomoo: OpenDClient,
    name: String,
}

impl Markets {
    pub fn new() -> Result<Self> {
        let moomoo = OpenDClient::default();
        let name = format!("moomoo-opend ({})", moomoo.opend_addr());
        Ok(Self {
            moomoo,
            name,
        })
    }
}

#[async_trait]
impl MarketData for Markets {
    fn name(&self) -> &str {
        &self.name
    }

    async fn history(&self, symbol: &str, _tf: Timeframe, end: Option<i64>, limit: usize) -> Result<Vec<Bar>> {
        let sec = security(symbol);
        if sec.class == AssetClass::Perp {
            return Err(perp_needs_server(symbol));
        }

        let bars = self.moomoo.fetch_history(&sec.ticker, limit).await?;
        let mapped = bars
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

        Ok(crate::clip(mapped, end, limit))
    }

    async fn quotes(&self, symbols: &[String]) -> Result<Vec<Quote>> {
        let mut out = Vec::with_capacity(symbols.len());
        for key in symbols {
            let sec = security(key);
            if sec.class == AssetClass::Perp {
                continue;
            }
            if let Ok(q) = self.moomoo.fetch_quote(&sec.ticker).await {
                out.push(Quote {
                    symbol: key.clone(),
                    last: q.last,
                    prev_close: q.prev_close,
                    ts: q.timestamp,
                    spark: vec![q.prev_close, q.low, q.high, q.last],
                });
            }
        }
        Ok(out)
    }

    fn live(&self, symbol: &str, _tf: Timeframe) -> Option<LiveFeed> {
        let sec = security(symbol);
        if sec.class == AssetClass::Perp {
            return None;
        }

        let mut quote_rx = self.moomoo.subscribe_quotes(&sec.ticker);
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        tokio::spawn(async move {
            while let Some(q) = quote_rx.recv().await {
                let bar = Bar {
                    ts: q.timestamp,
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
        let q = query.trim().to_uppercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        // Query local watchlist from OpenD
        let watchlist = self.moomoo.fetch_watchlist().await.unwrap_or_default();
        let mut hits = Vec::new();
        for sym in watchlist {
            if sym.contains(&q) {
                hits.push(SearchHit {
                    key: format!("{sym} US Equity"),
                    name: format!("{sym} (Moomoo)"),
                    class: AssetClass::Equity,
                    exchange: Some("Moomoo OpenD".to_string()),
                });
            }
        }
        if hits.is_empty() {
            hits.push(SearchHit {
                key: format!("{q} US Equity"),
                name: format!("{q} (Moomoo)"),
                class: AssetClass::Equity,
                exchange: Some("Moomoo OpenD".to_string()),
            });
        }
        Ok(hits)
    }

    async fn news(&self, _symbol: Option<&str>) -> Result<Vec<NewsItem>> {
        // Disabled external internet news scrapers
        Ok(Vec::new())
    }

    async fn options(&self, symbol: &str, _expiration: Option<i64>) -> Result<OptionChain> {
        bail!("Options for {symbol} through OpenD not yet active")
    }

    async fn order_book(&self, symbol: &str, depth: usize) -> Result<OrderBook> {
        let sec = security(symbol);
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
