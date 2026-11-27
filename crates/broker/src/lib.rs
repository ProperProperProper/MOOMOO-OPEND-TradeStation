//! Brokerage integration through Moomoo OpenD: client, normalised portfolio model
//! (accounts, balances, positions, orders, activity), secure credential storage,
//! an offline simulated portfolio and a local paper-trading [`demo`] account.
//!
//! Order entry is in [`trading`]: every order is previewed with an order-impact check
//! before it can be placed, and the terminal gates live orders behind an explicit setting.

pub mod demo;
pub mod model;
pub mod moomoo;
pub mod sample;
pub mod secrets;
pub mod trading;

pub use model::{Account, Activity, Connection, InstrumentKind, Order, Portfolio, Position, Totals};
pub use moomoo::{ACTIVITY_DAYS, MoomooClient, ORDER_DAYS};
pub use secrets::{Credentials, SecretStore};
