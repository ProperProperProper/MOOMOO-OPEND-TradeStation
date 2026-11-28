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

pub use model::{BacktestResult, EntryMode, ExitMode, GridLevels, PromotionGate, StrategyMarker, StrategyParams, StrategySignal, SymbolPaperConfig, Trade, TradingSystemConfig};
pub use grid::{calculate_grid_levels, evaluate_signals, strategy_signal_latest};
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
}
