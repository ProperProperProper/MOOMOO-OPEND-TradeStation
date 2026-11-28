use tr_moomoo::Bar;
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Timelike, Utc};
use crate::model::{EntryMode, ExitMode, GridLevels, MarketSessionPhase, StrategyParams, StrategySignal};

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

/// Translates any UTC timestamp to US Eastern Time (ET), accurately switching between
/// Eastern Daylight Time (EDT: UTC-4) and Eastern Standard Time (EST: UTC-5).
pub fn utc_to_us_eastern(ts: i64) -> Option<DateTime<FixedOffset>> {
    let utc_dt = DateTime::from_timestamp(ts, 0)?;
    let year = utc_dt.year();

    // In the US, DST begins second Sunday in March at 02:00 local (07:00 UTC)
    // and ends first Sunday in November at 02:00 local (06:00 UTC).
    let march_first = NaiveDate::from_ymd_opt(year, 3, 1)?.and_hms_opt(0, 0, 0)?;
    let march_first_weekday = march_first.weekday().num_days_from_sunday();
    let second_sunday_day = if march_first_weekday == 0 { 8 } else { 1 + (7 - march_first_weekday) + 7 };
    let dst_start_naive = NaiveDate::from_ymd_opt(year, 3, second_sunday_day)?.and_hms_opt(7, 0, 0)?;
    let dst_start = DateTime::<Utc>::from_naive_utc_and_offset(dst_start_naive, Utc).timestamp();

    let nov_first = NaiveDate::from_ymd_opt(year, 11, 1)?.and_hms_opt(0, 0, 0)?;
    let nov_first_weekday = nov_first.weekday().num_days_from_sunday();
    let first_sunday_day = if nov_first_weekday == 0 { 1 } else { 1 + (7 - nov_first_weekday) };
    let dst_end_naive = NaiveDate::from_ymd_opt(year, 11, first_sunday_day)?.and_hms_opt(6, 0, 0)?;
    let dst_end = DateTime::<Utc>::from_naive_utc_and_offset(dst_end_naive, Utc).timestamp();

    let offset_seconds = if ts >= dst_start && ts < dst_end {
        -4 * 3600 // EDT (UTC-4)
    } else {
        -5 * 3600 // EST (UTC-5)
    };

    let offset = FixedOffset::east_opt(offset_seconds)?;
    Some(utc_dt.with_timezone(&offset))
}

/// Evaluates the market session lifecycle phase for a given timestamp:
/// - IdleOffSession: 17:00 ET through 08:30 ET (and weekends) -> Completely idle, doing nothing.
/// - WakingPreSession: 08:30 ET to 09:30 ET (wakes 1 hour before session) -> Warm-up, prep quotes/levels, no entries.
/// - OpenSessionActive: 09:30 ET to 16:00 ET -> Open session trading (with open delay if configured).
/// - PostSessionWindDown: 16:00 ET to 17:00 ET (up to 1 hour after close) -> Position exit & settlement; idle at 17:00.
pub fn get_market_session_phase(ts: i64) -> MarketSessionPhase {
    let Some(et) = utc_to_us_eastern(ts) else {
        return MarketSessionPhase::IdleOffSession;
    };

    // Weekends (Sunday=0, Saturday=6) are completely idle
    let weekday = et.weekday().num_days_from_sunday();
    if weekday == 0 || weekday == 6 {
        return MarketSessionPhase::IdleOffSession;
    }

    let min_of_day = et.hour() * 60 + et.minute();
    // 08:30 ET = 510 min
    // 09:30 ET = 570 min
    // 16:00 ET = 960 min
    // 17:00 ET = 1020 min

    if !(510..1020).contains(&min_of_day) {
        MarketSessionPhase::IdleOffSession
    } else if min_of_day < 570 {
        MarketSessionPhase::WakingPreSession
    } else if min_of_day < 960 {
        MarketSessionPhase::OpenSessionActive
    } else {
        MarketSessionPhase::PostSessionWindDown
    }
}

/// Checks if entry is allowed. Requires:
/// 1. Current phase must be `OpenSessionActive` (09:30 to 16:00 ET).
/// 2. If within the first hour of trading, the configured `open_delay_minutes` must have elapsed.
pub fn is_entry_allowed(ts: i64, open_delay_minutes: usize) -> bool {
    let phase = get_market_session_phase(ts);
    if phase != MarketSessionPhase::OpenSessionActive {
        return false;
    }

    if let Some(et) = utc_to_us_eastern(ts) {
        let min_of_day = et.hour() * 60 + et.minute();
        let earliest_allowed = 9 * 60 + 30 + (open_delay_minutes as u32);
        min_of_day >= earliest_allowed
    } else {
        false
    }
}

/// Checks if session exit is due. True if in `PostSessionWindDown` (after 16:00 ET)
/// or within the final 15 minutes of regular market close (>= 15:45 ET).
pub fn is_session_exit_due(ts: i64) -> bool {
    let phase = get_market_session_phase(ts);
    if phase == MarketSessionPhase::PostSessionWindDown || phase == MarketSessionPhase::IdleOffSession {
        return true;
    }

    if let Some(et) = utc_to_us_eastern(ts) {
        let min_of_day = et.hour() * 60 + et.minute();
        // 15:45 ET = 945 minutes (15 min before regular close)
        min_of_day >= 15 * 60 + 45
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
