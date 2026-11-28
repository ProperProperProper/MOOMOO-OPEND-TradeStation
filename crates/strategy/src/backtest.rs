use tr_moomoo::Bar;
use crate::exits::RiskTracker;
use crate::grid::evaluate_signals;
use crate::model::{BacktestResult, StrategyMarker, StrategyParams, Trade};

/// Converts simulated trades into visual chart markers for entry and exit points.
pub fn trades_to_markers(trades: &[Trade]) -> Vec<StrategyMarker> {
    let mut markers = Vec::new();
    for t in trades {
        let entry_ts = t.entry_time.parse::<i64>().unwrap_or(0);
        let exit_ts = t.exit_time.parse::<i64>().unwrap_or(0);
        if entry_ts > 0 {
            markers.push(StrategyMarker {
                ts: entry_ts,
                price: t.entry_price,
                is_buy: true,
                label: format!("BUY @ ${:.2}", t.entry_price),
                pnl_pct: None,
            });
        }
        if exit_ts > 0 {
            markers.push(StrategyMarker {
                ts: exit_ts,
                price: t.exit_price,
                is_buy: false,
                label: format!("{} @ ${:.2}", t.reason, t.exit_price),
                pnl_pct: Some(t.return_pct),
            });
        }
    }
    markers.sort_by_key(|m| m.ts);
    markers
}

/// Replays a series of completed candles through the Grid Open-Cross strategy with causal risk exits.
#[allow(clippy::too_many_arguments)]
pub fn run_backtest(
    symbol: &str,
    interval: usize,
    candles: &[Bar],
    params: &StrategyParams,
    capital: f64,
    allocation: f64,
    slippage_bps: f64,
    commission: f64,
) -> BacktestResult {
    let signals = evaluate_signals(candles, params);
    let mut cash = capital;
    let mut position_qty = 0.0;
    let mut entry_price = 0.0;
    let mut entry_time = String::new();
    let mut risk_tracker: Option<RiskTracker> = None;
    let mut trades = Vec::new();
    let slip = slippage_bps / 10000.0;
    let mut peak_equity = capital;
    let mut max_drawdown = 0.0;
    let mut pending_buy = false;

    for sig in &signals {
        let price = sig.open;

        // 1. If currently in position, evaluate exits
        if position_qty > 0.0 {
            let mut exit_reason = None;

            // Check intrabar risk exits (using high/low of bar)
            if let Some(tracker) = risk_tracker.as_mut() {
                if let Some(r) = tracker.evaluate_intrabar_exit(sig.high) {
                    exit_reason = Some(r);
                } else if let Some(r) = tracker.evaluate_intrabar_exit(sig.low) {
                    exit_reason = Some(r);
                }
            }

            // Check session exit
            if exit_reason.is_none() && sig.session_exit {
                exit_reason = Some("SESSION_EXIT_30M_BEFORE_CLOSE");
            }

            // Check strategy close signal (open crossing grid down)
            if exit_reason.is_none() && sig.close_signal {
                exit_reason = Some("OPEN_CROSSED_GRID_DOWN");
            }

            if let Some(reason) = exit_reason {
                let exit_p = price * (1.0 - slip);
                let gross = (exit_p - entry_price) * position_qty;
                let costs = commission * 2.0;
                let net = gross - costs;
                cash += position_qty * exit_p - costs;
                let notional = entry_price * position_qty;
                let ret_pct = if notional > 0.0 { (net / notional) * 100.0 } else { 0.0 };

                trades.push(Trade {
                    entry_time: entry_time.clone(),
                    exit_time: sig.bar_time.to_string(),
                    entry_price,
                    exit_price: exit_p,
                    qty: position_qty,
                    gross_pnl: gross,
                    costs,
                    net_pnl: net,
                    return_pct: ret_pct,
                    reason: reason.to_string(),
                });

                position_qty = 0.0;
                entry_price = 0.0;
                risk_tracker = None;
            }
        }

        // 2. If flat and an entry signal confirmed on the previous bar, execute at this bar's open
        if position_qty == 0.0 && pending_buy {
            let fill_p = price * (1.0 + slip);
            let spend = cash * allocation;
            let qty = (spend / fill_p).floor();
            if qty > 0.0 {
                cash -= qty * fill_p + commission;
                position_qty = qty;
                entry_price = fill_p;
                entry_time = sig.bar_time.to_string();
                risk_tracker = Some(RiskTracker::new(
                    symbol.to_string(),
                    fill_p,
                    qty,
                    params.clone(),
                ));
            }
            pending_buy = false;
        }

        // 3. If flat, evaluate entry signal confirming on candle close -> queue for open of next bar
        if position_qty == 0.0 && sig.long_signal {
            pending_buy = true;
        }

        // Track running equity and drawdown
        let current_equity = cash + position_qty * sig.close;
        if current_equity > peak_equity {
            peak_equity = current_equity;
        }
        let dd = if peak_equity > 0.0 {
            (peak_equity - current_equity) / peak_equity
        } else {
            0.0
        };
        if dd > max_drawdown {
            max_drawdown = dd;
        }
    }

    // Close any trailing position at final bar close
    if position_qty > 0.0
        && let Some(last_sig) = signals.last() {
            let exit_p = last_sig.close * (1.0 - slip);
            let gross = (exit_p - entry_price) * position_qty;
            let costs = commission * 2.0;
            let net = gross - costs;
            cash += position_qty * exit_p - costs;
            let notional = entry_price * position_qty;
            let ret_pct = if notional > 0.0 { (net / notional) * 100.0 } else { 0.0 };

            trades.push(Trade {
                entry_time,
                exit_time: last_sig.bar_time.to_string(),
                entry_price,
                exit_price: exit_p,
                qty: position_qty,
                gross_pnl: gross,
                costs,
                net_pnl: net,
                return_pct: ret_pct,
                reason: "END_OF_DATA".to_string(),
            });
        }

    let ending_equity = cash;
    let total_return_pct = ((ending_equity - capital) / capital) * 100.0;

    let total_trades = trades.len();
    let wins = trades.iter().filter(|t| t.net_pnl > 0.0).count();
    let win_rate = if total_trades > 0 {
        (wins as f64 / total_trades as f64) * 100.0
    } else {
        0.0
    };

    let gross_profit: f64 = trades.iter().filter(|t| t.net_pnl > 0.0).map(|t| t.net_pnl).sum();
    let gross_loss: f64 = trades.iter().filter(|t| t.net_pnl < 0.0).map(|t| t.net_pnl.abs()).sum();
    let profit_factor = if gross_loss > 0.0 {
        gross_profit / gross_loss
    } else if gross_profit > 0.0 {
        99.9
    } else {
        0.0
    };

    // Sharpe approximation
    let returns: Vec<f64> = trades.iter().map(|t| t.return_pct).collect();
    let mean_ret = if !returns.is_empty() {
        returns.iter().sum::<f64>() / returns.len() as f64
    } else {
        0.0
    };
    let var = if returns.len() > 1 {
        returns.iter().map(|r| (r - mean_ret).powi(2)).sum::<f64>() / (returns.len() - 1) as f64
    } else {
        0.0
    };
    let std_dev = var.sqrt();
    let sharpe = if std_dev > 0.0 {
        (mean_ret / std_dev) * (252.0_f64).sqrt()
    } else {
        0.0
    };

    BacktestResult {
        symbol: symbol.to_string(),
        interval,
        params: params.clone(),
        trades,
        total_return_pct,
        win_rate,
        profit_factor,
        max_drawdown: max_drawdown * 100.0,
        total_trades,
        sharpe,
        ending_equity,
    }
}
