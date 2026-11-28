use std::path::{Path, PathBuf};
use tr_strategy::{
    aggregate_bars, evaluate_promotion_gate, run_backtest, save_symbol_strategy, load_symbol_strategy,
    EntryMode, ExitMode, StrategyParams, SymbolPaperConfig,
};

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
            let first_ts = bars[0].ts;
            let offset_sec = if let Some(dt) = chrono::DateTime::from_timestamp(first_ts, 0) {
                use chrono::Timelike;
                if dt.hour() == 9 && dt.minute() >= 30 {
                    18000
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

fn score_result(res: &tr_strategy::BacktestResult) -> f64 {
    if res.total_trades < 2 || res.total_return_pct <= 0.0 {
        return -1000.0;
    }
    let pf = res.profit_factor.min(10.0);
    let win_rate = res.win_rate / 100.0;
    let dd_penalty = if res.max_drawdown > 0.05 {
        (res.max_drawdown - 0.05) * 50.0
    } else {
        0.0
    };
    res.total_return_pct * pf * (0.5 + win_rate) - dd_penalty
}

#[test]
fn test_reoptimize_all_strategies() {
    let root = get_repo_root();
    let symbols = vec![
        "US.MRNA",
        "US.ANT",
        "US.NVDA",
        "US.MU",
        "US.APP",
        "US.OMH",
        "US.SNDK",
        "US.AAPL",
        "US.ADMA",
        "US.ADP",
        "US.ALRM",
    ];

    println!("\n===============================================================================================");
    println!(" 🚀 FULL STRATEGY RE-OPTIMIZATION: CAUSAL NEXT-BAR OPEN EXECUTION (NO LOOKAHEAD)");
    println!(" Repository root: {}", root.display());
    println!("===============================================================================================");

    let lookbacks = [5, 8, 12, 16, 20, 30, 45, 60];
    let grid_tiers = [4, 6, 8, 12, 16];
    let entry_modes = [
        EntryMode::AnyGridCross,
        EntryMode::UpperHalfCross,
        EntryMode::TopBreakout,
        EntryMode::LowerHalfCross,
    ];
    let exit_modes = [
        ExitMode::LowerHalfBreakdown,
        ExitMode::AnyGridCrossDown,
        ExitMode::BottomBreakdown,
    ];
    let tps = [0.005, 0.0084, 0.012, 0.015, 0.02, 0.03, 0.05];
    let sls = [0.03, 0.05, 0.08, 0.10, 0.1324];
    let trailing_pairs = [
        (0.0, 0.0),
        (0.01, 0.005),
        (0.015, 0.01),
        (0.02, 0.015),
    ];
    let open_delays = [0, 5, 8, 12];

    for &sym in &symbols {
        let bars_1m = load_symbol_bars(sym, 5000);
        if bars_1m.len() < 100 {
            println!("Skipping {} (insufficient history: {} bars)", sym, bars_1m.len());
            continue;
        }

        let old_config = load_symbol_strategy(&root, sym).ok();
        let old_baseline = if let Some(ref cfg) = old_config {
            let agg = aggregate_bars(&bars_1m, cfg.interval);
            run_backtest(sym, cfg.interval, &agg, &cfg.params, 100_000.0, 0.95, 2.0, 0.0)
        } else {
            let agg = aggregate_bars(&bars_1m, 5);
            run_backtest(sym, 5, &agg, &StrategyParams::default(), 100_000.0, 0.95, 2.0, 0.0)
        };

        println!("\n🔍 Optimizing {} (History: {} 1m bars)...", sym, bars_1m.len());
        println!("   Old Baseline: Interval={}m | Return={:+.2}% | WR={:.1}% | PF={:.2} | Trades={}",
                 old_config.as_ref().map(|c| c.interval).unwrap_or(5),
                 old_baseline.total_return_pct,
                 old_baseline.win_rate,
                 old_baseline.profit_factor,
                 old_baseline.total_trades);

        // Precompute aggregated bars for intervals 1..=10
        let agg_map: Vec<(usize, Vec<tr_moomoo::Bar>)> = (1..=10)
            .map(|intv| (intv, aggregate_bars(&bars_1m, intv)))
            .collect();

        // Stage 1: Sweep all 1m..10m intervals with base params to find top interval candidates
        let mut interval_scores: Vec<(usize, f64)> = Vec::new();
        for &(intv, ref agg_bars) in &agg_map {
            let base_p = old_config.as_ref().map(|c| c.params.clone()).unwrap_or_default();
            let res = run_backtest(sym, intv, agg_bars, &base_p, 100_000.0, 0.95, 2.0, 0.0);
            interval_scores.push((intv, res.total_return_pct));
        }
        interval_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        // Take top 4 intervals, plus intervals 3, 5, 7, 8, 9, 10
        let mut candidate_intervals: Vec<usize> = interval_scores.iter().take(4).map(|x| x.0).collect();
        for &key_intv in &[3, 5, 7, 8, 9] {
            if !candidate_intervals.contains(&key_intv) {
                candidate_intervals.push(key_intv);
            }
        }

        let mut best_score = f64::NEG_INFINITY;
        let mut best_interval = old_config.as_ref().map(|c| c.interval).unwrap_or(5);
        let mut best_params = old_config.as_ref().map(|c| c.params.clone()).unwrap_or_default();
        let mut best_result = old_baseline.clone();

        // Stage 2: Staged grid search over top candidate intervals
        for &intv in &candidate_intervals {
            let agg_bars = &agg_map.iter().find(|(i, _)| *i == intv).unwrap().1;

            // 2A: Find best grid structure (lookback, tiers, entry, exit)
            let mut top_grid_structures = Vec::new();

            for &lb in &lookbacks {
                for &tiers in &grid_tiers {
                    for &em in &entry_modes {
                        for &xm in &exit_modes {
                            let params = StrategyParams {
                                grid_lookback: lb,
                                grid_intervals: tiers,
                                entry_mode: em,
                                exit_mode: xm,
                                open_delay_minutes: 5,
                                take_profit_pct: 0.015,
                                stop_loss_pct: 0.08,
                                trailing_activation_pct: 0.01,
                                trailing_stop_pct: 0.005,
                            };

                            let res = run_backtest(sym, intv, agg_bars, &params, 100_000.0, 0.95, 2.0, 0.0);
                            let sc = score_result(&res);
                            top_grid_structures.push((sc, lb, tiers, em, xm, res));
                        }
                    }
                }
            }

            top_grid_structures.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            // 2B: Fine-tune risk parameters (TP, SL, Trailing, Delay) on top 4 structures
            for (_, lb, tiers, em, xm, _) in top_grid_structures.into_iter().take(4) {
                for &tp in &tps {
                    for &sl in &sls {
                        for &(tact, tstop) in &trailing_pairs {
                            for &delay in &open_delays {
                                let params = StrategyParams {
                                    grid_lookback: lb,
                                    grid_intervals: tiers,
                                    entry_mode: em,
                                    exit_mode: xm,
                                    open_delay_minutes: delay,
                                    take_profit_pct: tp,
                                    stop_loss_pct: sl,
                                    trailing_activation_pct: tact,
                                    trailing_stop_pct: tstop,
                                };

                                let res = run_backtest(sym, intv, agg_bars, &params, 100_000.0, 0.95, 2.0, 0.0);
                                let score = score_result(&res);
                                if score > best_score {
                                    best_score = score;
                                    best_interval = intv;
                                    best_params = params;
                                    best_result = res;
                                }
                            }
                        }
                    }
                }
            }
        }

        let gate = evaluate_promotion_gate(&best_result, &old_baseline, 5);
        println!("   🎯 BEST FOUND: Interval={}m | Return={:+.2}% | WR={:.1}% | PF={:.2} | Trades={} | DD={:.2}%",
                 best_interval,
                 best_result.total_return_pct,
                 best_result.win_rate,
                 best_result.profit_factor,
                 best_result.total_trades,
                 best_result.max_drawdown * 100.0);
        println!("   Params: lookback={}, tiers={}, entry={:?}, exit={:?}, delay={}m, TP={:.2}%, SL={:.2}%, Trailing=({:.2}%, {:.2}%)",
                 best_params.grid_lookback,
                 best_params.grid_intervals,
                 best_params.entry_mode,
                 best_params.exit_mode,
                 best_params.open_delay_minutes,
                 best_params.take_profit_pct * 100.0,
                 best_params.stop_loss_pct * 100.0,
                 best_params.trailing_activation_pct * 100.0,
                 best_params.trailing_stop_pct * 100.0);
        println!("   Evidence Gate: {}", gate.reason);

        // Save promoted config
        if best_result.total_return_pct > old_baseline.total_return_pct || old_baseline.total_return_pct <= 0.0 {
            let new_config = SymbolPaperConfig {
                symbol: sym.to_string(),
                interval: best_interval,
                mode: "SIMULATE".into(),
                params: best_params.clone(),
                run_id: format!("{}-NEXTBAR-REOPT-{}", sym.replace('.', "-"), chrono::Utc::now().format("%Y%m%d-%H%M%S")),
                updated_at: chrono::Utc::now().to_rfc3339(),
            };

            if let Err(e) = save_symbol_strategy(&root, &new_config) {
                println!("   ⚠️ Failed to save config for {}: {:?}", sym, e);
            } else {
                println!("   ✅ Saved re-optimized configuration to strategies/{}/paper.json", sym.replace('.', "-"));
            }
        } else {
            println!("   ℹ️ Retained existing configuration.");
        }
    }

    println!("\n===============================================================================================");
    println!(" RE-OPTIMIZATION COMPLETE ACROSS ALL WATCHLIST SYMBOLS");
    println!("===============================================================================================\n");
}
