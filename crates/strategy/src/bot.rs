use std::collections::HashMap;
use std::path::PathBuf;
use anyhow::{bail, Result};
use tr_moomoo::{clean_ticker, OpenDClient};
use crate::exits::RiskTracker;
use crate::grid::{evaluate_signals, get_market_session_phase, is_entry_allowed, is_session_exit_due, utc_to_us_eastern};
use crate::model::{MarketSessionPhase, PaperPosition, StrategyParams, SymbolPaperConfig, Trade};
use crate::optimizer::aggregate_bars;
use crate::storage::{load_symbol_strategy, load_trading_system, save_symbol_strategy, save_trading_system};

/// Local paper-trading autonomous bot engine managing SIMULATE accounts.
/// Hard rule: Live paper trading can only have at most ONE position open per symbol.
#[derive(Debug, Clone)]
pub struct PaperTradingBot {
    pub client: OpenDClient,
    pub config_root: PathBuf,
    pub active_symbol: String,
    pub active_interval: usize,
    pub active_params: StrategyParams,
    pub session_phase: MarketSessionPhase,
    pub running: bool,
    /// Active positions tracked per symbol. Invariant: At most ONE position per symbol.
    pub positions: HashMap<String, PaperPosition>,
    pub paper_balance: f64,
    pub last_trade: Option<Trade>,
    pub trade_history: Vec<Trade>,
    pub status_message: String,
    // Active symbol convenience projections for UI:
    pub position_qty: f64,
    pub entry_price: f64,
    pub peak_price: f64,
    pub trailing_armed: bool,
}

impl PaperTradingBot {
    pub fn new(client: OpenDClient, config_root: PathBuf) -> Self {
        let mut bot = Self {
            client,
            config_root,
            active_symbol: "US.MRNA".to_string(),
            active_interval: 8,
            active_params: StrategyParams::default(),
            session_phase: MarketSessionPhase::IdleOffSession,
            running: false,
            positions: HashMap::new(),
            position_qty: 0.0,
            entry_price: 0.0,
            peak_price: 0.0,
            trailing_armed: false,
            paper_balance: 100_000.0,
            last_trade: None,
            trade_history: Vec::new(),
            status_message: "Initialized".to_string(),
        };
        let _ = bot.reload_config();
        bot
    }

    /// Checks if a symbol currently has an active open position.
    /// Strictly enforces: at most ONE position open per symbol.
    pub fn has_open_position(&self, symbol: &str) -> bool {
        self.positions.get(symbol).map(|p| p.qty > 0.0).unwrap_or(false)
    }

    /// Returns a reference to the active position for a symbol if one exists.
    pub fn position_for(&self, symbol: &str) -> Option<&PaperPosition> {
        self.positions.get(symbol)
    }

    /// Syncs projection fields for the active symbol so UI metric cards render correctly.
    pub fn sync_active_projections(&mut self) {
        if let Some(pos) = self.positions.get(&self.active_symbol) {
            self.position_qty = pos.qty;
            self.entry_price = pos.entry_price;
            self.peak_price = pos.peak_price;
            self.trailing_armed = pos.trailing_armed;
        } else {
            self.position_qty = 0.0;
            self.entry_price = 0.0;
            self.peak_price = 0.0;
            self.trailing_armed = false;
        }
    }

    /// Reloads portfolio and active symbol strategy from disk.
    pub fn reload_config(&mut self) -> Result<()> {
        let sys = load_trading_system(&self.config_root)?;
        self.active_symbol = sys.active_symbol;
        let strat = load_symbol_strategy(&self.config_root, &self.active_symbol)?;
        self.active_interval = strat.interval.clamp(1, 10);
        self.active_params = strat.params;
        self.sync_active_projections();
        let pos_str = if self.position_qty > 0.0 {
            format!("Open: {:.0} shs @ ${:.2}", self.position_qty, self.entry_price)
        } else {
            "Flat".to_string()
        };
        self.status_message = format!("Active: {} ({}m, {})", self.active_symbol, self.active_interval, pos_str);
        Ok(())
    }

    /// Switches the active trading symbol; each symbol independently maintains its own single position.
    pub fn switch_symbol(&mut self, new_symbol: &str) -> Result<()> {
        let clean = clean_ticker(new_symbol);
        let normalized = if clean.contains('.') { clean } else { format!("US.{}", clean) };
        self.active_symbol = normalized.clone();

        // Update trading_system.json
        if let Ok(mut sys) = load_trading_system(&self.config_root) {
            sys.active_symbol = normalized.clone();
            if !sys.active_symbols.contains(&normalized) {
                sys.active_symbols.push(normalized.clone());
            }
            let _ = save_trading_system(&self.config_root, &sys);
        }

        // Load new symbol strategy
        let strat = load_symbol_strategy(&self.config_root, &normalized)?;
        self.active_interval = strat.interval.clamp(1, 10);
        self.active_params = strat.params;

        self.sync_active_projections();
        let pos_str = if self.position_qty > 0.0 {
            format!("Open: {:.0} shs @ ${:.2}", self.position_qty, self.entry_price)
        } else {
            "Flat".to_string()
        };
        self.status_message = format!("Switched to {} ({}m, {})", self.active_symbol, self.active_interval, pos_str);
        Ok(())
    }

    /// Promotes an optimized winner to active paper strategy; enforces flat check on target symbol.
    pub fn promote_winner(&mut self, symbol: &str, interval: usize, params: StrategyParams) -> Result<()> {
        if self.has_open_position(symbol) {
            bail!("HARD RULE: Cannot promote strategy for {} while holding an open position in it", symbol);
        }

        let config = SymbolPaperConfig {
            symbol: symbol.to_string(),
            interval: interval.clamp(1, 10),
            mode: "SIMULATE".to_string(),
            params: params.clone(),
            run_id: format!("{}-SWEEP-{}", symbol.replace('.', "-"), chrono::Utc::now().format("%Y%m%d-%H%M%S")),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };

        save_symbol_strategy(&self.config_root, &config)?;

        if self.active_symbol.eq_ignore_ascii_case(symbol) {
            self.active_interval = config.interval;
            self.active_params = params;
            self.sync_active_projections();
            self.status_message = format!("Promoted new strategy for {} ({}m)", symbol, interval);
        }
        Ok(())
    }

    /// Evaluates one step of the paper bot against latest market data and active positions.
    /// Strictly enforces: at most ONE position open per symbol.
    pub async fn tick(&mut self) -> Result<Option<String>> {
        if !self.running {
            return Ok(None);
        }

        let now_ts = chrono::Utc::now().timestamp();
        let phase = get_market_session_phase(now_ts);
        self.session_phase = phase;

        let et_str = utc_to_us_eastern(now_ts)
            .map(|et| et.format("%H:%M:%S ET").to_string())
            .unwrap_or_else(|| "ET".into());

        // 1. Off-Session / Sleep Phase (17:00 ET to 08:30 ET & weekends):
        // Completely idle and doing NOTHING outside open session.
        if phase == MarketSessionPhase::IdleOffSession {
            self.status_message = format!("🌙 IDLE (Off-Session) · {} · Sleeping until 08:30 ET", et_str);
            return Ok(None);
        }

        // 2. Pre-Session Wakeup Phase (08:30 ET to 09:30 ET - 1 hour before session open):
        // Wakes up, warms up quotes and calculates grid levels, flat position enforced (zero new entries).
        if phase == MarketSessionPhase::WakingPreSession {
            if let Ok(quote) = self.client.fetch_quote(&self.active_symbol).await {
                if let Ok(bars_1m) = self.client.fetch_1min_history(&self.active_symbol, 200).await {
                    let bars = aggregate_bars(&bars_1m, self.active_interval);
                    let _ = evaluate_signals(&bars, &self.active_params);
                }
                self.status_message = format!("🌅 WAKING UP (Pre-Session) · {} · Ready for 09:30 ET open (${:.2})", et_str, quote.last);
            } else {
                self.status_message = format!("🌅 WAKING UP (Pre-Session) · {} · Pre-market sync...", et_str);
            }
            return Ok(None);
        }

        // 3. Post-Session Wind-Down (16:00 ET to 17:00 ET - 1 hour after session close):
        // Liquidate any remaining open positions across all symbols. Once flat, idle.
        if phase == MarketSessionPhase::PostSessionWindDown {
            if self.positions.is_empty() {
                self.status_message = format!("🌆 POST-SESSION (Wind-Down) · {} · All positions flat; idle", et_str);
                return Ok(None);
            }

            let open_syms: Vec<String> = self.positions.keys().cloned().collect();
            let mut last_closed = None;
            for sym in open_syms {
                if let Some(pos) = self.positions.remove(&sym) {
                    let current_price = match self.client.fetch_quote(&sym).await {
                        Ok(q) => q.last,
                        Err(_) => pos.entry_price,
                    };
                    let exit_price = current_price * (1.0 - 0.0002);
                    let gross = (exit_price - pos.entry_price) * pos.qty;
                    let net = gross;
                    self.paper_balance += pos.qty * exit_price;
                    let return_pct = (net / (pos.entry_price * pos.qty)) * 100.0;

                    let trade = Trade {
                        entry_time: pos.entry_time,
                        exit_time: chrono::Utc::now().to_rfc3339(),
                        entry_price: pos.entry_price,
                        exit_price,
                        qty: pos.qty,
                        gross_pnl: gross,
                        costs: 0.0,
                        net_pnl: net,
                        return_pct,
                        reason: "POST_SESSION_CLOSE_EXIT".to_string(),
                    };

                    self.last_trade = Some(trade.clone());
                    self.trade_history.push(trade);
                    last_closed = Some(format!("ORDER EXECUTED: SELL {} @ ${:.2} (Wind-Down, PnL: ${:.2})", sym, exit_price, net));
                }
            }
            self.sync_active_projections();
            if let Some(msg) = last_closed {
                self.status_message = msg.clone();
                return Ok(Some(msg));
            }
            return Ok(None);
        }

        // 4. Regular Session Trading: Evaluate active symbol + any symbol currently holding an open position.
        let mut symbols_to_check = vec![self.active_symbol.clone()];
        for sym in self.positions.keys() {
            if !symbols_to_check.contains(sym) {
                symbols_to_check.push(sym.clone());
            }
        }

        let mut executed_event = None;

        for sym in symbols_to_check {
            let quote = match self.client.fetch_quote(&sym).await {
                Ok(q) => q,
                Err(e) => {
                    if sym == self.active_symbol {
                        self.status_message = format!("Quote error for {}: {e}", sym);
                    }
                    continue;
                }
            };
            let current_price = quote.last;

            let (strat_interval, strat_params) = if sym == self.active_symbol {
                (self.active_interval, self.active_params.clone())
            } else if let Ok(s) = load_symbol_strategy(&self.config_root, &sym) {
                (s.interval.clamp(1, 10), s.params)
            } else {
                (8, StrategyParams::default())
            };

            let bars_1m = self.client.fetch_1min_history(&sym, 200).await.unwrap_or_default();
            if bars_1m.is_empty() {
                continue;
            }
            let bars = aggregate_bars(&bars_1m, strat_interval);
            let signals = evaluate_signals(&bars, &strat_params);
            let latest_signal = signals.last();

            // A. Check exit if this symbol currently holds an open position
            if let Some(mut pos) = self.positions.get(&sym).cloned() {
                let mut risk_tracker = RiskTracker::new(
                    sym.clone(),
                    pos.entry_price,
                    pos.qty,
                    strat_params.clone(),
                );
                risk_tracker.peak_price = pos.peak_price.max(current_price);
                pos.peak_price = risk_tracker.peak_price;
                pos.trailing_armed = risk_tracker.is_trailing_armed();

                let mut exit_reason = None;

                // Check intrabar risk exits (take profit, stop loss, trailing stop)
                if let Some(r) = risk_tracker.evaluate_intrabar_exit(current_price) {
                    exit_reason = Some(r);
                }

                // Check session exit window (final 15m of regular session)
                if exit_reason.is_none() && is_session_exit_due(quote.timestamp) {
                    exit_reason = Some("SESSION_EXIT_WINDOW");
                }

                // Check candle open cross downward
                if exit_reason.is_none()
                    && let Some(sig) = latest_signal
                    && sig.close_signal {
                        exit_reason = Some("OPEN_CROSSED_GRID_DOWN");
                    }

                if let Some(reason) = exit_reason {
                    // Liquidate position for sym
                    let exit_price = current_price * (1.0 - 0.0002);
                    let gross = (exit_price - pos.entry_price) * pos.qty;
                    let net = gross;
                    self.paper_balance += pos.qty * exit_price;
                    let return_pct = (net / (pos.entry_price * pos.qty)) * 100.0;

                    let trade = Trade {
                        entry_time: pos.entry_time.clone(),
                        exit_time: chrono::Utc::now().to_rfc3339(),
                        entry_price: pos.entry_price,
                        exit_price,
                        qty: pos.qty,
                        gross_pnl: gross,
                        costs: 0.0,
                        net_pnl: net,
                        return_pct,
                        reason: reason.to_string(),
                    };

                    self.positions.remove(&sym);
                    self.last_trade = Some(trade.clone());
                    self.trade_history.push(trade);

                    let msg = format!("ORDER EXECUTED: SELL {} @ ${:.2} (Reason: {}, PnL: ${:.2})", sym, exit_price, reason, net);
                    self.status_message = msg.clone();
                    executed_event = Some(msg);
                } else {
                    // Update peak price and trailing armed in positions map
                    self.positions.insert(sym.clone(), pos);
                }
            } else {
                // B. Handle Flat Position Entry:
                // HARD RULE: Only one position open per symbol.
                // If sym is already in self.positions, do NOT enter (strictly 1 position max).
                if !self.has_open_position(&sym)
                    && let Some(sig) = latest_signal
                    && sig.long_signal && is_entry_allowed(quote.timestamp, strat_params.open_delay_minutes) {
                        let entry_price = current_price * (1.0 + 0.0002);
                        // Portfolio allocation: divide available cash evenly among potential open slots (e.g. up to 4 concurrent positions)
                        let max_concurrent_symbols = 4.0;
                        let available_slots = (max_concurrent_symbols - self.positions.len() as f64).max(1.0);
                        let allocation = 0.95 / available_slots;
                        let spend = (self.paper_balance * allocation).min(self.paper_balance * 0.95);
                        let qty = (spend / entry_price).floor();

                        if qty > 0.0 {
                            self.paper_balance -= qty * entry_price;
                            let new_pos = PaperPosition {
                                symbol: sym.clone(),
                                qty,
                                entry_price,
                                entry_time: chrono::Utc::now().to_rfc3339(),
                                peak_price: entry_price,
                                trailing_armed: false,
                            };
                            self.positions.insert(sym.clone(), new_pos);

                            let msg = format!("ORDER EXECUTED: BUY {} x {:.0} @ ${:.2} (Reason: {})", sym, qty, entry_price, sig.entry_mode);
                            self.status_message = msg.clone();
                            executed_event = Some(msg);
                        }
                    }
            }
        }

        self.sync_active_projections();
        Ok(executed_event)
    }

    pub fn start(&mut self) {
        self.running = true;
        self.status_message = format!("Bot running on {}", self.active_symbol);
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.status_message = "Bot stopped".to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_one_position_per_symbol_constraint() {
        let temp_dir = std::env::temp_dir().join(format!("tr_bot_test_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        let _ = std::fs::create_dir_all(&temp_dir);

        let mut bot = PaperTradingBot::new(OpenDClient::default(), temp_dir.clone());
        bot.active_symbol = "US.MRNA".to_string();

        assert!(!bot.has_open_position("US.MRNA"));
        assert!(!bot.has_open_position("US.NVDA"));
        assert_eq!(bot.position_qty, 0.0);

        // Open 1 position for MRNA
        bot.positions.insert("US.MRNA".to_string(), PaperPosition {
            symbol: "US.MRNA".to_string(),
            qty: 100.0,
            entry_price: 25.50,
            entry_time: chrono::Utc::now().to_rfc3339(),
            peak_price: 25.50,
            trailing_armed: false,
        });
        bot.sync_active_projections();

        assert!(bot.has_open_position("US.MRNA"));
        assert_eq!(bot.position_qty, 100.0);
        assert_eq!(bot.entry_price, 25.50);

        // Cannot promote MRNA while holding open position
        let promote_err = bot.promote_winner("US.MRNA", 5, StrategyParams::default());
        assert!(promote_err.is_err());
        assert!(promote_err.unwrap_err().to_string().contains("holding an open position"));

        // But NVDA is flat, so can promote or trade NVDA
        assert!(!bot.has_open_position("US.NVDA"));

        // Open 1 position for NVDA (independent position per symbol)
        bot.positions.insert("US.NVDA".to_string(), PaperPosition {
            symbol: "US.NVDA".to_string(),
            qty: 50.0,
            entry_price: 135.00,
            entry_time: chrono::Utc::now().to_rfc3339(),
            peak_price: 135.00,
            trailing_armed: false,
        });

        assert!(bot.has_open_position("US.MRNA"));
        assert!(bot.has_open_position("US.NVDA"));
        assert!(!bot.has_open_position("US.AAPL"));

        // Switch active symbol to NVDA: unblocked, projections sync to NVDA
        let _ = bot.switch_symbol("US.NVDA");
        assert_eq!(bot.active_symbol, "US.NVDA");
        assert_eq!(bot.position_qty, 50.0);
        assert_eq!(bot.entry_price, 135.00);

        // Switch active symbol to AAPL (flat): unblocked, projections sync to flat
        let _ = bot.switch_symbol("US.AAPL");
        assert_eq!(bot.active_symbol, "US.AAPL");
        assert_eq!(bot.position_qty, 0.0);
        assert_eq!(bot.entry_price, 0.0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
