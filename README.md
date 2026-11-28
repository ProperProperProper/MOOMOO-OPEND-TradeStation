# MOOMOO-OPEND-TradeStation

A fast, keyboard-driven market terminal in pure Rust: GPU charting with a Bloomberg Terminal aesthetic (dense tiled panes, monospace numbers, a command line like `AAPL <GO>`), **dedicated exclusively to Moomoo data via the local OpenD gateway** for your Moomoo account watchlist. Features native GPU rendering, an AI Quantitative Bot & Strategy Lab, dedicated Backtest Runner panes, multi-core Apple Silicon thermal management, custom themes, and a fully dockable workspace with persistent saved layouts.

<img width="1463" height="882" alt="Screenshot 2026-11-29 at 1 09 36 AM" src="https://github.com/user-attachments/assets/b2df8e69-f457-4c16-9851-8688b8a4c4e6" />


> **Security & Privacy First Architecture:**
> All external internet data feeds (Yahoo Finance, Alpaca, Binance, Coinbase, US Treasury scrapers, external RSS feeds, and congressional web scrapers) have been replaced or disabled. The application communicates **strictly and exclusively with your local Moomoo OpenD gateway (`127.0.0.1:11111`)**.

---

## Strict Security & Isolation Guarantees

* **Moomoo OpenD Only**: Connects strictly to your local OpenD gateway (`127.0.0.1:11111`).
* **Watchlist-Restricted Data Feeds**: Quotes, historical candles, searches, and tick streaming are **strictly restricted to instruments in your Moomoo account watchlist**. Any request for a security outside your watchlist is rejected.
* **No External Internet Access (Loopback Enforced)**: All network operations are validated against loopback addresses (`127.0.0.1`, `::1`, `localhost`). Outbound internet traffic is blocked by the built-in firewall.
* **No Third-Party Logins**: Zero TradingView authentication, cookies, sessions, or logins exist in the codebase.
* **No Emailing or External Dispatching**: Zero email/SMTP libraries. All notifications display strictly via **in-app desktop toasts**.
* **Forbidden Protocols Blocked**: SSH, FTP, Telnet, and SMTP protocols and ports (20, 21, 22, 23, 25, 465, 587) are rejected by network assertions.
* **Forbidden Webhooks**: External webhooks (Discord, Slack, or generic URLs) are strictly blocked.
* **Zero Telemetry & No Auto-Updater**: Sparkle updater and phone-home telemetry have been permanently excised.
* **Zero Keys in Git**: No account passwords, API secret keys, or live account credentials are stored or committed to git. Authentication is managed locally by your OpenD daemon.
* **Dependency Supply-Chain Lock (`cargo-deny`)**: Automated policy-based crate auditing enforcing zero known CVEs, banning vulnerable cryptographic routines (`md5`, `des`, `rc4`, `blowfish`, `openssl-sys`), banning undeclared remote network libraries (`curl`, `ssh2`, `ftp`, `telnet`, `smtp`), requiring trusted registries, and validating license compliance across all workspace dependencies. Enforced via CI and pre-commit/pre-push git hooks.
* **macOS Hardened Runtime**: Bundled application executes with Apple Hardened Runtime (`--options runtime`, flags `0x10000`) and strict Library Validation enforced (`app/assets/entitlements.plist`). Prevents local dynamic library (`.dylib`) tampering, injection attacks (`DYLD_INSERT_LIBRARIES`), and unauthorized debugger memory attachment. Verified via `scripts/verify-hardened-runtime.sh`.

> 📖 **Full Security Documentation**: See [SECURITY.md](SECURITY.md) and [docs/SECURITY_AUDIT_AND_HARDENING_REPORT.md](docs/SECURITY_AUDIT_AND_HARDENING_REPORT.md) for full phase-by-phase auditing, firewall architecture, and policy specifications.

---

## Core Capabilities & Architecture

### 1. Dual Charting Architecture (Strict Separation)
- **Standard Watchlist Chart (`GP`)**:
  - Displays real and paper historical executed order fills directly on candlestick bars.
  - **Buy Fills (`B`)**: Green circle badge positioned below candle low.
  - **Sell Fills (`S`)**: Red circle badge positioned above candle high.
  - **Interactive Hover Cards**: Hovering reveals fill type, execution price, quantity, total dollar value, timestamp (UTC), and order ID.
  - **Execution Price Ticks**: Subtle horizontal level ticks on the candle bar.
  - **Stacking**: Multiple fills within the same timeframe candle stack vertically for full visibility.
  - Zero simulated backtest trade markers appear on standard charts.
- **Dedicated Backtest Runner Pane (`BT`)**:
  - Distinct pane tabbed as `BT` with header: `🔬 BACKTEST RUNNER · SYMBOL · ★ Interval · Simulated Trades`.
  - Shows simulated winning parameter trades without cluttering the operational watchlist chart.
  - Quick-switch navigation buttons `[📈 Watchlist Chart]` and `[📊 Lab]`.

### 2. DRBT-Grid: Dynamic Range-Break Trend Strategy & Bot Lab (`AI` Pane)
- Comprehensive quantitative research and execution module (`crates/strategy`).
- **DRBT-Grid Architecture**: Quantizes a rolling 45-period range into 4 adaptive quartile bands, capturing dynamic range breakouts and mean-reversion trends with asymmetric trailing stops.
- **Session Lifecycle Governance (Auto-Idle & Pre-Session Wakeup)**:
  - `🌙 IDLE (Off-Session)` (17:00–08:30 ET & Weekends): Completely idle and sleeping outside open session; consumes zero network and CPU resources.
  - `🌅 WAKING UP (Pre-Session)` (08:30–09:30 ET): Wakes up 1 hour prior to session open to ingest pre-market quotes and compute opening grid levels. Zero new entries permitted.
  - `⚡ OPEN SESSION (Active)` (09:30–16:00 ET): Executes active DRBT-Grid algorithmic entries and dynamic trailing exits.
  - `🌆 POST-SESSION (Wind-Down)` (16:00–17:00 ET): 1 hour after session close to liquidate intraday positions, finalize daily settlement, and transition back to complete idle at 17:00 ET.
- **Interval Sweeps**: Parallel evaluation across 1m, 2m, 3m, 4m, 5m, 6m, 7m, 8m, 9m, and 10m intervals.
- **Hyperparameter Grid Optimization**: Evaluates entry models, exit breakdown triggers, lookback windows, stop losses, take profits, and trailing activations.
- **Winning Parameter Promotion**: Automatically saves top-performing strategy parameters to `strategies/<SYMBOL>/paper.json` for live simulated execution.
- **Live Strategy Runner**: Simulates paper order execution and maintains trade history with equity tracking.

### 3. Apple Silicon Multi-Core Cooling Architecture
- **E-Core Parallelism**: Research sweeps run across all available Apple Silicon Efficiency cores (e.g. 6 E-cores on Apple M4) using scoped concurrency (`std::thread::scope`).
- **Thermal Governor & Background QoS**: Workers apply Darwin background Quality of Service (`QOS_CLASS_BACKGROUND: 0x09`).
- **Hotspot & Throttling Prevention**: Spreads the computational load evenly across all E-cores, keeping Performance (P) cores cool, preventing CPU thermal hotspots, and avoiding fan throttling.

### 4. GPU-Accelerated Charting Engine
- **Rendering**: Pure `wgpu` Metal/Vulkan/DX12 GPU pipeline rendering 1M+ bars smoothly.
- **Timeframes**: 1m, 2m, 3m, 5m, 8m, 15m, 30m, 1H, 4H, 1D, 1W, 1M with linear and logarithmic scales.
- **151 Pure Math Built-In Indicators**: 100% computed in native Rust using closed-form analytical formulas and DSP filters (zero external indicator APIs/services). Includes Moving Averages (SMA, EMA, WMA, HMA, DEMA, TEMA, KAMA, Ehlers SuperSmoother), Volatility Bands (Bollinger, Keltner, Donchian, ATR), Momentum Oscillators (RSI, MACD, Stochastics, CCI), and Session Levels (VWAP, Camarilla/Fibonacci Pivots).
  > 📐 **Full Mathematical Formulations**: See [docs/INDICATOR_MATHEMATICS_AND_NUMERICAL_PRECISION.md](docs/INDICATOR_MATHEMATICS_AND_NUMERICAL_PRECISION.md) for complete analytical equations, DSP dampening coefficients, and IEEE 754 catastrophic-cancellation defenses.
- **Interactive Tools**: Trendlines (Alt+T), Horizontal Lines (Alt+H), Vertical Lines (Alt+V), Fibonacci Retracements (Alt+F).

### 5. Persistent Workspace & Layouts
- **Dockable Panels**: Tiling, splitting, tabbed groupings, and detached floating windows.
- **Full Layout Lifecycle**: Create, save, switch, and delete layouts via the UI (`Layouts › Delete Layout...`) or command line (`DELETE LAYOUT <NAME> <GO>`).
- **Complete Persistence**: All layout configurations, watchlist items, chart intervals, and demo trade histories persist atomically to application storage.

---

## Command Line & Navigation Shortcuts

Enter acts as `<GO>`:

| Command | Action |
|---|---|
| `<SYMBOL> <GO>` | Load chart for watchlist symbol (e.g., `MRNA <GO>`) |
| `<SYMBOL> GP <GO>` | Open standard watchlist price chart with executed fills |
| `<SYMBOL> BT <GO>` | Open dedicated backtest runner chart with simulated trades |
| `<SYMBOL> STRAT <GO>` or `AI <GO>` | Open AI Quantitative Bot & Strategy Lab |
| `SAVE LAYOUT <NAME> <GO>` | Save the current window and pane configuration |
| `DELETE LAYOUT <NAME> <GO>` | Delete a saved layout |
| `HELP <GO>` or `F1` | Show keyboard shortcuts and command reference |

---

## Running with Moomoo OpenD

### 1. Launch Moomoo OpenD
Start your local Moomoo OpenD gateway (or Moomoo desktop app with OpenD external access enabled):
- Default address: `127.0.0.1:11111`
- OpenD manages authentication locally on your computer.

### 2. Run the Terminal
```sh
cargo run --release                          # Connects to local Moomoo OpenD (127.0.0.1:11111)
MOOMOO_OPEND_ADDR=127.0.0.1:11111 cargo run  # Optional custom OpenD port/address
TR_PROVIDER=sample cargo run --release       # Fully offline sample mode
```

---

## Testing & Quality Assurance

Run the test suite across workspace crates:
```sh
# Verify order fill synchronization and backtest marker separation
cargo test --bin terminal -- standard_chart_separates_strategy_markers_and_syncs_order_fills

# Verify OpenD loopback firewall and security constraints
cargo test --package tr-moomoo

# Verify strategy governor and backtesting optimizer
cargo test --package tr-strategy

# Full workspace compilation check
cargo check --workspace
```
