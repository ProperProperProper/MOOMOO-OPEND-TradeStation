//! Paper trading session equity curve view.
//! Displays session equity progression, cumulative P&L, trade impacts and metrics.

use std::collections::HashMap;

use chrono::{NaiveDate, Utc};
use egui::{Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use tr_broker::demo::DemoBook;
use tr_broker::model::Order;

use crate::congress_view::Action;
use crate::theme::{self, mono, muted, text, ui_font};

#[derive(Debug, Clone)]
pub struct EquityPoint {
    #[allow(dead_code)]
    pub ts: i64,
    pub time_str: String,
    pub equity: f64,
    pub cash: f64,
    pub pos_val: f64,
    pub trade_desc: Option<String>,
    #[allow(dead_code)]
    pub trade_sym: Option<String>,
}

pub struct SessionData {
    pub date_str: String,
    pub is_open: bool,
    pub starting_cash: f64,
    pub current_equity: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub peak_equity: f64,
    pub trough_equity: f64,
    pub total_trades: usize,
    pub total_volume: f64,
    pub points: Vec<EquityPoint>,
    pub trades: Vec<SessionTrade>,
}

#[derive(Debug, Clone)]
pub struct SessionTrade {
    pub time_str: String,
    pub symbol: String,
    pub action: String,
    pub qty: f64,
    pub price: f64,
    pub value: f64,
    pub equity: f64,
    pub pnl_at_time: f64,
}

pub fn calculate_session_equity(book: &DemoBook) -> SessionData {
    let now = Utc::now();
    let today = now.date_naive();

    // 1. Gather all filled orders with valid timestamps
    let mut valid_orders: Vec<&Order> = book
        .orders
        .iter()
        .filter(|o| o.placed.is_some() || o.updated.is_some())
        .collect();

    // Sort chronologically ascending
    valid_orders.sort_by_key(|o| o.placed.or(o.updated).unwrap_or(now));

    // 2. Identify the active session date:
    // If today has orders, use today; otherwise pick the most recent date with orders.
    let target_date: Option<NaiveDate> = if valid_orders.iter().any(|o| {
        o.placed.or(o.updated).map(|t| t.date_naive()) == Some(today)
    }) {
        Some(today)
    } else {
        valid_orders
            .last()
            .and_then(|o| o.placed.or(o.updated).map(|t| t.date_naive()))
    };

    let Some(session_date) = target_date else {
        // No orders recorded yet: return a flat baseline at current book cash
        let cash = book.cash;
        return SessionData {
            date_str: today.format("%d %b %Y").to_string(),
            is_open: true,
            starting_cash: cash,
            current_equity: cash,
            pnl: 0.0,
            pnl_pct: 0.0,
            peak_equity: cash,
            trough_equity: cash,
            total_trades: 0,
            total_volume: 0.0,
            points: vec![
                EquityPoint {
                    ts: now.timestamp() - 3600,
                    time_str: "09:30".into(),
                    equity: cash,
                    cash,
                    pos_val: 0.0,
                    trade_desc: None,
                    trade_sym: None,
                },
                EquityPoint {
                    ts: now.timestamp(),
                    time_str: now.format("%H:%M").to_string(),
                    equity: cash,
                    cash,
                    pos_val: 0.0,
                    trade_desc: None,
                    trade_sym: None,
                },
            ],
            trades: Vec::new(),
        };
    };

    let session_orders: Vec<&Order> = valid_orders
        .into_iter()
        .filter(|o| o.placed.or(o.updated).map(|t| t.date_naive()) == Some(session_date))
        .collect();

    // 3. Compute net cash change over this session to establish starting cash
    let mut net_cash_delta = 0.0;
    let mut total_volume = 0.0;
    for o in &session_orders {
        let qty = o.filled.unwrap_or(o.quantity.unwrap_or(0.0));
        let px = o.execution_price.unwrap_or(o.limit_price.unwrap_or(0.0));
        let val = qty * px;
        total_volume += val;
        if o.is_buy() {
            net_cash_delta -= val;
        } else {
            net_cash_delta += val;
        }
    }

    let starting_cash = (book.cash - net_cash_delta).max(0.0);
    let mut sim_cash = starting_cash;
    let mut holdings: HashMap<String, (f64, f64)> = HashMap::new(); // sym -> (shares, last_px)
    let mut points = Vec::new();
    let mut session_trades = Vec::new();

    // Add opening point at start of day
    if let Some(first) = session_orders.first() {
        let first_ts = first.placed.or(first.updated).unwrap_or(now).timestamp();
        points.push(EquityPoint {
            ts: first_ts.saturating_sub(60),
            time_str: "Open".into(),
            equity: starting_cash,
            cash: starting_cash,
            pos_val: 0.0,
            trade_desc: Some("Session Start".into()),
            trade_sym: None,
        });
    }

    let mut peak = starting_cash;
    let mut trough = starting_cash;

    for o in &session_orders {
        let dt = o.updated.or(o.placed).unwrap_or(now);
        let ts = dt.timestamp();
        let time_str = dt.format("%H:%M:%S").to_string();
        let sym = o.symbol.clone();
        let qty = o.filled.unwrap_or(o.quantity.unwrap_or(0.0));
        let px = o.execution_price.unwrap_or(o.limit_price.unwrap_or(0.0));
        let val = qty * px;
        let is_buy = o.is_buy();

        let (mut shares, _) = holdings.get(&sym).copied().unwrap_or((0.0, px));
        if is_buy {
            sim_cash -= val;
            shares += qty;
        } else {
            sim_cash += val;
            shares -= qty;
        }
        holdings.insert(sym.clone(), (shares, px));

        let pos_val: f64 = holdings.values().map(|(s, p)| s * p).sum();
        let eq = sim_cash + pos_val;
        peak = peak.max(eq);
        trough = trough.min(eq);

        let trade_desc = format!(
            "{} {} shares of {} @ ${:.2}",
            if is_buy { "BUY" } else { "SELL" },
            qty,
            sym,
            px
        );

        let pnl_at_time = eq - starting_cash;

        points.push(EquityPoint {
            ts,
            time_str: time_str.clone(),
            equity: eq,
            cash: sim_cash,
            pos_val,
            trade_desc: Some(trade_desc),
            trade_sym: Some(sym.clone()),
        });

        session_trades.push(SessionTrade {
            time_str,
            symbol: sym,
            action: if is_buy { "BUY".into() } else { "SELL".into() },
            qty,
            price: px,
            value: val,
            equity: eq,
            pnl_at_time,
        });
    }

    let current_equity = points.last().map(|p| p.equity).unwrap_or(book.cash);
    let pnl = current_equity - starting_cash;
    let pnl_pct = if starting_cash > 0.0 { (pnl / starting_cash) * 100.0 } else { 0.0 };
    let is_open = session_date == today;

    SessionData {
        date_str: session_date.format("%d %b %Y").to_string(),
        is_open,
        starting_cash,
        current_equity,
        pnl,
        pnl_pct,
        peak_equity: peak,
        trough_equity: trough,
        total_trades: session_trades.len(),
        total_volume,
        points,
        trades: session_trades,
    }
}

/// Renders the equity curve pane for the paper account session.
pub fn equity_curve(ui: &mut Ui, book: &DemoBook) -> Vec<Action> {
    let mut out = Vec::new();
    let data = calculate_session_equity(book);

    let is_positive = data.pnl >= 0.0;
    let line_col = if is_positive { theme::up() } else { theme::down() };

    // 1. Header Banner
    ui.horizontal(|ui| {
        let badge_text = if data.is_open {
            "● LIVE PAPER SESSION"
        } else {
            "● SESSION CLOSED"
        };
        let badge_col = if data.is_open { theme::up() } else { muted() };
        ui.label(RichText::new(badge_text).font(mono(11.0)).color(badge_col));
        ui.label(RichText::new(format!("({})", data.date_str)).font(mono(11.0)).color(muted()));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!(
                    "P&L: {}${:.2} ({:+.2}%)",
                    if data.pnl >= 0.0 { "+" } else { "-" },
                    data.pnl.abs(),
                    data.pnl_pct
                ))
                .font(mono(14.0))
                .color(line_col),
            );
            ui.label(
                RichText::new(format!("${:.2}", data.current_equity))
                    .font(mono(16.0))
                    .color(text()),
            );
            ui.label(RichText::new("Equity:").font(mono(12.0)).color(muted()));
        });
    });

    ui.add_space(4.0);

    // 2. Metrics Bar
    ui.horizontal(|ui| {
        let chip = |ui: &mut Ui, label: &str, val: String| {
            ui.label(RichText::new(label).font(ui_font(11.0)).color(muted()));
            ui.label(RichText::new(val).font(mono(11.5)).color(text()));
            ui.add_space(8.0);
        };

        chip(ui, "Starting:", format!("${:.2}", data.starting_cash));
        chip(ui, "Peak:", format!("${:.2}", data.peak_equity));
        chip(ui, "Trough:", format!("${:.2}", data.trough_equity));
        chip(ui, "Trades:", data.total_trades.to_string());
        chip(ui, "Turnover:", format!("${:.2}", data.total_volume));
    });

    ui.add_space(6.0);

    // 3. Equity Curve Plot Area
    let avail = ui.available_size();
    let plot_h = (avail.y * 0.52).clamp(120.0, 360.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(avail.x, plot_h), Sense::hover());
    let p = ui.painter_at(rect);

    // Background & border
    p.rect_filled(rect, 4.0, theme::card());
    p.rect_stroke(rect, 4.0, Stroke::new(1.0, theme::border()), egui::StrokeKind::Inside);

    let plot = Rect::from_min_max(rect.min + vec2(12.0, 12.0), rect.max - vec2(68.0, 24.0));
    let n = data.points.len();

    if n >= 2 {
        let (lo_val, hi_val) = data
            .points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), pt| {
                (lo.min(pt.equity), hi.max(pt.equity))
            });

        let pad = ((hi_val - lo_val) * 0.15).max(10.0);
        let lo = lo_val - pad;
        let hi = hi_val + pad;

        let x_of = |i: usize| plot.left() + plot.width() * i as f32 / (n - 1) as f32;
        let y_of = |v: f64| plot.bottom() - plot.height() * ((v - lo) / (hi - lo)) as f32;

        // Draw horizontal grid lines & price labels
        let grid_steps = 5;
        for i in 0..=grid_steps {
            let val = lo + (hi - lo) * i as f64 / grid_steps as f64;
            let y = y_of(val);
            p.hline(plot.x_range(), y, Stroke::new(0.5, theme::grid()));
            p.text(
                pos2(plot.right() + 6.0, y),
                Align2::LEFT_CENTER,
                format!("${:.0}", val),
                mono(10.0),
                muted(),
            );
        }

        // Draw time labels along x axis
        let time_steps = 4.min(n - 1);
        for s in 0..=time_steps {
            let idx = s * (n - 1) / time_steps;
            let x = x_of(idx);
            p.vline(x, plot.y_range(), Stroke::new(0.5, theme::grid()));
            let label = &data.points[idx].time_str;
            p.text(
                pos2(x, plot.bottom() + 8.0),
                Align2::CENTER_TOP,
                label,
                mono(10.0),
                muted(),
            );
        }

        // Generate plot points
        let pts: Vec<Pos2> = data
            .points
            .iter()
            .enumerate()
            .map(|(i, pt)| pos2(x_of(i), y_of(pt.equity)))
            .collect();

        // Fill area under curve
        let mut fill_pts = pts.clone();
        fill_pts.push(pos2(plot.right(), plot.bottom()));
        fill_pts.push(pos2(plot.left(), plot.bottom()));
        p.add(egui::Shape::convex_polygon(
            fill_pts,
            line_col.gamma_multiply(0.12),
            Stroke::NONE,
        ));

        // Draw main curve line
        p.add(egui::Shape::line(pts.clone(), Stroke::new(2.4, line_col)));

        // Hover crosshair and tooltip
        if let Some(mouse) = resp.hover_pos()
            && plot.contains(mouse) {
                // Find nearest point
                let frac = ((mouse.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
                let nearest_idx = (frac * (n - 1) as f32).round() as usize;
                if let Some(pt) = data.points.get(nearest_idx) {
                    let point_pos = pts[nearest_idx];

                    // Draw crosshair
                    p.vline(point_pos.x, plot.y_range(), Stroke::new(1.0, theme::cyan().gamma_multiply(0.5)));
                    p.hline(plot.x_range(), point_pos.y, Stroke::new(1.0, theme::cyan().gamma_multiply(0.5)));
                    p.circle_filled(point_pos, 4.0, line_col);
                    p.circle_stroke(point_pos, 6.0, Stroke::new(1.5, Color32::WHITE));

                    // Tooltip box
                    let pnl_now = pt.equity - data.starting_cash;
                    let trade_line = pt.trade_desc.as_deref().map(|d| format!("\nTrade: {d}")).unwrap_or_default();
                    let tip_text = format!(
                        "Time: {}\nEquity: ${:.2}\nCash: ${:.2}\nPositions: ${:.2}\nP&L: {}${:.2}{trade_line}",
                        pt.time_str,
                        pt.equity,
                        pt.cash,
                        pt.pos_val,
                        if pnl_now >= 0.0 { "+" } else { "-" },
                        pnl_now.abs()
                    );

                    let tip_pos = if point_pos.x > plot.center().x {
                        point_pos - vec2(150.0, 40.0)
                    } else {
                        point_pos + vec2(16.0, -40.0)
                    };

                    let tip_h = if pt.trade_desc.is_some() { 88.0 } else { 72.0 };
                    let tip_rect = Rect::from_min_size(tip_pos, vec2(150.0, tip_h));
                    p.rect_filled(tip_rect, 4.0, Color32::from_black_alpha(225));
                    p.rect_stroke(tip_rect, 4.0, Stroke::new(1.0, theme::border()), egui::StrokeKind::Inside);
                    p.text(
                        tip_rect.min + vec2(8.0, 8.0),
                        Align2::LEFT_TOP,
                        tip_text,
                        mono(10.5),
                        Color32::WHITE,
                    );
                }
            }
    } else {
        p.text(
            plot.center(),
            Align2::CENTER_CENTER,
            "No trading activity recorded for this session yet.",
            mono(12.0),
            muted(),
        );
    }

    ui.add_space(8.0);

    // 4. Session Executions Table
    ui.label(
        RichText::new(" TIME       SIDE  SYMBOL    SHARES    PRICE         VALUE        EQUITY     SESSION P&L")
            .font(mono(11.0))
            .color(theme::cyan())
            .background_color(theme::header()),
    );

    egui::ScrollArea::vertical()
        .id_salt("session-trades-table")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for t in data.trades.iter().rev() {
                let side_col = if t.action == "BUY" { theme::up() } else { theme::down() };
                let pnl_col = if t.pnl_at_time >= 0.0 { theme::up() } else { theme::down() };

                let row_text = format!(
                    " {:<10} {:<4}  {:<8} {:>7.0}   ${:>8.2}   ${:>10.2}   ${:>10.2}   {:+>10.2}",
                    t.time_str,
                    t.action,
                    t.symbol,
                    t.qty,
                    t.price,
                    t.value,
                    t.equity,
                    t.pnl_at_time
                );

                let resp = ui.add(
                    egui::Label::new(
                        RichText::new(row_text)
                            .font(mono(11.5))
                            .color(if t.pnl_at_time >= 0.0 { text() } else { pnl_col }),
                    )
                    .extend()
                    .sense(Sense::click()),
                );

                if resp.clicked() {
                    out.push(Action::OpenTicker(t.symbol.clone()));
                }

                resp.on_hover_ui(|ui| {
                    ui.label(RichText::new(format!("Click to open {} chart", t.symbol)).font(mono(11.0)).color(side_col));
                });
            }
        });

    out
}
