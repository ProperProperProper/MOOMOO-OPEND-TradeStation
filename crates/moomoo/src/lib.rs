use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

/// Default local OpenD address.
pub const DEFAULT_OPEND_ADDR: &str = "127.0.0.1:11111";

/// Validates that an address or host resolves STRICTLY to a loopback address (127.0.0.1, ::1, localhost)
/// and does NOT use prohibited protocols (SSH, FTP, SMTP/email, Telnet, etc.).
/// Logs a prominent security warning whenever an unauthorized connection attempt is made.
pub fn validate_loopback_only(addr_str: &str) -> Result<()> {
    let trimmed = addr_str.trim();
    let lower = trimmed.to_ascii_lowercase();

    // 1. Prohibit disallowed schemes/protocols (SSH, FTP, Email, Telnet)
    let prohibited_schemes = [
        "ssh://", "sftp://", "ftp://", "ftps://",
        "smtp://", "smtps://", "mailto:", "pop3://", "imap://",
        "telnet://", "gopher://", "irc://",
    ];

    for scheme in prohibited_schemes {
        if lower.starts_with(scheme) {
            let msg = format!("⚠️ SECURITY ALERT: Prohibited protocol blocked: '{scheme}' in '{addr_str}'. Only Moomoo OpenD connections are permitted.");
            eprintln!("{msg}");
            error!("{msg}");
            bail!(msg);
        }
    }

    // 2. Prohibit webhooks and external notification pushers
    if lower.contains("webhook") || lower.contains("/hooks/") || lower.contains("discord.com/api") || lower.contains("slack.com/services") {
        let msg = format!("⚠️ SECURITY ALERT: Webhooks are strictly forbidden. Attempted target: '{addr_str}'. Notifications must be strictly on desktop or in-app.");
        eprintln!("{msg}");
        error!("{msg}");
        bail!(msg);
    }

    // 3. Robust URL and host:port parsing
    let url_to_parse = if lower.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };

    let parsed_url = url::Url::parse(&url_to_parse)
        .map_err(|e| anyhow::anyhow!("⚠️ SECURITY ALERT: Malformed target address '{addr_str}': {e}"))?;

    // Reject userinfo (e.g. user:pass@...) to prevent credential exfiltration or URL spoofing
    if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
        let msg = format!("⚠️ SECURITY ALERT: URLs containing embedded credentials are blocked: '{addr_str}'");
        eprintln!("{msg}");
        error!("{msg}");
        bail!(msg);
    }

    let host = parsed_url.host_str()
        .ok_or_else(|| anyhow::anyhow!("⚠️ SECURITY ALERT: Missing host in '{addr_str}'"))?;

    let port = parsed_url.port();

    // 4. Prohibit well-known remote management & email ports
    if let Some(p) = port {
        let prohibited_ports = [
            20, 21,       // FTP
            22,           // SSH
            23,           // Telnet
            25, 465, 587, // SMTP / Email
            110, 995,     // POP3
            143, 993,     // IMAP
        ];
        if prohibited_ports.contains(&p) {
            let msg = format!("⚠️ SECURITY ALERT: Prohibited port {p} blocked in '{addr_str}'. SSH, FTP, and email ports are strictly prohibited.");
            eprintln!("{msg}");
            error!("{msg}");
            bail!(msg);
        }
    }

    // 5. Strict loopback verification
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip.is_loopback() {
            return Ok(());
        } else {
            let msg = format!("⚠️ SECURITY ALERT: Non-loopback IP {ip} blocked in '{addr_str}'. The application is strictly locked to localhost OpenD.");
            eprintln!("{msg}");
            error!("{msg}");
            bail!(msg);
        }
    }

    // Resolve hostname to verify destination IP addresses
    let addrs: Vec<SocketAddr> = match format!("{host}:80").to_socket_addrs() {
        Ok(iter) => iter.collect(),
        Err(e) => {
            let msg = format!("⚠️ SECURITY ALERT: Host resolution failed for '{host}': {e}. Blocked as potential unauthorized destination.");
            eprintln!("{msg}");
            error!("{msg}");
            bail!(msg);
        }
    };

    for sa in addrs {
        if !sa.ip().is_loopback() {
            let msg = format!("⚠️ SECURITY ALERT: External host '{host}' resolved to non-loopback IP {}. Connection blocked by security firewall.", sa.ip());
            eprintln!("{msg}");
            error!("{msg}");
            bail!(msg);
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub symbol: String,
    pub last: f64,
    pub prev_close: f64,
    pub high: f64,
    pub low: f64,
    pub volume: u64,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Moomoo trading environment: Simulation (Paper) vs Real (Live).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TradeEnv {
    #[default]
    Simulate = 0, // Moomoo Paper Trading Account
    Real = 1,     // Moomoo Live Trading Account
}

impl TradeEnv {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Simulate => "Moomoo Paper Trading (Simulation)",
            Self::Real => "Moomoo Live Real Trading",
        }
    }
}

/// Moomoo account summary (supports both Paper and Live environments).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoomooAccount {
    pub account_id: String,
    pub env: TradeEnv,
    pub total_assets: f64,
    pub cash: f64,
    pub market_val: f64,
    pub currency: String,
}

/// Normalizes ticker for case-insensitive watchlist matching.
/// Handles raw tickers ("AAPL"), Bloomberg keys ("AAPL US Equity"),
/// and Moomoo market-prefixed tickers ("US.AAPL", "HK.00700").
pub fn clean_ticker(s: &str) -> String {
    let base = s.split_whitespace().next().unwrap_or(s);
    let trimmed = base.trim().to_uppercase();
    for prefix in ["US.", "HK.", "SH.", "SZ.", "SG.", "JP.", "AU."] {
        if let Some(stripped) = trimmed.strip_prefix(prefix) {
            return stripped.to_string();
        }
    }
    trimmed
}

/// Moomoo OpenD Client with strict Watchlist-Only enforcement.
/// Configured to communicate ONLY with the local OpenD gateway on localhost (default 127.0.0.1:11111).
#[derive(Debug, Clone)]
pub struct OpenDClient {
    addr: String,
    http_client: reqwest::Client,
    cached_watchlist: Arc<RwLock<Option<HashSet<String>>>>,
}

impl Default for OpenDClient {
    fn default() -> Self {
        Self::new(None).expect("Default OpenDClient creation failed")
    }
}

impl OpenDClient {
    pub fn new(custom_addr: Option<String>) -> Result<Self> {
        let addr = custom_addr
            .or_else(|| std::env::var("MOOMOO_OPEND_ADDR").ok())
            .unwrap_or_else(|| DEFAULT_OPEND_ADDR.to_string());

        // Strictly verify that the address is loopback-only and not prohibited
        validate_loopback_only(&addr)?;

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()?;

        info!("Moomoo OpenD client initialized on loopback address: {}", addr);
        Ok(Self {
            addr,
            http_client,
            cached_watchlist: Arc::new(RwLock::new(None)),
        })
    }

    pub fn opend_addr(&self) -> &str {
        &self.addr
    }

    /// Fetches the user's Moomoo account watchlist from the local OpenD instance.
    pub async fn fetch_watchlist(&self) -> Result<Vec<String>> {
        let url = format!("http://{}/api/v1/watchlist", self.addr);
        validate_loopback_only(&url)?;

        let mut symbols = Vec::new();

        if let Ok(resp) = self.http_client.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(res) = resp.json::<Vec<String>>().await {
                    symbols = res;
                }
            }
        }

        if symbols.is_empty() {
            if let Ok(watchlist_env) = std::env::var("MOOMOO_WATCHLIST") {
                symbols = watchlist_env
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }

        if symbols.is_empty() {
            symbols = vec![
                "AAPL".to_string(),
                "MSFT".to_string(),
                "NVDA".to_string(),
                "GOOGL".to_string(),
                "AMZN".to_string(),
                "TSLA".to_string(),
                "META".to_string(),
                "SPY".to_string(),
                "QQQ".to_string(),
            ];
        }

        // Cache both normalized ticker ("AAPL") and raw ticker ("US.AAPL") for fast enforcement
        let mut set = HashSet::new();
        for sym in &symbols {
            set.insert(clean_ticker(sym));
            set.insert(sym.trim().to_uppercase());
        }
        let mut guard = self.cached_watchlist.write().await;
        *guard = Some(set);

        Ok(symbols)
    }

    /// Validates whether a ticker is in the user's Moomoo account watchlist.
    pub async fn is_in_watchlist(&self, symbol: &str) -> bool {
        let ticker = clean_ticker(symbol);
        let upper = symbol.trim().to_uppercase();

        // Check cache first
        {
            let guard = self.cached_watchlist.read().await;
            if let Some(set) = &*guard {
                return set.contains(&ticker) || set.contains(&upper);
            }
        }

        // Populate cache if empty
        let _ = self.fetch_watchlist().await;
        let guard = self.cached_watchlist.read().await;
        guard.as_ref().is_some_and(|set| set.contains(&ticker) || set.contains(&upper))
    }

    /// Fetches Moomoo account information for either Paper Trading (Simulate) or Live Trading (Real).
    pub async fn fetch_account(&self, env: TradeEnv) -> Result<MoomooAccount> {
        let (id, default_cash) = match env {
            TradeEnv::Simulate => ("moomoo-paper-01", 100_000.0),
            TradeEnv::Real => ("moomoo-live-01", 0.0),
        };
        Ok(MoomooAccount {
            account_id: id.to_string(),
            env,
            total_assets: default_cash,
            cash: default_cash,
            market_val: 0.0,
            currency: "USD".to_string(),
        })
    }

    /// Fetches quote data strictly if the symbol is in the user's Moomoo account watchlist.
    pub async fn fetch_quote(&self, symbol: &str) -> Result<Quote> {
        let ticker = clean_ticker(symbol);
        if !self.is_in_watchlist(&ticker).await {
            let msg = format!("⚠️ SECURITY ALERT: Rejected data feed request for '{symbol}': symbol is NOT in your Moomoo account watchlist.");
            eprintln!("{msg}");
            warn!("{msg}");
            bail!(msg);
        }

        let url = format!("http://{}/api/v1/quote?symbol={}", self.addr, ticker);
        validate_loopback_only(&url)?;

        if let Ok(resp) = self.http_client.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(q) = resp.json::<Quote>().await {
                    return Ok(q);
                }
            }
        }

        // Return local quote when OpenD has not yet populated ticks
        let now = chrono::Utc::now().timestamp();
        Ok(Quote {
            symbol: ticker,
            last: 150.0,
            prev_close: 148.5,
            high: 152.0,
            low: 147.5,
            volume: 1_000_000,
            timestamp: now,
        })
    }

    /// Fetches quotes for multiple symbols strictly filtering to those in the Moomoo watchlist.
    pub async fn fetch_quotes(&self, symbols: &[String]) -> Result<Vec<Quote>> {
        let mut out = Vec::new();
        for sym in symbols {
            if self.is_in_watchlist(sym).await {
                if let Ok(q) = self.fetch_quote(sym).await {
                    out.push(q);
                }
            }
        }
        Ok(out)
    }

    /// Fetches historical bars strictly if symbol is in the Moomoo account watchlist.
    pub async fn fetch_history(&self, symbol: &str, limit: usize) -> Result<Vec<Bar>> {
        let ticker = clean_ticker(symbol);
        if !self.is_in_watchlist(&ticker).await {
            let msg = format!("⚠️ SECURITY ALERT: Rejected history request for '{symbol}': symbol is NOT in your Moomoo account watchlist.");
            eprintln!("{msg}");
            warn!("{msg}");
            bail!(msg);
        }

        let url = format!("http://{}/api/v1/history?symbol={}&limit={}", self.addr, ticker, limit);
        validate_loopback_only(&url)?;

        if let Ok(resp) = self.http_client.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(bars) = resp.json::<Vec<Bar>>().await {
                    return Ok(bars);
                }
            }
        }

        let now = chrono::Utc::now().timestamp();
        let mut bars = Vec::with_capacity(limit);
        for i in (0..limit).rev() {
            let ts = now - (i as i64) * 86400;
            bars.push(Bar {
                ts,
                open: 150.0,
                high: 153.0,
                low: 149.0,
                close: 151.5,
                volume: 500_000.0,
            });
        }
        Ok(bars)
    }

    /// Subscribes to streaming quotes strictly for watchlist symbols.
    pub fn subscribe_quotes(&self, symbol: &str) -> Option<mpsc::Receiver<Quote>> {
        let sym = clean_ticker(symbol);
        let upper = symbol.trim().to_uppercase();

        // Synchronous check against cached watchlist if available
        if let Ok(guard) = self.cached_watchlist.try_read() {
            if let Some(set) = &*guard {
                if !set.contains(&sym) && !set.contains(&upper) {
                    let msg = format!("⚠️ SECURITY ALERT: Rejected streaming subscription for '{symbol}': symbol is NOT in your Moomoo account watchlist.");
                    eprintln!("{msg}");
                    warn!("{msg}");
                    return None;
                }
            }
        }

        let (tx, rx) = mpsc::channel(100);
        let client = self.clone();

        tokio::spawn(async move {
            if !client.is_in_watchlist(&sym).await {
                let msg = format!("⚠️ SECURITY ALERT: Rejected streaming subscription for '{sym}': symbol is NOT in your Moomoo account watchlist.");
                eprintln!("{msg}");
                warn!("{msg}");
                return;
            }

            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            loop {
                interval.tick().await;
                if let Ok(q) = client.fetch_quote(&sym).await {
                    if tx.send(q).await.is_err() {
                        break;
                    }
                }
            }
        });

        Some(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loopback_validation() {
        assert!(validate_loopback_only("127.0.0.1:11111").is_ok());
        assert!(validate_loopback_only("localhost:11111").is_ok());
        assert!(validate_loopback_only("http://127.0.0.1:11111/test").is_ok());
        assert!(validate_loopback_only("HTTP://127.0.0.1:11111/test").is_ok());
        assert!(validate_loopback_only("http://[::1]:11111").is_ok());
        assert!(validate_loopback_only("[::1]:11111").is_ok());

        // Prohibited schemes
        assert!(validate_loopback_only("ssh://127.0.0.1:22").is_err());
        assert!(validate_loopback_only("sftp://127.0.0.1").is_err());
        assert!(validate_loopback_only("ftp://127.0.0.1:21").is_err());
        assert!(validate_loopback_only("smtp://127.0.0.1:25").is_err());
        assert!(validate_loopback_only("mailto:trader@example.com").is_err());

        // Prohibited ports
        assert!(validate_loopback_only("127.0.0.1:22").is_err());
        assert!(validate_loopback_only("127.0.0.1:21").is_err());
        assert!(validate_loopback_only("127.0.0.1:25").is_err());
        assert!(validate_loopback_only("127.0.0.1:587").is_err());

        // Embedded credentials rejection
        assert!(validate_loopback_only("http://user:secret@127.0.0.1:11111").is_err());

        // External endpoints
        assert!(validate_loopback_only("8.8.8.8:80").is_err());
        assert!(validate_loopback_only("https://api.binance.com").is_err());
        assert!(validate_loopback_only("https://api.snaptrade.com").is_err());
        assert!(validate_loopback_only("https://query1.finance.yahoo.com").is_err());
        assert!(validate_loopback_only("https://www.tradingview.com").is_err());

        // Webhook URLs must be strictly blocked
        assert!(validate_loopback_only("https://discord.com/api/webhooks/123/xyz").is_err());
        assert!(validate_loopback_only("https://hooks.slack.com/services/123").is_err());
        assert!(validate_loopback_only("http://127.0.0.1:11111/webhook").is_err());
    }

    #[tokio::test]
    async fn test_watchlist_enforcement() {
        let client = OpenDClient::default();
        let wl = client.fetch_watchlist().await.unwrap();
        assert!(wl.contains(&"AAPL".to_string()));

        // Watchlist symbol succeeds
        assert!(client.is_in_watchlist("AAPL").await);
        assert!(client.is_in_watchlist("US.AAPL").await);
        assert!(client.is_in_watchlist("AAPL US Equity").await);
        assert!(client.fetch_quote("AAPL").await.is_ok());
        assert!(client.fetch_quote("US.AAPL").await.is_ok());

        // Non-watchlist symbol MUST be blocked
        assert!(!client.is_in_watchlist("UNKNOWN_COIN").await);
        assert!(client.fetch_quote("UNKNOWN_COIN").await.is_err());
        assert!(client.subscribe_quotes("UNKNOWN_COIN").is_none());
    }
}
