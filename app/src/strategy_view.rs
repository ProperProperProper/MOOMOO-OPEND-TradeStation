//! Quantitative trading bot, strategy research, interval sweeps and AI-assisted exit management.

use std::path::PathBuf;
use egui::{Color32, RichText, Stroke, Ui};
use tr_moomoo::OpenDClient;
use tr_strategy::{
    aggregate_bars, evaluate_promotion_gate, generate_candidates, run_backtest, run_interval_sweep,
    BacktestResult, EntryMode, ExitMode, PaperTradingBot, PromotionGate, StrategyParams, SymbolPaperConfig,
};

use crate::congress_view::Action;
use crate::theme::{amber, down, dp, mono, muted, text, ui_font, up};

#[derive(Debug, Clone)]
pub struct StrategyPaneState {
    pub symbol: String,
    pub bot: PaperTradingBot,
    pub active_params: StrategyParams,
    pub active_interval: usize,
    pub sweep_results: Vec<BacktestResult>,
    pub ai_proposals: Vec<StrategyParams>,
    pub promotion_status: Option<PromotionGate>,
    pub available_symbols: Vec<String>,
    pub governor: tr_strategy::GovernorState,
    pub status_message: String,
}

impl StrategyPaneState {
    pub fn new(symbol: &str) -> Self {
        let root = PathBuf::from(".");
        let client = OpenDClient::default();
        let mut bot = PaperTradingBot::new(client, root.clone());
        let _ = bot.reload_config();
        bot.running = true;

        let active_symbol = if !symbol.trim().is_empty() {
            symbol.trim().to_uppercase()
        } else {
            bot.active_symbol.clone()
        };

        let available = tr_strategy::storage::list_configured_symbols(&root);
        let available_symbols = if available.is_empty() {
            vec![
                "US.MRNA".into(),
                "US.ANT".into(),
                "US.NVDA".into(),
                "US.MU".into(),
                "US.SNDK".into(),
                "US.OMH".into(),
                "US.APP".into(),
                "US.AAPL".into(),
            ]
        } else {
            available
        };

        let strat = tr_strategy::load_symbol_strategy(&root, &active_symbol).unwrap_or_else(|_| SymbolPaperConfig {
            symbol: active_symbol.clone(),
            interval: 10,
            mode: "SIMULATE".into(),
            params: StrategyParams::default(),
            run_id: "DEFAULT".into(),
            updated_at: String::new(),
        });

        // Initialize Darwin background QoS and cooling governor
        let governor = tr_strategy::get_governor_state();

        // Run baseline walk-forward sweep across 1m..10m so testing stats appear immediately
        let bars_1m = load_real_history(&active_symbol, 3000);
        let intervals: Vec<usize> = (1..=10).collect();
        let sweep_results = run_interval_sweep(&active_symbol, &bars_1m, &intervals, &strat.params, 100_000.0);

        let promotion_status = sweep_results.first().map(|winner| {
            let baseline_res = run_backtest(
                &active_symbol,
                strat.interval,
                &aggregate_bars(&bars_1m, strat.interval),
                &strat.params,
                100_000.0,
                0.95,
                2.0,
                0.0,
            );
            evaluate_promotion_gate(winner, &baseline_res, 5)
        });

        let ai_proposals = generate_candidates(4, &strat.params);

        Self {
            symbol: active_symbol.clone(),
            bot,
            active_params: strat.params,
            active_interval: strat.interval,
            sweep_results,
            ai_proposals,
            promotion_status,
            available_symbols,
            governor,
            status_message: format!("AI Strategy Lab Active · 1m..10m sweeps evaluated for {active_symbol}"),
        }
    }
}

pub fn strategy_panel(ui: &mut Ui, state: &mut StrategyPaneState) -> Vec<Action> {
    let mut actions = Vec::new();

    egui::ScrollArea::vertical().show(ui, |ui| {
        // --- 1. Header & Active Bot Status ---
        ui.horizontal(|ui| {
            ui.label(RichText::new("ANT-TRADER BOT & STRATEGY LAB").font(ui_font(13.0)).strong().color(amber()));
            ui.add_space(dp(12.0));

            let (badge_text, badge_color) = if state.bot.running {
                ("● LIVE PAPER RUNNING (SIMULATE)", up())
            } else {
                ("○ BOT STOPPED", muted())
            };
            ui.label(RichText::new(badge_text).font(mono(11.0)).color(badge_color));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if state.bot.running {
                    if ui.button(RichText::new("⏹ STOP BOT").color(down())).clicked() {
                        state.bot.stop();
                        state.status_message = "Bot stopped".to_string();
                    }
                } else {
                    if ui.button(RichText::new("▶ START PAPER BOT").color(up())).clicked() {
                        state.bot.start();
                        state.status_message = format!("Bot started on {}", state.symbol);
                    }
                }
            });
        });

        ui.add_space(dp(4.0));

        // --- 1b. Apple Silicon Hardware Governor & Cooling Status ---
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🍎 APPLE SILICON GOVERNOR:").font(mono(10.5)).strong().color(amber()));
                ui.label(RichText::new(format!("{} ({} E-Cores · {} P-Cores)", state.governor.topology.chip_name, state.governor.topology.e_cores, state.governor.topology.p_cores)).font(mono(10.5)).color(text()));
                ui.label(RichText::new("·").color(muted()));
                ui.label(RichText::new("Affinity: E-Cores (Darwin Background QoS)").font(mono(10.5)).color(up()));
                ui.label(RichText::new("·").color(muted()));
                let therm_color = match state.governor.thermal_status {
                    tr_strategy::ThermalStatus::Nominal => up(),
                    tr_strategy::ThermalStatus::Warm => amber(),
                    _ => down(),
                };
                ui.label(RichText::new(state.governor.thermal_status.as_str()).font(mono(10.5)).color(therm_color));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("🔄 Recheck Thermal").font(mono(10.0))).clicked() {
                        state.governor = tr_strategy::get_governor_state();
                    }
                });
            });
        });

        ui.add_space(dp(6.0));
        ui.separator();
        ui.add_space(dp(6.0));

        // --- 2. Portfolio Symbol Selector & Live Metrics ---
        ui.horizontal(|ui| {
            ui.label(RichText::new("Active Symbol:").font(ui_font(11.0)).color(muted()));
            for sym in &state.available_symbols {
                let is_selected = sym.eq_ignore_ascii_case(&state.symbol);
                let btn = egui::Button::new(RichText::new(sym).font(mono(11.0)))
                    .fill(if is_selected { Color32::from_rgb(30, 60, 90) } else { Color32::TRANSPARENT })
                    .stroke(Stroke::new(1.0, if is_selected { amber() } else { Color32::from_gray(60) }));

                if ui.add(btn).clicked() {
                    if state.bot.position_qty > 0.0 {
                        state.status_message = "⚠️ Rejected: Cannot switch symbol while holding open position".into();
                    } else {
                        state.symbol = sym.clone();
                        let _ = state.bot.switch_symbol(sym);
                        if let Ok(strat) = tr_strategy::load_symbol_strategy(&PathBuf::from("."), sym) {
                            state.active_params = strat.params;
                            state.active_interval = strat.interval;
                        }
                        let bars_1m = load_real_history(sym, 3000);
                        let intervals: Vec<usize> = (1..=10).collect();
                        let results = run_interval_sweep(sym, &bars_1m, &intervals, &state.active_params, 100_000.0);
                        if let Some(winner) = results.first() {
                            let baseline_res = run_backtest(
                                sym,
                                state.active_interval,
                                &aggregate_bars(&bars_1m, state.active_interval),
                                &state.active_params,
                                100_000.0,
                                0.95,
                                2.0,
                                0.0,
                            );
                            state.promotion_status = Some(evaluate_promotion_gate(winner, &baseline_res, 5));
                        }
                        state.sweep_results = results;
                        actions.push(Action::OpenChart(sym.clone()));
                    }
                }
            }
        });

        ui.add_space(dp(8.0));

        // Real-time execution stats card
        ui.horizontal(|ui| {
            metric_card(ui, "Paper Cash", &format!("${:.2}", state.bot.paper_balance), amber());
            let pos_str = if state.bot.position_qty > 0.0 {
                format!("{:.0} shs @ ${:.2}", state.bot.position_qty, state.bot.entry_price)
            } else {
                "Flat".into()
            };
            metric_card(ui, "Position", &pos_str, if state.bot.position_qty > 0.0 { up() } else { muted() });
            metric_card(ui, "Peak Price", &format!("${:.2}", state.bot.peak_price), text());
            let trailing_str = if state.bot.trailing_armed { "ARMED ⚡" } else { "Off" };
            metric_card(ui, "Trailing Stop", trailing_str, if state.bot.trailing_armed { up() } else { muted() });
            metric_card(ui, "Active Interval", &format!("{}m", state.active_interval), text());
        });

        if !state.status_message.is_empty() {
            ui.add_space(dp(4.0));
            ui.label(RichText::new(&state.status_message).font(mono(10.5)).color(amber()));
        }

        ui.add_space(dp(8.0));
        ui.separator();
        ui.add_space(dp(8.0));

        // --- 3. Active Strategy Parameters & Controls ---
        ui.label(RichText::new("STRATEGY PARAMETERS & RISK EXITS").font(ui_font(12.0)).strong());
        ui.add_space(dp(4.0));

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Grid Lookback:");
                    ui.add(egui::DragValue::new(&mut state.active_params.grid_lookback).range(5..=240));
                    ui.label("candles");
                });
                ui.horizontal(|ui| {
                    ui.label("Grid Levels:");
                    ui.add(egui::DragValue::new(&mut state.active_params.grid_intervals).range(2..=20));
                    ui.label("intervals");
                });
                ui.horizontal(|ui| {
                    ui.label("Entry Mode:");
                    egui::ComboBox::from_id_salt("entry_mode_cb")
                        .selected_text(format!("{:?}", state.active_params.entry_mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut state.active_params.entry_mode, EntryMode::AnyGridCross, "AnyGridCross");
                            ui.selectable_value(&mut state.active_params.entry_mode, EntryMode::TopBreakout, "TopBreakout");
                            ui.selectable_value(&mut state.active_params.entry_mode, EntryMode::UpperHalfCross, "UpperHalfCross");
                            ui.selectable_value(&mut state.active_params.entry_mode, EntryMode::LowerHalfCross, "LowerHalfCross");
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Exit Mode:");
                    egui::ComboBox::from_id_salt("exit_mode_cb")
                        .selected_text(format!("{:?}", state.active_params.exit_mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut state.active_params.exit_mode, ExitMode::AnyGridCrossDown, "AnyGridCrossDown");
                            ui.selectable_value(&mut state.active_params.exit_mode, ExitMode::TopBreakdown, "TopBreakdown");
                            ui.selectable_value(&mut state.active_params.exit_mode, ExitMode::BottomBreakdown, "BottomBreakdown");
                            ui.selectable_value(&mut state.active_params.exit_mode, ExitMode::LowerHalfBreakdown, "LowerHalfBreakdown");
                        });
                });
            });

            ui.add_space(dp(20.0));

            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Open Delay:");
                    ui.add(egui::DragValue::new(&mut state.active_params.open_delay_minutes).range(0..=60));
                    ui.label("minutes");
                });
                ui.horizontal(|ui| {
                    ui.label("Take Profit %:");
                    let mut tp = state.active_params.take_profit_pct * 100.0;
                    if ui.add(egui::DragValue::new(&mut tp).range(0.1..=20.0).speed(0.1)).changed() {
                        state.active_params.take_profit_pct = tp / 100.0;
                    }
                    ui.label("%");
                });
                ui.horizontal(|ui| {
                    ui.label("Trailing Arm %:");
                    let mut ta = state.active_params.trailing_activation_pct * 100.0;
                    if ui.add(egui::DragValue::new(&mut ta).range(0.0..=10.0).speed(0.1)).changed() {
                        state.active_params.trailing_activation_pct = ta / 100.0;
                    }
                    ui.label("%");
                });
                ui.horizontal(|ui| {
                    ui.label("Trailing Stop %:");
                    let mut ts = state.active_params.trailing_stop_pct * 100.0;
                    if ui.add(egui::DragValue::new(&mut ts).range(0.0..=10.0).speed(0.1)).changed() {
                        state.active_params.trailing_stop_pct = ts / 100.0;
                    }
                    ui.label("%");
                });
            });

            ui.add_space(dp(20.0));

            ui.vertical(|ui| {
                if ui.button("💾 Save Strategy to Paper").clicked() {
                    let config = SymbolPaperConfig {
                        symbol: state.symbol.clone(),
                        interval: state.active_interval,
                        mode: "SIMULATE".into(),
                        params: state.active_params.clone(),
                        run_id: format!("{}-MANUAL", state.symbol.replace('.', "-")),
                        updated_at: chrono::Utc::now().to_rfc3339(),
                    };
                    if let Ok(()) = tr_strategy::save_symbol_strategy(&PathBuf::from("."), &config) {
                        state.status_message = format!("Strategy saved for {}", state.symbol);
                    }
                }
            });
        });

        ui.add_space(dp(10.0));
        ui.separator();
        ui.add_space(dp(8.0));

        // --- 4. Backtesting & 1m–10m Research Sweeps ---
        ui.horizontal(|ui| {
            ui.label(RichText::new("RESEARCH ENGINE & WALK-FORWARD SWEEPS").font(ui_font(12.0)).strong());
            ui.add_space(dp(12.0));

            if ui.button(RichText::new("⚡ RUN 1m–10m INTERVAL SWEEP").color(amber())).clicked() {
                // Load real 1m candles from persistent SQLite cache for sweep
                let bars_1m = load_real_history(&state.symbol, 5000);
                let intervals: Vec<usize> = (1..=10).collect();
                let results = run_interval_sweep(&state.symbol, &bars_1m, &intervals, &state.active_params, 100_000.0);

                if let Some(winner) = results.first() {
                    let baseline_res = run_backtest(
                        &state.symbol,
                        state.active_interval,
                        &aggregate_bars(&bars_1m, state.active_interval),
                        &state.active_params,
                        100_000.0,
                        0.95,
                        2.0,
                        0.0,
                    );
                    state.promotion_status = Some(evaluate_promotion_gate(winner, &baseline_res, 5));
                }
                state.sweep_results = results;
                state.status_message = format!("Sweep complete: tested 1m..10m intervals for {}", state.symbol);
            }

            if ui.button(RichText::new("🤖 AI PARAMETER PROPOSALS").color(up())).clicked() {
                let candidates = generate_candidates(4, &state.active_params);
                state.ai_proposals = candidates;
                state.status_message = format!("Generated {} AI-assisted candidate parameter sets", state.ai_proposals.len());
            }
        });

        // Promotion Gate Card
        if let Some(gate) = &state.promotion_status {
            ui.add_space(dp(6.0));
            let color = if gate.allowed { up() } else { down() };
            ui.horizontal(|ui| {
                ui.label(RichText::new("Evidence Gate:").font(mono(11.0)).color(color));
                ui.label(RichText::new(&gate.reason).font(mono(11.0)).color(text()));
                if gate.allowed {
                    if ui.button(RichText::new("🚀 PROMOTE WINNER TO PAPER").color(up())).clicked() {
                        if state.bot.position_qty > 0.0 {
                            state.status_message = "⚠️ Rejected: Cannot promote strategy while holding open position".into();
                        } else if let Some(winner) = state.sweep_results.first() {
                            let _ = state.bot.promote_winner(&state.symbol, winner.interval, winner.params.clone());
                            state.active_interval = winner.interval;
                            state.active_params = winner.params.clone();
                            state.status_message = format!("Promoted {}m winner into paper strategy!", winner.interval);
                        }
                    }
                }
            });
        }

        // Interval sweep results table
        if !state.sweep_results.is_empty() {
            ui.add_space(dp(6.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new("Interval").strong().color(muted()));
                ui.add_space(dp(20.0));
                ui.label(RichText::new("Return %").strong().color(muted()));
                ui.add_space(dp(20.0));
                ui.label(RichText::new("Win Rate").strong().color(muted()));
                ui.add_space(dp(20.0));
                ui.label(RichText::new("Profit Factor").strong().color(muted()));
                ui.add_space(dp(20.0));
                ui.label(RichText::new("Max DD %").strong().color(muted()));
                ui.add_space(dp(20.0));
                ui.label(RichText::new("Trades").strong().color(muted()));
                ui.add_space(dp(20.0));
                ui.label(RichText::new("Sharpe").strong().color(muted()));
            });

            for (idx, r) in state.sweep_results.iter().enumerate() {
                let is_winner = idx == 0;
                let text_color = if is_winner { amber() } else { text() };
                ui.horizontal(|ui| {
                    let star = if is_winner { "★ " } else { "  " };
                    ui.label(RichText::new(format!("{star}{}m", r.interval)).font(mono(11.0)).color(text_color));
                    ui.add_space(dp(24.0));
                    let ret_color = if r.total_return_pct >= 0.0 { up() } else { down() };
                    ui.label(RichText::new(format!("{:+.2}%", r.total_return_pct)).font(mono(11.0)).color(ret_color));
                    ui.add_space(dp(20.0));
                    ui.label(RichText::new(format!("{:.1}%", r.win_rate)).font(mono(11.0)).color(text()));
                    ui.add_space(dp(28.0));
                    ui.label(RichText::new(format!("{:.2}", r.profit_factor)).font(mono(11.0)).color(text()));
                    ui.add_space(dp(36.0));
                    ui.label(RichText::new(format!("{:.2}%", r.max_drawdown)).font(mono(11.0)).color(down()));
                    ui.add_space(dp(32.0));
                    ui.label(RichText::new(format!("{}", r.total_trades)).font(mono(11.0)).color(text()));
                    ui.add_space(dp(28.0));
                    ui.label(RichText::new(format!("{:.2}", r.sharpe)).font(mono(11.0)).color(text()));

                    if ui.button("Apply").clicked() {
                        state.active_interval = r.interval;
                        state.active_params = r.params.clone();
                        state.status_message = format!("Selected {}m params", r.interval);
                    }
                    let tf = match r.interval {
                        1 => tr_core::Timeframe::M1,
                        2 => tr_core::Timeframe::M2,
                        3 => tr_core::Timeframe::M3,
                        4 => tr_core::Timeframe::M4,
                        5 => tr_core::Timeframe::M5,
                        6 => tr_core::Timeframe::M6,
                        7 => tr_core::Timeframe::M7,
                        8 => tr_core::Timeframe::M8,
                        _ => tr_core::Timeframe::M9,
                    };
                    if ui.button(RichText::new("Chart").color(up())).on_hover_text("View on chart with Buy/Sell markers").clicked() {
                        actions.push(Action::ShowStratChart(state.symbol.clone(), tf));
                    }
                });
            }
        }

        // AI proposals section
        if !state.ai_proposals.is_empty() {
            ui.add_space(dp(8.0));
            ui.label(RichText::new("AI-PROPOSED CANDIDATE PARAMETERS:").font(ui_font(11.0)).color(up()));
            for (i, p) in state.ai_proposals.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!(
                        "Proposal #{}: Lookback={}, Levels={}, Entry={:?}, Exit={:?}, TP={:.2}%, TrailArm={:.1}%",
                        i + 1, p.grid_lookback, p.grid_intervals, p.entry_mode, p.exit_mode, p.take_profit_pct * 100.0, p.trailing_activation_pct * 100.0
                    )).font(mono(10.5)));
                    if ui.button("Adopt").clicked() {
                        state.active_params = p.clone();
                        state.status_message = format!("Adopted AI Proposal #{}", i + 1);
                    }
                });
            }
        }

        // --- 5. Paper Order Execution & Audit History ---
        ui.add_space(dp(10.0));
        ui.separator();
        ui.add_space(dp(8.0));
        ui.label(RichText::new("PAPER TRADE EXECUTION LOG").font(ui_font(12.0)).strong());
        ui.add_space(dp(4.0));

        if state.bot.trade_history.is_empty() {
            ui.label(RichText::new("No executions yet. Start bot or run sweep to generate simulated trades.").color(muted()));
        } else {
            for trade in state.bot.trade_history.iter().rev().take(10) {
                ui.horizontal(|ui| {
                    let ret_color = if trade.net_pnl >= 0.0 { up() } else { down() };
                    ui.label(RichText::new(&trade.exit_time).font(mono(10.5)).color(muted()));
                    ui.label(RichText::new(&trade.reason).font(mono(10.5)).color(amber()));
                    ui.label(RichText::new(format!("Qty: {:.0} @ ${:.2} -> ${:.2}", trade.qty, trade.entry_price, trade.exit_price)).font(mono(10.5)));
                    ui.label(RichText::new(format!("PnL: ${:+.2} ({:+.2}%)", trade.net_pnl, trade.return_pct)).font(mono(10.5)).color(ret_color));
                });
            }
        }
    });

    actions
}

fn metric_card(ui: &mut Ui, label: &str, value: &str, val_color: Color32) {
    ui.group(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(label).font(ui_font(10.0)).color(muted()));
            ui.label(RichText::new(value).font(mono(12.0)).strong().color(val_color));
        });
    });
}

fn load_real_history(symbol: &str, count: usize) -> Vec<tr_moomoo::Bar> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let p = home.join("Library/Application Support/terminal/bars.sqlite3");
    let sec = tr_data::security(symbol);
    if let Ok(c) = tr_data::cache::BarCache::open(&p) {
        let bars = c.load(&sec.ticker, tr_core::Timeframe::M1, count)
            .or_else(|_| c.load(symbol, tr_core::Timeframe::M1, count))
            .unwrap_or_default();
        if !bars.is_empty() {
            return bars.into_iter().map(|b| tr_moomoo::Bar {
                ts: b.ts,
                open: b.open,
                high: b.high,
                low: b.low,
                close: b.close,
                volume: b.volume,
            }).collect();
        }
    }
    generate_mock_history(symbol, count)
}

fn generate_mock_history(symbol: &str, count: usize) -> Vec<tr_moomoo::Bar> {
    let now = chrono::Utc::now().timestamp();
    let mut bars = Vec::with_capacity(count);
    let mut price = match symbol {
        "US.NVDA" => 135.0,
        "US.MRNA" => 42.0,
        "US.ANT" => 18.0,
        "US.MU" => 105.0,
        _ => 100.0,
    };
    for i in (0..count).rev() {
        let ts = now - (i as i64) * 60;
        let change = if (i % 7) < 4 { 0.25 } else { -0.22 };
        price += change;
        bars.push(tr_moomoo::Bar {
            ts,
            open: price - 0.1,
            high: price + 0.35,
            low: price - 0.35,
            close: price + 0.08,
            volume: 12000.0,
        });
    }
    bars
}
