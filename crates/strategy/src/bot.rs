use std::path::PathBuf;
use anyhow::{bail, Result};
use tr_moomoo::{clean_ticker, OpenDClient};
use crate::exits::RiskTracker;
use crate::grid::{evaluate_signals, is_entry_allowed, is_session_exit_due};
use crate::model::{StrategyParams, SymbolPaperConfig, Trade};
use crate::optimizer::aggregate_bars;
use crate::storage::{load_symbol_strategy, load_trading_system, save_symbol_strategy, save_trading_system};

/// Local paper-trading autonomous bot engine managing SIMULATE accounts.
#[derive(Debug, Clone)]
pub struct PaperTradingBot {
    pub client: OpenDClient,
    pub config_root: PathBuf,
    pub active_symbol: String,
    pub active_interval: usize,
    pub active_params: StrategyParams,
    pub running: bool,
    pub position_qty: f64,
    pub entry_price: f64,
    pub peak_price: f64,
    pub trailing_armed: bool,
    pub paper_balance: f64,
    pub last_trade: Option<Trade>,
    pub trade_history: Vec<Trade>,
    pub status_message: String,
}

impl PaperTradingBot {
    pub fn new(client: OpenDClient, config_root: PathBuf) -> Self {
        let mut bot = Self {
            client,
            config_root,
            active_symbol: "US.MRNA".to_string(),
            active_interval: 8,
            active_params: StrategyParams::default(),
            running: false,
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

    /// Reloads portfolio and active symbol strategy from disk.
    pub fn reload_config(&mut self) -> Result<()> {
        let sys = load_trading_system(&self.config_root)?;
        self.active_symbol = sys.active_symbol;
        let strat = load_symbol_strategy(&self.config_root, &self.active_symbol)?;
        self.active_interval = strat.interval.clamp(1, 10);
        self.active_params = strat.params;
        self.status_message = format!("Active: {} ({}m)", self.active_symbol, self.active_interval);
        Ok(())
    }

    /// Switches the active trading symbol; enforces flat position check.
    pub fn switch_symbol(&mut self, new_symbol: &str) -> Result<()> {
        if self.position_qty > 0.0 {
            bail!("HARD RULE: Cannot switch symbol while holding open position (qty={})", self.position_qty);
        }
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
        self.status_message = format!("Switched to {} ({}m)", self.active_symbol, self.active_interval);
        Ok(())
    }

    /// Promotes an optimized winner to active paper strategy; enforces flat position check.
    pub fn promote_winner(&mut self, symbol: &str, interval: usize, params: StrategyParams) -> Result<()> {
        if self.position_qty > 0.0 {
            bail!("HARD RULE: Cannot promote strategy while holding open position (qty={})", self.position_qty);
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
            self.status_message = format!("Promoted new strategy for {} ({}m)", symbol, interval);
        }
        Ok(())
    }

    /// Evaluates one step of the paper bot against latest market data and active positions.
    pub async fn tick(&mut self) -> Result<Option<String>> {
        if !self.running {
            return Ok(None);
        }

        // 1. Fetch latest quote & balance from OpenD
        let quote = match self.client.fetch_quote(&self.active_symbol).await {
            Ok(q) => q,
            Err(e) => {
                self.status_message = format!("Quote error: {e}");
                return Ok(None);
            }
        };

        let current_price = quote.last;

        // 2. Fetch 1m candles and aggregate
        let bars_1m = self.client.fetch_1min_history(&self.active_symbol, 200).await.unwrap_or_default();
        if bars_1m.is_empty() {
            self.status_message = "Waiting for candles...".to_string();
            return Ok(None);
        }

        let bars = aggregate_bars(&bars_1m, self.active_interval);
        let signals = evaluate_signals(&bars, &self.active_params);

        let latest_signal = signals.last();

        // 3. Handle Active Position Exits
        if self.position_qty > 0.0 {
            let mut risk_tracker = RiskTracker::new(
                self.active_symbol.clone(),
                self.entry_price,
                self.position_qty,
                self.active_params.clone(),
            );
            risk_tracker.peak_price = self.peak_price.max(current_price);
            self.peak_price = risk_tracker.peak_price;
            self.trailing_armed = risk_tracker.is_trailing_armed();

            let mut exit_reason = None;

            // Check intrabar risk exits (take profit, stop loss, trailing stop)
            if let Some(r) = risk_tracker.evaluate_intrabar_exit(current_price) {
                exit_reason = Some(r);
            }

            // Check session exit (30m before close)
            if exit_reason.is_none() && is_session_exit_due(quote.timestamp) {
                exit_reason = Some("SESSION_EXIT_30M_BEFORE_CLOSE");
            }

            // Check candle open cross downward
            if exit_reason.is_none()
                && let Some(sig) = latest_signal
                    && sig.close_signal {
                        exit_reason = Some("OPEN_CROSSED_GRID_DOWN");
                    }

            if let Some(reason) = exit_reason {
                let exit_price = current_price * (1.0 - 0.0002);
                let gross = (exit_price - self.entry_price) * self.position_qty;
                let net = gross;
                self.paper_balance += self.position_qty * exit_price;
                let return_pct = (net / (self.entry_price * self.position_qty)) * 100.0;

                let trade = Trade {
                    entry_time: chrono::Utc::now().to_rfc3339(),
                    exit_time: chrono::Utc::now().to_rfc3339(),
                    entry_price: self.entry_price,
                    exit_price,
                    qty: self.position_qty,
                    gross_pnl: gross,
                    costs: 0.0,
                    net_pnl: net,
                    return_pct,
                    reason: reason.to_string(),
                };

                self.last_trade = Some(trade.clone());
                self.trade_history.push(trade);

                self.position_qty = 0.0;
                self.entry_price = 0.0;
                self.peak_price = 0.0;
                self.trailing_armed = false;

                let msg = format!("ORDER EXECUTED: SELL {} @ ${:.2} (Reason: {}, PnL: ${:.2})", self.active_symbol, exit_price, reason, net);
                self.status_message = msg.clone();
                return Ok(Some(msg));
            }
        }

        // 4. Handle Flat Position Entries
        if self.position_qty == 0.0
            && let Some(sig) = latest_signal
                && sig.long_signal && is_entry_allowed(quote.timestamp, self.active_params.open_delay_minutes) {
                    let entry_price = current_price * (1.0 + 0.0002);
                    let allocation = 0.95;
                    let spend = self.paper_balance * allocation;
                    let qty = (spend / entry_price).floor();

                    if qty > 0.0 {
                        self.paper_balance -= qty * entry_price;
                        self.position_qty = qty;
                        self.entry_price = entry_price;
                        self.peak_price = entry_price;
                        self.trailing_armed = false;

                        let msg = format!("ORDER EXECUTED: BUY {} x {:.0} @ ${:.2} (Reason: {})", self.active_symbol, qty, entry_price, sig.entry_mode);
                        self.status_message = msg.clone();
                        return Ok(Some(msg));
                    }
                }

        Ok(None)
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
