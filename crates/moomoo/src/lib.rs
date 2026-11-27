use anyhow::Result;
use serde::{Deserialize, Serialize};
use reqwest::Client;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub timestamp: i64,
}

/// Fetch the watchlist (list of symbols) for the authenticated user.
/// `token` must be a valid OpenD API token.
pub async fn fetch_watchlist(token: &str) -> Result<Vec<String>> {
    let client = Client::new();
    let resp = client
        .get("https://open.d/api/v1/watchlist")
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?;
    let symbols: Vec<String> = resp.json().await?;
    Ok(symbols)
}

/// Fetch a single quote for `symbol`.
pub async fn fetch_quote(symbol: &str, token: &str) -> Result<Quote> {
    let client = Client::new();
    let url = format!("https://open.d/api/v1/quote?symbol={}", symbol);
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?;
    let quote: Quote = resp.json().await?;
    Ok(quote)
}
