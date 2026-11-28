//! DRBT-Grid: Dynamic Range-Break Trend Strategy Engine
//!
//! Implements multi-timeframe interval sweeps (1m–10m), rolling Donchian quartile grid quantization,
//! asymmetric risk-managed exits with dynamic trailing stops, and Apple Silicon E-core thermal governance.

pub mod model;
pub mod grid;
pub mod exits;
pub mod backtest;
pub mod optimizer;
pub mod storage;
pub mod bot;
pub mod governor;

pub use model::{BacktestResult, EntryMode, ExitMode, GridLevels, MarketSessionPhase, PromotionGate, StrategyMarker, StrategyParams, StrategySignal, SymbolPaperConfig, Trade, TradingSystemConfig};
pub use grid::{calculate_grid_levels, evaluate_signals, get_market_session_phase, is_entry_allowed, is_session_exit_due, strategy_signal_latest, utc_to_us_eastern};
pub use exits::RiskTracker;
pub use backtest::{run_backtest, trades_to_markers};
pub use optimizer::{aggregate_bars, evaluate_promotion_gate, generate_candidates, run_interval_sweep};
pub use storage::{load_symbol_strategy, load_trading_system, save_symbol_strategy, save_trading_system, symbol_slug};
pub use bot::PaperTradingBot;
pub use governor::{apply_cooling_qos, check_thermal_status, detect_cpu_topology, get_governor_state, pace_for_cooling, CpuTopology, GovernorState, ThermalStatus};

#[cfg(test)]
mod tests {
    use super::*;
    use tr_moomoo::Bar;

    fn make_test_bars(n: usize) -> Vec<Bar> {
        let mut bars = Vec::with_capacity(n);
        let base_ts = 1700000000;
        let mut price = 100.0;
        for i in 0..n {
            let change = if i % 2 == 0 { 1.5 } else { -0.8 };
            price += change;
            bars.push(Bar {
                ts: base_ts + (i as i64) * 60,
                open: price - 0.5,
                high: price + 1.0,
                low: price - 1.0,
                close: price + 0.4,
                volume: 1000.0,
            });
        }
        bars
    }

    #[test]
    fn test_cooling_governor() {
        let topo = detect_cpu_topology();
        assert!(topo.total_cores > 0);
        assert!(topo.e_cores > 0);
        let gov = get_governor_state();
        assert_eq!(gov.qos_active, true);
        assert_eq!(gov.e_core_pinned, true);
    }

    #[test]
    fn test_grid_calculation() {
        let highs = vec![100.0, 105.0, 110.0, 108.0, 112.0];
        let lows = vec![95.0, 96.0, 98.0, 97.0, 99.0];
        let grid = calculate_grid_levels(5, 4, &highs, &lows).expect("grid calculated");
        assert_eq!(grid.bottom, 95.0);
        assert_eq!(grid.top, 112.0);
        assert_eq!(grid.lower_mid, 95.0 + 17.0 * 0.25);
        assert_eq!(grid.upper_mid, 95.0 + 17.0 * 0.50);
        assert_eq!(grid.levels.len(), 4);
    }

    #[test]
    fn test_signals_generation() {
        let bars = make_test_bars(60);
        let params = StrategyParams {
            grid_lookback: 20,
            grid_intervals: 4,
            ..Default::default()
        };
        let signals = evaluate_signals(&bars, &params);
        assert!(!signals.is_empty());
        assert!(signals[0].grid.top > signals[0].grid.bottom);
    }

    #[test]
    fn test_risk_tracker_exits() {
        let mut tracker = RiskTracker::new(
            "US.ANT".to_string(),
            100.0,
            10.0,
            StrategyParams {
                take_profit_pct: 0.05,
                stop_loss_pct: 0.05,
                trailing_activation_pct: 0.02,
                trailing_stop_pct: 0.01,
                ..Default::default()
            },
        );

        // Before triggers
        assert_eq!(tracker.evaluate_intrabar_exit(101.0), None);
        assert!(!tracker.is_trailing_armed());

        // Price rises to 102.5 -> trailing arms!
        assert_eq!(tracker.evaluate_intrabar_exit(102.5), None);
        assert!(tracker.is_trailing_armed());
        assert_eq!(tracker.peak_price, 102.5);

        // Price falls 1% from 102.5 peak to 101.4 -> trailing stop triggers!
        assert_eq!(tracker.evaluate_intrabar_exit(101.4), Some("TRAILING_STOP"));
    }

    #[test]
    fn test_backtest_runner() {
        let bars = make_test_bars(120);
        let params = StrategyParams {
            grid_lookback: 20,
            grid_intervals: 4,
            take_profit_pct: 0.03,
            stop_loss_pct: 0.05,
            ..Default::default()
        };
        let res = run_backtest("US.ANT", 1, &bars, &params, 100_000.0, 0.95, 2.0, 0.0);
        assert_eq!(res.symbol, "US.ANT");
        assert!(res.ending_equity > 0.0);
    }

    #[test]
    fn test_promotion_gate() {
        let base = BacktestResult {
            symbol: "US.ANT".into(),
            interval: 10,
            params: StrategyParams::default(),
            trades: vec![],
            total_return_pct: 1.5,
            win_rate: 60.0,
            profit_factor: 1.5,
            max_drawdown: 2.0,
            total_trades: 5,
            sharpe: 1.2,
            ending_equity: 101500.0,
        };

        let mut candidate = base.clone();
        candidate.total_return_pct = 3.2;

        // Fails with < 2 sessions
        let gate = evaluate_promotion_gate(&candidate, &base, 1);
        assert!(!gate.allowed);

        // Succeeds with 2 sessions and higher return
        let gate = evaluate_promotion_gate(&candidate, &base, 2);
        assert!(gate.allowed);
        assert!(gate.beats_baseline);
    }

    #[test]
    fn test_market_session_lifecycle() {
        // Wednesday, Oct 14, 2026 (EDT is UTC-4: subtract 4 hours from UTC to get ET)
        // 04:00 ET -> 08:00 UTC
        let ts_off_session = 1791964800; // 2026-10-14 08:00:00 UTC = 04:00:00 EDT
        assert_eq!(get_market_session_phase(ts_off_session), MarketSessionPhase::IdleOffSession);
        assert!(!is_entry_allowed(ts_off_session, 5));

        // 08:30 ET -> 12:30 UTC (wakes 1 hour before session open)
        let ts_pre_session_start = 1791981000; // 2026-10-14 12:30:00 UTC = 08:30:00 EDT
        assert_eq!(get_market_session_phase(ts_pre_session_start), MarketSessionPhase::WakingPreSession);
        assert!(!is_entry_allowed(ts_pre_session_start, 5)); // No entries during pre-market prep

        // 09:15 ET -> 13:15 UTC (still pre-session)
        let ts_pre_session_mid = 1791983700; // 2026-10-14 13:15:00 UTC = 09:15:00 EDT
        assert_eq!(get_market_session_phase(ts_pre_session_mid), MarketSessionPhase::WakingPreSession);
        assert!(!is_entry_allowed(ts_pre_session_mid, 5));

        // 09:32 ET -> 13:32 UTC (open session, but 5m open delay has not elapsed)
        let ts_open_delayed = 1791984720; // 2026-10-14 13:32:00 UTC = 09:32:00 EDT
        assert_eq!(get_market_session_phase(ts_open_delayed), MarketSessionPhase::OpenSessionActive);
        assert!(!is_entry_allowed(ts_open_delayed, 5));

        // 09:36 ET -> 13:36 UTC (open session, 5m open delay elapsed -> allowed!)
        let ts_open_active = 1791984960; // 2026-10-14 13:36:00 UTC = 09:36:00 EDT
        assert_eq!(get_market_session_phase(ts_open_active), MarketSessionPhase::OpenSessionActive);
        assert!(is_entry_allowed(ts_open_active, 5));
        assert!(!is_session_exit_due(ts_open_active));

        // 15:50 ET -> 19:50 UTC (final 15m before close -> exit due)
        let ts_near_close = 1792007400; // 2026-10-14 19:50:00 UTC = 15:50:00 EDT
        assert_eq!(get_market_session_phase(ts_near_close), MarketSessionPhase::OpenSessionActive);
        assert!(is_session_exit_due(ts_near_close));

        // 16:15 ET -> 20:15 UTC (post-session wind-down: 1 hour after close)
        let ts_post_session = 1792008900; // 2026-10-14 20:15:00 UTC = 16:15:00 EDT
        assert_eq!(get_market_session_phase(ts_post_session), MarketSessionPhase::PostSessionWindDown);
        assert!(!is_entry_allowed(ts_post_session, 5));
        assert!(is_session_exit_due(ts_post_session));

        // 17:05 ET -> 21:05 UTC (after 17:00 ET -> completely idle until 08:30 next morning)
        let ts_evening_idle = 1792011900; // 2026-10-14 21:05:00 UTC = 17:05:00 EDT
        assert_eq!(get_market_session_phase(ts_evening_idle), MarketSessionPhase::IdleOffSession);
        assert!(!is_entry_allowed(ts_evening_idle, 5));

        // Saturday Oct 17, 2026 12:00 ET -> Weekend is completely idle
        let ts_weekend = 1792252800;
        assert_eq!(get_market_session_phase(ts_weekend), MarketSessionPhase::IdleOffSession);
    }
}
