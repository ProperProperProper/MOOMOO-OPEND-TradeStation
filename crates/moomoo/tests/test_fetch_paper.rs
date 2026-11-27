use tr_moomoo::OpenDClient;

#[tokio::test]
async fn test_live_paper_sync() {
    let client = OpenDClient::default();
    if let Ok((funds, positions, orders)) = client.fetch_paper_trading().await {
        println!("SUCCESS: funds = {:?}", funds);
        println!("positions = {:?}", positions);
        println!("orders = {:?}", orders);
        assert!(funds.cash >= 0.0);
    }
}
