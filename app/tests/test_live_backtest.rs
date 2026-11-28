use std::path::{Path, PathBuf};
use tr_strategy::{aggregate_bars, run_backtest, run_interval_sweep, load_symbol_strategy};

fn get_repo_root() -> PathBuf {
    if Path::new("strategies").exists() {
        PathBuf::from(".")
    } else if Path::new("../strategies").exists() {
        PathBuf::from("..")
    } else if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(manifest);
        if p.join("strategies").exists() {
            p
        } else if p.parent().map(|parent| parent.join("strategies").exists()).unwrap_or(false) {
            p.parent().unwrap().to_path_buf()
        } else {
            PathBuf::from(".")
        }
    } else {
        PathBuf::from(".")
    }
}

fn load_symbol_bars(symbol: &str, limit: usize) -> Vec<tr_moomoo::Bar> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let p = home.join("Library/Application Support/terminal/bars.sqlite3");
    let sec = tr_data::security(symbol);
    if let Ok(c) = tr_data::cache::BarCache::open(&p) {
        let bars = c.load(&sec.ticker, tr_core::Timeframe::M1, limit)
            .or_else(|_| c.load(symbol, tr_core::Timeframe::M1, limit))
            .unwrap_or_default();
        if !bars.is_empty() {
            // Check if timestamps in DB were stored as raw Eastern Time epoch seconds (09:31:00 UTC = 04:31:00 ET).
            // If so, shift by +18,000s (+5 hours) so that 09:31:00 in ET becomes 14:31:00 UTC.
            let first_ts = bars[0].ts;
            let offset_sec = if let Some(dt) = chrono::DateTime::from_timestamp(first_ts, 0) {
                use chrono::Timelike;
                if dt.hour() == 9 && dt.minute() >= 30 {
                    18000 // Convert raw ET epoch to proper UTC epoch
                } else {
                    0
                }
            } else {
                0
            };

            return bars.into_iter().map(|b| tr_moomoo::Bar {
                ts: b.ts + offset_sec,
                open: b.open,
                high: b.high,
                low: b.low,
                close: b.close,
                volume: b.volume,
            }).collect();
        }
    }
    Vec::new()
}

fn filter_last_session(bars: &[tr_moomoo::Bar]) -> Vec<tr_moomoo::Bar> {
    bars.iter()
        .filter(|b| {
            if let Some(et) = tr_strategy::utc_to_us_eastern(b.ts) {
                et.format("%Y-%m-%d").to_string() == "2026-11-27"
            } else {
                false
            }
        })
        .cloned()
        .collect()
}

#[test]
fn test_live_session_and_full_history_backtest() {
    let root = get_repo_root();
    let symbols = vec![
        "US.MRNA",
        "US.ANT",
        "US.NVDA",
        "US.MU",
        "US.APP",
        "US.OMH",
        "US.SNDK",
    ];

    println!("\n===============================================================================================");
    println!(" OFFICIAL RUST BACKTEST ENGINE: NEXT-BAR OPEN EXECUTION (NO LOOKAHEAD BIAS)");
    println!(" Repository root: {}", root.display());
    println!(" Verifying DRBT-Grid Strategy on Real Data across all Watchlist Symbols");
    println!("===============================================================================================");

    println!("\n--- PART 1: LAST LIVE SESSION (2026-11-27) WITH SAVED paper.json CONFIGS ---");
    println!("{:<9} | {:<8} | {:<8} | {:<6} | {:<8} | {:<12} | {:<10} | {:<13} | {:<8}",
             "Symbol", "Interval", "Lookback", "Trades", "Win Rate", "Net PnL ($)", "Return %", "Profit Factor", "Max DD %");
    println!("{:-<9}-|-{:-<8}-|-{:-<8}-|-{:-<6}-|-{:-<8}-|-{:-<12}-|-{:-<10}-|-{:-<13}-|-{:-<8}",
             "", "", "", "", "", "", "", "", "");

    let mut session_total_pnl = 0.0;
    let mut total_session_trades = 0;

    for &sym in &symbols {
        let config = match load_symbol_strategy(&root, sym) {
            Ok(c) => c,
            Err(e) => {
                println!("Error loading config for {}: {:?}", sym, e);
                continue;
            }
        };

        let all_bars = load_symbol_bars(sym, 5000);
        let session_bars = filter_last_session(&all_bars);

        if session_bars.is_empty() {
            println!("{:<9} | {:<8} | {:<8} | {:<6} | {:<8} | {:<12} | {:<10} | {:<13} | {:<8}",
                     sym, format!("{}m", config.interval), config.params.grid_lookback, 0, "0.0%", "$0.00", "0.00%", "0.00", "0.00%");
            continue;
        }

        let agg = aggregate_bars(&session_bars, config.interval);
        let res = run_backtest(
            sym,
            config.interval,
            &agg,
            &config.params,
            100_000.0,
            0.95,
            2.0, // 2 bps slippage
            0.0, // 0 commission (paper)
        );

        let pnl = res.ending_equity - 100_000.0;
        session_total_pnl += pnl;
        total_session_trades += res.total_trades;

        println!("{:<9} | {:<8} | {:<8} | {:<6} | {:<7.1}% | ${:<11.2} | {:<9.2}% | {:<13.2} | {:<7.2}%",
                 sym,
                 format!("{}m", config.interval),
                 config.params.grid_lookback,
                 res.total_trades,
                 res.win_rate,
                 pnl,
                 res.total_return_pct,
                 res.profit_factor,
                 res.max_drawdown * 100.0,
        );

        for (idx, t) in res.trades.iter().enumerate() {
            println!("   └─ Trade #{}: Entry @ ${:.2} (t={}) -> Exit @ ${:.2} (t={}) | Net: ${:.2} ({:.2}%) | Reason: {}",
                     idx + 1, t.entry_price, t.entry_time, t.exit_price, t.exit_time, t.net_pnl, t.return_pct, t.reason);
        }
    }

    println!("-----------------------------------------------------------------------------------------------");
    println!(" Last Session Total Trades: {} | Total PnL across all symbols: ${:.2}", total_session_trades, session_total_pnl);

    println!("\n--- PART 2: MULTI-DAY WALK-FORWARD HISTORY (UP TO 5,000 1M BARS) WITH SAVED paper.json ---");
    println!("{:<9} | {:<8} | {:<8} | {:<8} | {:<6} | {:<8} | {:<12} | {:<10} | {:<13} | {:<8}",
             "Symbol", "Bars 1m", "Interval", "Lookback", "Trades", "Win Rate", "Net PnL ($)", "Return %", "Profit Factor", "Max DD %");
    println!("{:-<9}-|-{:-<8}-|-{:-<8}-|-{:-<8}-|-{:-<6}-|-{:-<8}-|-{:-<12}-|-{:-<10}-|-{:-<13}-|-{:-<8}",
             "", "", "", "", "", "", "", "", "", "");

    for &sym in &symbols {
        let config = match load_symbol_strategy(&root, sym) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let all_bars = load_symbol_bars(sym, 5000);
        if all_bars.is_empty() {
            continue;
        }

        let agg = aggregate_bars(&all_bars, config.interval);
        let res = run_backtest(
            sym,
            config.interval,
            &agg,
            &config.params,
            100_000.0,
            0.95,
            2.0,
            0.0,
        );

        let pnl = res.ending_equity - 100_000.0;
        println!("{:<9} | {:<8} | {:<8} | {:<8} | {:<6} | {:<7.1}% | ${:<11.2} | {:<9.2}% | {:<13.2} | {:<7.2}%",
                 sym,
                 all_bars.len(),
                 format!("{}m", config.interval),
                 config.params.grid_lookback,
                 res.total_trades,
                 res.win_rate,
                 pnl,
                 res.total_return_pct,
                 res.profit_factor,
                 res.max_drawdown * 100.0,
        );
    }

    println!("\n--- PART 3: MULTI-TIMEFRAME INTERVAL SWEEPS (1m to 10m) UNDER NEXT-BAR OPEN EXECUTION ---");
    println!(" Identifies the single best interval for each symbol when lookahead bias is eliminated:");
    for &sym in &symbols {
        let config = match load_symbol_strategy(&root, sym) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let all_bars = load_symbol_bars(sym, 3000);
        if all_bars.len() < 50 {
            continue;
        }

        let intervals: Vec<usize> = (1..=10).collect();
        let sweeps = run_interval_sweep(sym, &all_bars, &intervals, &config.params, 100_000.0);
        if let Some(best) = sweeps.first() {
            println!("  • {:<8}: Saved={}m | Best Swept={}m -> Return: {:>+6.2}% | Win Rate: {:>5.1}% | Trades: {:>3} | Profit Factor: {:>5.2} | Max DD: {:>5.2}%",
                     sym, config.interval, best.interval, best.total_return_pct, best.win_rate, best.total_trades, best.profit_factor, best.max_drawdown * 100.0);
        }
    }
    println!("===============================================================================================\n");
}
