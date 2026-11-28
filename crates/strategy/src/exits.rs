use crate::model::StrategyParams;

/// Evaluates intrabar risk exits and AI-assisted dynamic trailing stops.
#[derive(Debug, Clone)]
pub struct RiskTracker {
    pub symbol: String,
    pub entry_price: f64,
    pub qty: f64,
    pub peak_price: f64,
    pub params: StrategyParams,
}

impl RiskTracker {
    pub fn new(symbol: String, entry_price: f64, qty: f64, params: StrategyParams) -> Self {
        Self {
            symbol,
            entry_price,
            qty,
            peak_price: entry_price,
            params,
        }
    }

    /// Evaluates executable intrabar risk exits without consuming a signal bar.
    pub fn evaluate_intrabar_exit(&mut self, current_price: f64) -> Option<&'static str> {
        if self.qty <= 0.0 || self.entry_price <= 0.0 || current_price <= 0.0 {
            return None;
        }

        if current_price > self.peak_price {
            self.peak_price = current_price;
        }

        // 1. Take Profit
        if self.params.take_profit_pct > 0.0
            && current_price >= self.entry_price * (1.0 + self.params.take_profit_pct)
        {
            return Some("TAKE_PROFIT");
        }

        // 2. Stop Loss
        if self.params.stop_loss_pct > 0.0
            && current_price <= self.entry_price * (1.0 - self.params.stop_loss_pct)
        {
            return Some("STOP_LOSS");
        }

        // 3. AI-assisted Trailing Stop
        let trailing_armed = self.params.trailing_activation_pct > 0.0
            && self.params.trailing_stop_pct > 0.0
            && self.peak_price >= self.entry_price * (1.0 + self.params.trailing_activation_pct);

        if trailing_armed && current_price <= self.peak_price * (1.0 - self.params.trailing_stop_pct) {
            return Some("TRAILING_STOP");
        }

        None
    }

    /// Returns whether the trailing stop is currently armed.
    pub fn is_trailing_armed(&self) -> bool {
        self.params.trailing_activation_pct > 0.0
            && self.params.trailing_stop_pct > 0.0
            && self.peak_price >= self.entry_price * (1.0 + self.params.trailing_activation_pct)
    }

    /// Returns the current stop level if trailing is armed.
    pub fn trailing_stop_price(&self) -> Option<f64> {
        if self.is_trailing_armed() {
            Some(self.peak_price * (1.0 - self.params.trailing_stop_pct))
        } else {
            None
        }
    }
}
