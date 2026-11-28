use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntryMode {
    #[default]
    AnyGridCross,
    TopBreakout,
    UpperHalfCross,
    LowerHalfCross,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExitMode {
    #[default]
    AnyGridCrossDown,
    TopBreakdown,
    BottomBreakdown,
    LowerHalfBreakdown,
}

fn default_grid_lookback() -> usize { 45 }
fn default_grid_intervals() -> usize { 4 }
fn default_open_delay() -> usize { 5 }
fn default_stop_loss() -> f64 { 0.1324 }
fn default_take_profit() -> f64 { 0.0274 }
fn default_active_symbol() -> String { "US.MRNA".to_string() }
fn default_mode() -> String { "SIMULATE".to_string() }
fn default_allocation() -> f64 { 0.95 }
fn default_workers() -> usize { 2 }
fn default_max_cpu() -> f64 { 78.0 }
fn default_interval() -> usize { 10 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyParams {
    #[serde(default = "default_grid_lookback")]
    pub grid_lookback: usize,
    #[serde(default = "default_grid_intervals")]
    pub grid_intervals: usize,
    #[serde(default)]
    pub entry_mode: EntryMode,
    #[serde(default)]
    pub exit_mode: ExitMode,
    #[serde(default = "default_open_delay")]
    pub open_delay_minutes: usize,
    #[serde(default = "default_stop_loss")]
    pub stop_loss_pct: f64,
    #[serde(default = "default_take_profit")]
    pub take_profit_pct: f64,
    #[serde(default)]
    pub trailing_activation_pct: f64,
    #[serde(default)]
    pub trailing_stop_pct: f64,
}

impl Default for StrategyParams {
    fn default() -> Self {
        Self {
            grid_lookback: default_grid_lookback(),
            grid_intervals: default_grid_intervals(),
            entry_mode: EntryMode::AnyGridCross,
            exit_mode: ExitMode::AnyGridCrossDown,
            open_delay_minutes: default_open_delay(),
            stop_loss_pct: default_stop_loss(),
            take_profit_pct: default_take_profit(),
            trailing_activation_pct: 0.0,
            trailing_stop_pct: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingSystemConfig {
    #[serde(default = "default_active_symbol")]
    pub active_symbol: String,
    #[serde(default)]
    pub active_symbols: Vec<String>,
    #[serde(default)]
    pub market: String,
    #[serde(default)]
    pub minute_intervals: Vec<usize>,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_allocation")]
    pub portfolio_allocation: f64,
    #[serde(default = "default_workers")]
    pub research_workers: usize,
    #[serde(default = "default_max_cpu")]
    pub max_cpu_percent: f64,
}

impl Default for TradingSystemConfig {
    fn default() -> Self {
        Self {
            active_symbol: "US.MRNA".to_string(),
            active_symbols: vec![
                "US.MRNA".into(),
                "US.ANT".into(),
                "US.NVDA".into(),
                "US.MU".into(),
                "US.SNDK".into(),
                "US.OMH".into(),
                "US.APP".into(),
            ],
            market: "US".to_string(),
            minute_intervals: (1..=10).collect(),
            mode: "SIMULATE".to_string(),
            portfolio_allocation: 0.95,
            research_workers: 2,
            max_cpu_percent: 78.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolPaperConfig {
    pub symbol: String,
    #[serde(default = "default_interval")]
    pub interval: usize,
    #[serde(default = "default_mode")]
    pub mode: String,
    pub params: StrategyParams,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridLevels {
    pub bottom: f64,
    pub lower_mid: f64,
    pub upper_mid: f64,
    pub top: f64,
    pub levels: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategySignal {
    pub bar_time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub grid: GridLevels,
    pub buy_signal: bool,
    pub close_signal: bool,
    pub long_signal: bool,
    pub exit_signal: bool,
    pub entry_allowed: bool,
    pub session_exit: bool,
    pub entry_mode: String,
    pub exit_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub entry_time: String,
    pub exit_time: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub qty: f64,
    pub gross_pnl: f64,
    pub costs: f64,
    pub net_pnl: f64,
    pub return_pct: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestResult {
    pub symbol: String,
    pub interval: usize,
    pub params: StrategyParams,
    pub trades: Vec<Trade>,
    pub total_return_pct: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub max_drawdown: f64,
    pub total_trades: usize,
    pub sharpe: f64,
    pub ending_equity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionGate {
    pub allowed: bool,
    pub reason: String,
    pub beats_baseline: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyMarker {
    pub ts: i64,
    pub price: f64,
    pub is_buy: bool,
    pub label: String,
    pub pnl_pct: Option<f64>,
}
