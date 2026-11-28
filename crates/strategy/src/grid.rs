use tr_moomoo::Bar;
use chrono::{DateTime, Timelike};
use crate::model::{EntryMode, ExitMode, GridLevels, StrategyParams, StrategySignal};

/// Calculates rolling causal price grid levels from prior completed high and low bars.
pub fn calculate_grid_levels(
    lookback: usize,
    intervals: usize,
    highs: &[f64],
    lows: &[f64],
) -> Option<GridLevels> {
    let lookback = lookback.max(2);
    if highs.len() < lookback || lows.len() < lookback {
        return None;
    }

    let start = highs.len() - lookback;
    let mut prior_high = f64::NEG_INFINITY;
    let mut prior_low = f64::INFINITY;

    for &h in &highs[start..] {
        if h > prior_high {
            prior_high = h;
        }
    }
    for &l in &lows[start..] {
        if l < prior_low {
            prior_low = l;
        }
    }

    if !prior_high.is_finite() || !prior_low.is_finite() || prior_high <= prior_low {
        return None;
    }

    let span = prior_high - prior_low;
    let intervals = intervals.clamp(2, 20);

    let fractions: Vec<f64> = if intervals == 4 {
        vec![0.0, 0.25, 0.50, 1.0]
    } else if intervals < 4 {
        (0..intervals).map(|i| i as f64 / (intervals - 1) as f64).collect()
    } else {
        let mut set = vec![0.0, 0.25, 0.50, 1.0];
        for i in 1..intervals {
            let f = (i as f64 / intervals as f64 * 1_000_000.0).round() / 1_000_000.0;
            if !set.iter().any(|&x| (x - f).abs() < 1e-5) {
                set.push(f);
            }
        }
        set.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        set
    };

    let levels: Vec<f64> = fractions.iter().map(|&f| prior_low + span * f).collect();
    let bottom = levels[0];
    let top = *levels.last().unwrap_or(&prior_high);
    let lower_mid = prior_low + span * 0.25;
    let upper_mid = prior_low + span * 0.50;

    Some(GridLevels {
        bottom,
        lower_mid,
        upper_mid,
        top,
        levels,
    })
}

/// Checks if regular US trading hours entry is allowed given the configured open delay.
pub fn is_entry_allowed(ts: i64, open_delay_minutes: usize) -> bool {
    if let Some(dt) = DateTime::from_timestamp(ts, 0) {
        // Approximate US market hours in UTC (Standard: 14:30 - 21:00 UTC, EDT: 13:30 - 20:00 UTC)
        // Check local time hour and minute or timestamp of day
        let hour = dt.hour();
        let minute = dt.minute();
        // If between 9:30 and 10:30 EST (represented as 13:30 - 14:30 / 14:30 - 15:30 UTC)
        // Generic relative minutes from market open (assumed 9:30 AM EST)
        let earliest_m = 30 + open_delay_minutes;
        let entry_hour = 9 + earliest_m / 60;
        let entry_min = earliest_m % 60;

        let market_min_of_day = hour * 60 + minute;
        // In US regular session, market opens at 9:30 AM EST.
        // We ensure that if within the first hour of regular trading, delay has elapsed.
        if (market_min_of_day >= 9 * 60 + 30) && (market_min_of_day < 10 * 60 + 30) {
            market_min_of_day >= (entry_hour * 60 + entry_min) as u32
        } else {
            true
        }
    } else {
        true
    }
}

/// Checks if session exit is due (30 minutes before regular close: 15:30 EST).
pub fn is_session_exit_due(ts: i64) -> bool {
    if let Some(dt) = DateTime::from_timestamp(ts, 0) {
        let hour = dt.hour();
        let minute = dt.minute();
        // 15:30 EST to 16:00 EST
        hour == 15 && minute >= 30
    } else {
        false
    }
}

/// Evaluates grid signals over a sequence of candlestick bars.
pub fn evaluate_signals(candles: &[Bar], params: &StrategyParams) -> Vec<StrategySignal> {
    let mut signals = Vec::with_capacity(candles.len());
    let lookback = params.grid_lookback.max(2);

    if candles.len() <= lookback {
        return signals;
    }

    let mut highs: Vec<f64> = Vec::with_capacity(candles.len());
    let mut lows: Vec<f64> = Vec::with_capacity(candles.len());

    for bar in candles {
        highs.push(bar.high);
        lows.push(bar.low);
    }

    for i in (lookback + 1)..candles.len() {
        let prev_highs = &highs[..i - 1];
        let prev_lows = &lows[..i - 1];

        let grid = match calculate_grid_levels(lookback, params.grid_intervals, prev_highs, prev_lows) {
            Some(g) => g,
            None => continue,
        };

        let cur_bar = &candles[i];
        let prev_bar = &candles[i - 1];

        let cur_open = cur_bar.open;
        let prev_open = prev_bar.open;
        let cur_close = cur_bar.close;

        // Check crossings across grid levels
        let mut open_cross_up = false;
        let mut open_cross_down = false;

        for &level in &grid.levels {
            if cur_open > level && prev_open <= level {
                open_cross_up = true;
            }
            if cur_open < level && prev_open >= level {
                open_cross_down = true;
            }
        }

        let top_breakout = cur_open > grid.top && prev_open <= grid.top;
        let top_breakdown = cur_open < grid.top && prev_open >= grid.top;
        let bottom_breakdown = cur_open < grid.bottom && prev_open >= grid.bottom;

        let upper_half_cross = top_breakout || (cur_open > grid.upper_mid && prev_open <= grid.upper_mid);
        let lower_half_cross = (cur_open > grid.lower_mid && prev_open <= grid.lower_mid)
            || (cur_open > grid.bottom && prev_open <= grid.bottom);
        let lower_half_breakdown = (cur_open < grid.lower_mid && prev_open >= grid.lower_mid)
            || bottom_breakdown;

        let buy_trigger = match params.entry_mode {
            EntryMode::AnyGridCross => open_cross_up,
            EntryMode::TopBreakout => top_breakout,
            EntryMode::UpperHalfCross => upper_half_cross,
            EntryMode::LowerHalfCross => lower_half_cross,
        };

        let close_trigger = match params.exit_mode {
            ExitMode::AnyGridCrossDown => open_cross_down,
            ExitMode::TopBreakdown => top_breakdown,
            ExitMode::BottomBreakdown => bottom_breakdown,
            ExitMode::LowerHalfBreakdown => lower_half_breakdown,
        };

        // Long only: bullish candle required (close >= open)
        let bullish = cur_close >= cur_open;
        let buy_signal = buy_trigger && bullish;
        let close_signal = close_trigger;

        let entry_allowed = is_entry_allowed(cur_bar.ts, params.open_delay_minutes);
        let session_exit = is_session_exit_due(cur_bar.ts);

        signals.push(StrategySignal {
            bar_time: cur_bar.ts,
            open: cur_bar.open,
            high: cur_bar.high,
            low: cur_bar.low,
            close: cur_bar.close,
            grid,
            buy_signal,
            close_signal,
            long_signal: buy_signal && entry_allowed && !session_exit,
            exit_signal: close_signal || session_exit,
            entry_allowed,
            session_exit,
            entry_mode: format!("{:?}", params.entry_mode),
            exit_mode: format!("{:?}", params.exit_mode),
        });
    }

    signals
}

/// Evaluates the strategy signal for the latest completed candle.
pub fn strategy_signal_latest(candles: &[Bar], params: &StrategyParams) -> anyhow::Result<StrategySignal> {
    let signals = evaluate_signals(candles, params);
    signals.into_iter().last().ok_or_else(|| anyhow::anyhow!("insufficient candles for grid strategy"))
}
