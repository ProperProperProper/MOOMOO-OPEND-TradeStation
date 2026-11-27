# tr-gateway (Standalone Gateway)

> **Notice**: For **MOOMOO-OPEND-TradeStation**, the desktop application connects directly to your local **Moomoo OpenD (`127.0.0.1:11111`)** on loopback. The standalone server gateway is an optional component and is not used for primary Moomoo OpenD operations.

## Security & Networking
* All connections are restricted to local loopback addresses.
* Third-party webhooks, telemetry, and external public feeds are disabled under the security hardening policy.

## Running
```sh
cargo run --release -p tr-gateway
```
Listening on `localhost:8787`.
