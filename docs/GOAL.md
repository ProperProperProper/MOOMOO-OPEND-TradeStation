# Goal: MOOMOO-OPEND-TradeStation

Build a fast, keyboard-driven market terminal in pure Rust: GPU charting with a Bloomberg Terminal look (black background, monospace numbers, dense tiled panels, a command line like `AAPL <GO>`), **dedicated exclusively to Moomoo account data via local OpenD**.

## Security Hardening Mandates
1. **Moomoo OpenD Only**: The application connects strictly to the local OpenD gateway (`127.0.0.1:11111`).
2. **Watchlist-Only Feeds**: Market data, history, quotes, and streaming are strictly confined to instruments in the user's Moomoo account watchlist.
3. **Loopback-Only Networking**: External non-loopback connections are blocked by the security firewall.
4. **No Third-Party Logins**: Zero TradingView or broker login handlers.
5. **No Email / Webhooks**: Zero email dispatching and zero webhooks. Notifications are 100% desktop/in-app toasts.
6. **No SSH / FTP**: Remote management and file transfer protocols are strictly forbidden.
7. **No Auto-Updater / Telemetry**: Auto-updater (Sparkle) and update commands are completely removed.
8. **No Secrets in Repo**: Authentication is handled locally by OpenD; zero credentials committed.

## Architecture
- **Pure Rust Stack**: Native wgpu GPU rendering, egui immediate-mode UI, tokio async engine, rusqlite local storage.
- **Dedicated Crate**: `crates/moomoo` implements `OpenDClient` with built-in loopback and watchlist verification.
- **Market Data Router**: `crates/data` feeds all quotes and history directly from `tr-moomoo`.
- **Strategy & AI Engine**: `crates/strategy` provides quantitative optimization, multi-core cooling governor, grid search, and paper execution.
- **In-App Notifications**: On-screen animated toasts deliver private notifications.

## Completed Milestones
- [x] Command line, docking workspace, saved layouts, layout deletion, rebindable hotkeys.
- [x] wgpu GPU charts: candles/bars/line/area, 1m–1M, drawings, 151 indicators.
- [x] Moomoo OpenD integration (`crates/moomoo`).
- [x] Loopback firewall with SSH/FTP/SMTP/webhook rejection.
- [x] Watchlist-only data feed enforcement.
- [x] Complete removal of auto-updater and telemetry.
- [x] Pre-push security verification and GitHub repository rename to `MOOMOO-OPEND-TradeStation`.
- [x] AI Quantitative Bot & Strategy Lab pane (`PaneKind::Strategy`, code `AI`).
- [x] Dedicated Backtest Runner chart (`PaneKind::Backtest`, code `BT`) separated completely from standard watchlist chart (`GP`).
- [x] Real & paper executed order fills rendered on standard watchlist chart (`B` green / `S` red badges, hover cards, price level ticks).
- [x] Apple Silicon multi-core thermal management distributing workload across all E-cores with background QoS.
- [x] Full state persistence for saved layouts, workspace dock states, and demo accounts.
