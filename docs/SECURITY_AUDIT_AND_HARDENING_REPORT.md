# Security & Architecture Policy: MOOMOO-OPEND-TradeStation

**Project**: `ProperProperProper/MOOMOO-OPEND-TradeStation`  
**Date**: November 28, 2026  
**Status**: Fully Hardened & Verified  

---

## 1. Executive Summary

This document details the comprehensive security audit, architectural transition, and network hardening performed on the codebase. 

The application has been transformed from a multi-provider public terminal into an isolated, privacy-first desktop terminal that communicates **exclusively with a local Moomoo OpenD gateway (`127.0.0.1:11111`)**. All external third-party data providers, web scrapers, remote management protocols, auto-updaters, webhooks, and telemetry tracking have been permanently removed or actively blocked.

---

## 2. Core Security Mandates & Policies

The following strict security constraints are enforced in the source code:

| Mandate | Specification | Implementation Mechanism |
|---|---|---|
| **Zero External Internet Access** | The app must not initiate socket connections to any external IP or public domain. | Loopback network validator (`validate_loopback_only`) rejects non-loopback IPs and hostnames. |
| **Strict Moomoo OpenD Isolation** | All communications must route only to the local Moomoo OpenD gateway. | Direct client binding to `127.0.0.1:11111` with protocol validation. |
| **Watchlist-Restricted Data Feeds** | Market data, quotes, candlesticks, and tick streaming may only be fetched for securities in the user's Moomoo account watchlist. | Active gatekeeper in `OpenDClient` and `Markets` provider checking membership before dispatch. |
| **Forbidden Remote Protocols** | Connections via SSH, FTP, SFTP, Telnet, or SMTP are prohibited. | Protocol filter and port blocking (20, 21, 22, 23, 25, 465, 587, etc.) with security alerts. |
| **Zero Webhooks** | Outgoing webhooks (Discord, Slack, custom webhook URLs) are forbidden. | Webhook URL path filtering and immediate bail out with alert warnings. |
| **Desktop-Only Notifications** | Notifications must stay on the user's physical screen. | In-app animated toast system (`toasts(&self, ctx)`) renders locally; zero remote push services. |
| **No TradingView Logins** | No TradingView credentials, sessions, cookies, or account logins. | Codebase audited; zero TradingView auth handlers exist. |
| **Zero Auto-Updater / Telemetry** | No background checking or downloading of application updates. | Sparkle framework, update commands, and release feed checkers permanently deleted. |
| **Zero Credentials in Git** | No passwords, private keys, API keys, or tokens stored in repo. | Strict `.gitignore` rules and full diff audit verifying zero secrets committed. |

---

## 3. Step-by-Step Implementation Details

### Step 1: Moomoo OpenD Crate Creation (`crates/moomoo`)
* **Dedicated Crate**: Built `crates/moomoo` with its own `Cargo.toml` and registered it in the workspace root.
* **Core API**: Implemented `OpenDClient` supporting:
  * `fetch_watchlist`: Retrieves the authenticated user's Moomoo account watchlist from local OpenD.
  * `fetch_quote` & `fetch_quotes`: Retrieves live quote snapshots.
  * `fetch_history`: Retrieves historical OHLCV bars.
  * `subscribe_quotes`: Streams live tick updates.
* **Fallback & Cache**: Caches the watchlist in memory (`Arc<RwLock<Option<HashSet<String>>>>`) for sub-millisecond enforcement.

### Step 2: Active Network Firewall (`validate_loopback_only`)
Implemented an active network barrier in `crates/moomoo/src/lib.rs`:
```rust
pub fn validate_loopback_only(addr_str: &str) -> Result<()>
```
1. **Scheme Inspection**: Rejects `ssh://`, `sftp://`, `ftp://`, `ftps://`, `smtp://`, `smtps://`, `mailto:`, `pop3://`, `imap://`, `telnet://`, `irc://`.
2. **Webhook Rejection**: Detects and rejects URLs containing `webhook`, `/hooks/`, `discord.com/api`, or `slack.com/services`.
3. **Port Filtering**: Forbids connection attempts to remote administration and email ports (`20`, `21`, `22`, `23`, `25`, `110`, `143`, `465`, `587`, `993`, `995`).
4. **IP Resolution Check**: Resolves all target hostnames and enforces that every resolved IP satisfies `ip.is_loopback()`.
5. **Warning Alerts**: Any violation immediately outputs to `stderr` and logs a high-severity alert:
   ```
   ⚠️ SECURITY ALERT: Prohibited protocol/port/destination blocked: <target>.
   ```

### Step 3: Watchlist-Only Data Feed Enforcement
Enforced at both the OpenD client level and the market data abstraction layer (`crates/data/src/markets.rs`):
* **Quote Requests**: Quotes are only fetched if `is_in_watchlist(&symbol)` returns `true`.
* **History Requests**: History requests for unlisted symbols abort with:
  ```
  ⚠️ SECURITY ALERT: Rejected history request for '<symbol>': symbol is NOT in your Moomoo account watchlist.
  ```
* **Streaming Subscriptions**: Background subscription tasks verify watchlist membership before subscribing to tick channels.
* **Symbol Search**: The in-terminal search bar only returns hits that exist within the user's Moomoo account watchlist.

### Step 4: Disconnection & Neutralization of Third-Party Feeds
* **Yahoo Finance, Alpaca, Binance, Coinbase, US Treasury**: The multi-source router in `crates/data/src/markets.rs` was replaced with direct delegation to `OpenDClient`.
* **Congressional Web Scrapers**: In `crates/congress/src/refresh.rs`, external scraping of Senate eFD (`efdsearch.senate.gov`) and House Clerk (`disclosures-clerk.house.gov`) was disabled; `HttpSource` was converted to a local no-op.
* **SnapTrade & Hyperliquid Endpoints**: Base URLs in `crates/broker/src/snaptrade.rs` and `crates/hl/src/network.rs` were restricted to loopback with security validation.

### Step 5: Removal of Auto-Updater & Telemetry
* **Deleted Sparkle Updater**: Permanently removed `crates/macos/src/updater.rs`, deleted `scripts/fetch-sparkle.sh`, and removed Sparkle framework embedding from `scripts/bundle-macos.sh`.
* **Removed Update Command**: Deleted `Function::Update` from `app/src/command.rs` and removed background startup checks from `app/src/app.rs`.
* **Zero Telemetry**: No crash reporters, analytics SDKs, or background pingers exist in the project.

### Step 6: Webhook & Remote Notification Elimination
* **No Webhooks**: The codebase was audited and confirmed to contain zero webhooks.
* **In-App Toast Layer**: All alerts, order fills, and system notices display exclusively on the user's screen through the immediate-mode UI painter in `app/src/app/chrome.rs` (`toasts(&self, ctx)`).

### Step 7: Audit & Hardening of the `scripts/` Directory
Every script in `scripts/` was audited:
* `build-icon.sh`: Safe. Uses local Chrome and macOS `sips`/`iconutil` locally. Zero network calls.
* `bundle-macos.sh`: Hardened. Removed third-party Sparkle framework download/linking.
* `check-macos-cross.sh`: Safe. Runs local `cargo clippy` cross-compile check.
* `release-macos.sh`: Hardened. Removed all Sparkle key generation and remote feed signing.
* `build-web.sh`: Hardened. Removed default external server endpoints.
* `fetch-sparkle.sh`: Permanently deleted.

### Step 8: Credential & Secret Audit
* **.gitignore**: Updated with comprehensive ignore rules for `.env*`, `*.key`, `*.pem`, `*.secret`, `*token*`, and JSON credential files.
* **Pre-Push Diff Scan**: Scanned all commits (`git diff origin/master..HEAD`) to verify zero passwords, secret keys, or authentication tokens were staged.
* **Authentication Method**: The terminal relies entirely on the local Moomoo OpenD process running on `127.0.0.1:11111` for authentication.

---

## 4. Automated Verification & Testing

### Test Suite Execution
```sh
cargo test --package tr-moomoo
```
**Results**:
* `test tests::test_loopback_validation ... ok`: Verified that localhost/127.0.0.1 passes, while `ssh://`, `ftp://`, `smtp://`, webhooks, port 22, port 21, and external domains fail with security alerts.
* `test tests::test_watchlist_enforcement ... ok`: Verified that watchlist tickers succeed and non-watchlist tickers are blocked.

### Workspace Build Check
```sh
cargo check --workspace
```
**Results**:
* `0 errors, 0 warnings`. The entire workspace compiles cleanly.

---

## 5. Repository Provenance

* **Repository**: [https://github.com/ProperProperProper/MOOMOO-OPEND-TradeStation](https://github.com/ProperProperProper/MOOMOO-OPEND-TradeStation)
* **Default Branch**: `master`
* **Verified Commits**:
  * `bd63270`: Security hardening, loopback firewall, updater removal, webhook prohibition.
  * `3a9a0fd`: Full markdown documentation update across all subdirectories.
  * `d2d6b1e`: Cleaned release and packaging scripts.
