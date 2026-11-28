use rand::Rng;
use tr_moomoo::Bar;
use crate::backtest::run_backtest;
use crate::model::{BacktestResult, EntryMode, ExitMode, PromotionGate, StrategyParams};

/// Aggregates 1-minute bars into higher minute interval bars (e.g. 2m..10m).
pub fn aggregate_bars(bars_1m: &[Bar], interval_minutes: usize) -> Vec<Bar> {
    if interval_minutes <= 1 || bars_1m.is_empty() {
        return bars_1m.to_vec();
    }

    let interval_sec = (interval_minutes as i64) * 60;
    let mut aggregated = Vec::new();

    let mut current_bucket: Option<i64> = None;
    let mut o = 0.0;
    let mut h = f64::NEG_INFINITY;
    let mut l = f64::INFINITY;
    let mut c = 0.0;
    let mut v = 0.0;

    for bar in bars_1m {
        let bucket = bar.ts - (bar.ts % interval_sec);

        match current_bucket {
            Some(b) if b == bucket => {
                h = h.max(bar.high);
                l = l.min(bar.low);
                c = bar.close;
                v += bar.volume;
            }
            Some(b) => {
                aggregated.push(Bar {
                    ts: b,
                    open: o,
                    high: h,
                    low: l,
                    close: c,
                    volume: v,
                });
                current_bucket = Some(bucket);
                o = bar.open;
                h = bar.high;
                l = bar.low;
                c = bar.close;
                v = bar.volume;
            }
            None => {
                current_bucket = Some(bucket);
                o = bar.open;
                h = bar.high;
                l = bar.low;
                c = bar.close;
                v = bar.volume;
            }
        }
    }

    if let Some(b) = current_bucket {
        aggregated.push(Bar {
            ts: b,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
        });
    }

    aggregated
}

/// Runs a research interval sweep across 1m..10m intervals using aggregated bars.
/// Distributes execution across multiple Apple Silicon E-cores using scoped parallel workers
/// with Darwin background QoS to prevent single-core hotspots, thermal throttling, and fan spin-up.
pub fn run_interval_sweep(
    symbol: &str,
    bars_1m: &[Bar],
    intervals: &[usize],
    params: &StrategyParams,
    capital: f64,
) -> Vec<BacktestResult> {
    if intervals.is_empty() {
        return Vec::new();
    }

    let topo = crate::governor::detect_cpu_topology();
    // Distribute across multiple E-cores (e.g. 2 to 6 cores on Apple Silicon M4)
    let num_workers = topo.e_cores.clamp(2, intervals.len());
    let (thermal, _) = crate::governor::check_thermal_status();

    // Partition intervals across worker threads
    let chunks: Vec<Vec<usize>> = {
        let mut groups = vec![Vec::new(); num_workers];
        for (i, &intv) in intervals.iter().enumerate() {
            groups[i % num_workers].push(intv);
        }
        groups.into_iter().filter(|g| !g.is_empty()).collect()
    };

    let results = std::thread::scope(|s| {
        let mut handles = Vec::new();
        for group in chunks {
            handles.push(s.spawn(move || {
                // Direct each worker thread to Apple Silicon E-cores under Darwin background QoS
                crate::governor::apply_cooling_qos();

                let mut partial = Vec::new();
                for &interval in &group {
                    let aggregated = aggregate_bars(bars_1m, interval);
                    let res = run_backtest(
                        symbol,
                        interval,
                        &aggregated,
                        params,
                        capital,
                        0.95,
                        2.0,
                        0.0,
                    );
                    partial.push(res);
                    crate::governor::pace_for_cooling(thermal);
                }
                partial
            }));
        }

        let mut all = Vec::new();
        for handle in handles {
            if let Ok(batch) = handle.join() {
                all.extend(batch);
            }
        }
        all
    });

    let mut sorted = results;
    // Sort descending by total return %
    sorted.sort_by(|a, b| b.total_return_pct.partial_cmp(&a.total_return_pct).unwrap_or(std::cmp::Ordering::Equal));
    sorted
}

/// Generates randomized candidate parameter sets bounded within verified strategy constraints.
pub fn generate_candidates(count: usize, base: &StrategyParams) -> Vec<StrategyParams> {
    let mut rng = rand::rng();
    let mut candidates = Vec::with_capacity(count);

    let entry_modes = [
        EntryMode::AnyGridCross,
        EntryMode::TopBreakout,
        EntryMode::UpperHalfCross,
        EntryMode::LowerHalfCross,
    ];

    let exit_modes = [
        ExitMode::AnyGridCrossDown,
        ExitMode::TopBreakdown,
        ExitMode::BottomBreakdown,
        ExitMode::LowerHalfBreakdown,
    ];

    let trailing_options = [0.0, 0.005, 0.01, 0.015, 0.02, 0.03, 0.04];
    let take_profit_options = [0.0056, 0.007, 0.0084, 0.0105, 0.015, 0.01918, 0.025, 0.03];
    let open_delay_options = [0, 2, 5, 8, 10, 12, 15, 20];

    for _ in 0..count {
        let lookback_offset: i32 = rng.random_range(-25..25);
        let lookback = (base.grid_lookback as i32 + lookback_offset).clamp(5, 120) as usize;
        let intervals = rng.random_range(2..=20);
        let entry_mode = entry_modes[rng.random_range(0..entry_modes.len())];
        let exit_mode = exit_modes[rng.random_range(0..exit_modes.len())];
        let trailing_act = trailing_options[rng.random_range(0..trailing_options.len())];
        let trailing_stop = if trailing_act > 0.0 {
            trailing_options[rng.random_range(1..trailing_options.len())]
        } else {
            0.0
        };
        let take_profit = take_profit_options[rng.random_range(0..take_profit_options.len())];
        let open_delay = open_delay_options[rng.random_range(0..open_delay_options.len())];

        candidates.push(StrategyParams {
            grid_lookback: lookback,
            grid_intervals: intervals,
            entry_mode,
            exit_mode,
            open_delay_minutes: open_delay,
            stop_loss_pct: base.stop_loss_pct,
            take_profit_pct: take_profit,
            trailing_activation_pct: trailing_act,
            trailing_stop_pct: trailing_stop,
        });
    }

    candidates
}

/// Evaluates if a backtest result qualifies for paper strategy promotion according to evidence gates.
pub fn evaluate_promotion_gate(
    candidate: &BacktestResult,
    baseline: &BacktestResult,
    completed_sessions: usize,
) -> PromotionGate {
    if completed_sessions < 2 {
        return PromotionGate {
            allowed: false,
            reason: format!("insufficient completed sessions ({completed_sessions} < 2)"),
            beats_baseline: false,
        };
    }

    if candidate.total_trades < 2 {
        return PromotionGate {
            allowed: false,
            reason: format!("insufficient trades ({} < 2)", candidate.total_trades),
            beats_baseline: false,
        };
    }

    if candidate.total_return_pct <= 0.0 {
        return PromotionGate {
            allowed: false,
            reason: format!("net return is non-positive ({:.2}%)", candidate.total_return_pct),
            beats_baseline: false,
        };
    }

    let beats = candidate.total_return_pct > baseline.total_return_pct;
    if !beats {
        return PromotionGate {
            allowed: false,
            reason: format!(
                "candidate return ({:.2}%) does not beat active baseline ({:.2}%)",
                candidate.total_return_pct, baseline.total_return_pct
            ),
            beats_baseline: false,
        };
    }

    PromotionGate {
        allowed: true,
        reason: format!(
            "meets all evidence gates: {} sessions, {} trades, +{:.2}% return (beats baseline {:.2}%)",
            completed_sessions, candidate.total_trades, candidate.total_return_pct, baseline.total_return_pct
        ),
        beats_baseline: true,
    }
}

/// Generates AI proposals, querying local AI gateway if online or using bounded intelligent candidate generator.
pub async fn fetch_ai_proposals(
    symbol: &str,
    interval: usize,
    gateway_url: &str,
    base: &StrategyParams,
) -> Vec<StrategyParams> {
    // Attempt local AI Gateway query
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .unwrap_or_default();

    let prompt = format!(
        "Return bounded quantitative parameter suggestions for {} {}m candle open grid strategy: grid_lookback 5..120, grid_intervals 2..20, trailing_activation_pct 0.0..0.04, trailing_stop_pct 0.0..0.04",
        symbol, interval
    );

    let body = serde_json::json!({
        "prompt": prompt,
        "max_tokens": 128,
        "qos": "background"
    });

    if let Ok(resp) = client.post(gateway_url).json(&body).send().await
        && resp.status().is_success() {
            // Local AI gateway answered
            if let Ok(val) = resp.json::<serde_json::Value>().await
                && let Some(text) = val.get("response").and_then(|r| r.as_str())
                    && let Ok(parsed) = serde_json::from_str::<Vec<StrategyParams>>(text)
                        && !parsed.is_empty() {
                            return parsed;
                        }
        }

    // Bounded stochastic proposals aligned with local research models
    generate_candidates(4, base)
}
