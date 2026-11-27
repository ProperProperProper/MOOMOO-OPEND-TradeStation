//! Tests for Moomoo OpenD market data provider.
use tr_core::Timeframe;
use tr_data::MarketData;
use tr_data::markets::Markets;

#[tokio::test]
async fn moomoo_markets_creation_and_naming() {
    let m = Markets::new().unwrap();
    assert!(m.name().starts_with("moomoo-opend"));
}

#[tokio::test]
async fn moomoo_markets_watchlist_restriction() {
    let m = Markets::new().unwrap();
    // Non-watchlist ticker must be rejected by security policy
    let res = m.history("NONEXISTENT_TICKER_999 US Equity", Timeframe::D1, None, 100).await;
    assert!(res.is_err(), "Non-watchlist ticker should be blocked");
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("SECURITY RESTRICTION"), "Error should explain security restriction: {err_msg}");
}
