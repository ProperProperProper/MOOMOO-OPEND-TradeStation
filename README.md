# MOOMOO-OPEND-TradeStation

A fast, keyboard-driven market terminal in pure Rust: GPU charting with a Bloomberg Terminal look (dense tiled panes, monospace numbers, a command line like `AAPL <GO>`), **dedicated exclusively to Moomoo data via the local OpenD gateway** for your Moomoo account watchlist. Native compact UI, custom themes, and a fully dockable workspace with saved layouts.

![Terminal Overview](docs/img/overview.png)

> **Security & Privacy First Architecture:**
> All external internet data feeds (Yahoo Finance, Alpaca, Binance, Coinbase, US Treasury scrapers, external RSS feeds, and congressional web scrapers) have been replaced or disabled. The application communicates **strictly and exclusively with your local Moomoo OpenD gateway (`127.0.0.1:11111`)**.

---

## Strict Security & Isolation Guarantees

* **Moomoo OpenD Only**: The application connects exclusively to your local OpenD gateway (default `127.0.0.1:11111`).
* **Watchlist-Restricted Data Feeds**: Market data quotes, history candles, searches, and tick streaming are **strictly restricted to instruments in your Moomoo account watchlist**. Any request for a security outside your watchlist is rejected.
* **No External Internet Access (Loopback Enforced)**: All network operations are strictly validated against loopback addresses (`127.0.0.1`, `::1`, `localhost`). Outbound requests to remote internet IPs or domains are blocked with an immediate security alert.
* **No TradingView Logins**: Zero TradingView authentication, cookies, sessions, or logins exist in the codebase.
* **No Emailing or External Dispatching**: Zero email/SMTP libraries or services. All notifications and alerts display strictly on your local screen via **in-app desktop toasts**.
* **Forbidden Protocols Blocked**: SSH, FTP, Telnet, and SMTP protocols and ports (20, 21, 22, 23, 25, 465, 587) are blocked by the built-in security firewall.
* **Forbidden Webhooks**: Webhooks (Discord, Slack, or generic webhook URLs) are forbidden and blocked by network assertion.
* **Zero Telemetry & No Auto-Updater**: Sparkle updater and all update commands have been permanently excised. The app has zero phone-home or analytics telemetry.
* **Zero Keys in Git**: No account passwords, API secret keys, or tokens are stored or committed to the repository. Authentication is handled locally by your OpenD daemon.

> 📖 **Full Security Documentation**: See [SECURITY.md](SECURITY.md) and [docs/SECURITY_AUDIT_AND_HARDENING_REPORT.md](docs/SECURITY_AUDIT_AND_HARDENING_REPORT.md) for full phase-by-phase auditing, firewall architecture, and policy specifications.

---

## Features

- **Multi-Asset with Bloomberg-Style Market Keys**: `AAPL <EQUITY>`, `NVDA <EQUITY>`, `MSFT <EQUITY>`, `SPY <EQUITY>`, `QQQ <EQUITY>`.
- **Command Line and Search**: `<SECURITY> [<FUNCTION>] <GO>` (Enter is `<GO>`). Search is confined to your Moomoo account watchlist.
- **Charts**: GPU-rendered candles, OHLC bars, line and area; 1m, 5m, 15m, 30m, 1H, 4H, 1D, 1W, 1M timeframes; log scale; crosshair with OHLC legend; smooth pan and zoom over 1M+ bars.
- **151 Built-In Indicators**: Moving averages (SMA, EMA, WMA, Hull, DEMA, TEMA, KAMA, VWAP), bands/channels (Bollinger, Keltner, Donchian), momentum oscillators (RSI, MACD, Stochastic, CCI), volatility measures (ATR), and pivot-point families.
- **Drawings**: Trendline (Alt+T), horizontal (Alt+H), vertical (Alt+V), and Fibonacci retracement (Alt+F).
- **Moomoo Market Data Provider (`tr_data::MarketData`)**:
  
  | Asset Class | History | Quotes | Live Streaming |
  |---|---|---|---|
  | Moomoo Account Watchlist Equities & ETFs | Local Moomoo OpenD (`127.0.0.1:11111`) | Local Moomoo OpenD (`127.0.0.1:11111`) | Local Moomoo OpenD Streaming |
  | Non-Watchlist Instruments | Blocked | Blocked | Blocked |

- **Dockable Workspace**: Drag tabs and headers to split, tab, or pop out panes.
- **Themes**: Built-in themes (Terminal Dark/Light, Bloomberg, High Contrast, Solarized, Nord, Monokai) plus live theme editor.

---

## Running with Moomoo OpenD

### 1. Launch Moomoo OpenD
Start your local Moomoo OpenD client (or Moomoo desktop app with External Access enabled):
- Default address: `127.0.0.1:11111`
- OpenD manages authentication locally on your computer.

### 2. Run the Application
```sh
cargo run --release                          # Connects to local Moomoo OpenD (127.0.0.1:11111)
MOOMOO_OPEND_ADDR=127.0.0.1:11111 cargo run  # Optional custom OpenD port/address
TR_PROVIDER=sample cargo run --release       # Fully offline sample mode
```

---

## Testing & Verification

Run the security test suite to verify firewall and watchlist constraints:
```sh
cargo test --package tr-moomoo
```
Run workspace-wide compile checks:
```sh
cargo check --workspace
```
