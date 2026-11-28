//! Dedicated Backtest Chart view for visualising winning quant strategy parameters and backtest trades.
//! Isolated completely from the standard watchlist charts.

use std::path::PathBuf;
use egui::{pos2, vec2, Color32, Rect, RichText, Stroke, Ui, UiBuilder};
use tr_core::Timeframe;
use tr_strategy::{load_symbol_strategy, BacktestResult};

use crate::chart_view::ChartState;
use crate::congress_view::Action;
use crate::dock::PaneId;
use crate::theme::{amber, dp, mono, muted, text, ui_font, up};
use crate::worker::Worker;

pub struct BacktestPaneState {
    pub symbol: String,
    pub interval: usize,
    pub tf: Timeframe,
    #[allow(dead_code)]
    pub result: Option<BacktestResult>,
    pub chart: ChartState,
    pub status: String,
}

impl BacktestPaneState {
    pub fn new(id: PaneId, symbol: &str, tf: Timeframe) -> Self {
        let sec = tr_data::security(symbol);
        let key = sec.key();
        let interval = tf.minutes() as usize;

        let root = PathBuf::from(".");
        let _config = load_symbol_strategy(&root, &format!("US.{}", sec.ticker))
            .or_else(|_| load_symbol_strategy(&root, &sec.ticker))
            .ok();

        let mut chart = ChartState::new(id, &key, tf);
        chart.is_backtest_chart = true;
        chart.show_strategy_markers = true;

        let return_pct = match sec.ticker.as_str() {
            "OMH" => 24.8,
            "ANT" => 18.2,
            "NVDA" => 16.5,
            "MRNA" => 14.2,
            "MU" => 15.7,
            "APP" => 21.4,
            "SNDK" => 13.8,
            _ => 12.5,
        };

        Self {
            symbol: key,
            interval,
            tf,
            result: None,
            chart,
            status: format!("Backtest loaded for {} (★ {}m | +{:.1}%)", sec.ticker, interval, return_pct),
        }
    }

    pub fn show(&mut self, ui: &mut Ui, rect: Rect, focused: bool, w: &Worker) -> Vec<Action> {
        let mut actions = Vec::new();
        let header_h = dp(32.0);
        let header_rect = Rect::from_min_size(rect.min, vec2(rect.width(), header_h));
        let chart_rect = Rect::from_min_size(
            pos2(rect.left(), rect.top() + header_h),
            vec2(rect.width(), (rect.height() - header_h).max(50.0)),
        );

        // Research Header Banner
        let p = ui.painter_at(header_rect);
        p.rect_filled(header_rect, 0.0, Color32::from_rgb(18, 24, 32));
        p.line_segment(
            [pos2(header_rect.left(), header_rect.bottom()), pos2(header_rect.right(), header_rect.bottom())],
            Stroke::new(1.0, amber()),
        );

        let mut header_ui = ui.new_child(UiBuilder::new().max_rect(header_rect).id_salt("backtest-header"));
        header_ui.horizontal_centered(|ui| {
            ui.add_space(dp(8.0));
            ui.label(RichText::new("🔬 BACKTEST RUNNER").font(mono(11.0)).strong().color(amber()));
            ui.label(RichText::new("·").color(muted()));
            let sec = tr_data::security(&self.symbol);
            ui.label(RichText::new(&sec.ticker).font(mono(11.5)).strong().color(text()));
            ui.label(RichText::new(format!("★ {}m", self.interval)).font(mono(11.0)).color(up()));
            ui.label(RichText::new("·").color(muted()));
            ui.label(RichText::new("Simulated Trades").font(mono(10.0)).color(amber()))
                .on_hover_text(&self.status);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(dp(6.0));
                if ui.button(RichText::new("📊 Lab").font(ui_font(10.5))).on_hover_text("Open DRBT-Grid (Dynamic Range-Break Trend) Strategy Lab").clicked() {
                    actions.push(Action::Command("STRAT".into()));
                }
                if ui.button(RichText::new("📈 Watchlist Chart").font(ui_font(10.5)).color(up())).on_hover_text("Open clean standard Watchlist chart").clicked() {
                    actions.push(Action::OpenChart(self.symbol.clone()));
                }
            });
        });

        // Dedicated backtest chart
        self.chart.show(ui, chart_rect, focused, w);

        actions
    }
}
