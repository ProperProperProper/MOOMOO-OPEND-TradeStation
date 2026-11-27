#!/usr/bin/env python3
"""
Fetches 1000 x 1m candles via local OpenD (127.0.0.1:11111) for all watchlist symbols.
Enforces proper rate limit management and NASDAQ-US-ANT-TRADER candle cleanup & validation.
Persists cleanly to TradeStation's bars.sqlite3 and ANT-TRADER's quant_dashboard.sqlite3.
"""

import os
import sys
import time
import sqlite3
from pathlib import Path
import pandas as pd

from moomoo import OpenQuoteContext, KLType, AuType, KL_FIELD, RET_OK

TERMINAL_DB = Path.home() / "Library/Application Support/terminal/bars.sqlite3"
ANT_TRADER_DB = Path("/Users/local.local/Projects/NASDAQ-US-ANT-TRADER/quant_dashboard.sqlite3")

SYMBOLS = [
    "US.MRNA",
    "US.ANT",
    "US.NVDA",
    "US.MU",
    "US.SNDK",
    "US.OMH",
    "US.APP",
]

def clean_candle_frame(df: pd.DataFrame) -> pd.DataFrame:
    """
    Applies the exact cleaning and validation pipeline from NASDAQ-US-ANT-TRADER:
    - Drops is_blank candles
    - Drops null values in OHLC
    - Validates strictly positive prices
    - Validates geometric OHLC integrity (high >= low, high >= max(open, close), low <= min(open, close))
    - Drops stale phantom candles (volume == 0 with identical OHLC)
    - Deduplicates on timestamp and sorts chronologically
    """
    if df is None or df.empty:
        return pd.DataFrame()

    x = df.copy()

    # 1. Drop blank candles flagged by OpenD
    if "is_blank" in x.columns:
        x = x[x["is_blank"] != True]

    # 2. Drop missing prices
    x = x.dropna(subset=["open", "high", "low", "close"])

    # 3. Numeric conversion
    for col in ["open", "high", "low", "close", "volume"]:
        if col in x.columns:
            x[col] = pd.to_numeric(x[col], errors="coerce")

    # 4. Filter strictly positive prices
    x = x[(x["open"] > 0) & (x["high"] > 0) & (x["low"] > 0) & (x["close"] > 0)]

    # 5. Geometric OHLC validity
    x = x[
        (x["high"] >= x["low"]) &
        (x["high"] >= x[["open", "close"]].max(axis=1)) &
        (x["low"] <= x[["open", "close"]].min(axis=1))
    ]

    # 6. Drop stale phantom candles (0 volume with completely flat zero-spread price)
    if "volume" in x.columns:
        stale_mask = (x["volume"] <= 0) & (x["high"] == x["low"]) & (x["open"] == x["close"])
        x = x[~stale_mask]

    # 7. Deduplicate time_key and sort
    x["time_key"] = pd.to_datetime(x["time_key"])
    x = x.sort_values("time_key").drop_duplicates("time_key")

    return x

def store_in_terminal_db(clean_df: pd.DataFrame, raw_symbol: str):
    """Stores 1m candles into TradeStation's bars.sqlite3 under multiple alias keys."""
    if clean_df.empty:
        return 0

    TERMINAL_DB.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(TERMINAL_DB, timeout=30)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute("""
        CREATE TABLE IF NOT EXISTS bars (
            symbol TEXT NOT NULL, tf TEXT NOT NULL, ts INTEGER NOT NULL,
            o REAL NOT NULL, h REAL NOT NULL, l REAL NOT NULL, c REAL NOT NULL, v REAL NOT NULL,
            PRIMARY KEY (symbol, tf, ts)
        ) WITHOUT ROWID;
    """)

    bare_ticker = raw_symbol.replace("US.", "")
    bloomberg_key = f"{bare_ticker} US Equity"
    moomoo_key = raw_symbol

    keys_to_store = [bare_ticker, bloomberg_key, moomoo_key]

    rows = []
    for row in clean_df.itertuples():
        ts = int(pd.Timestamp(row.time_key).timestamp())
        o = float(row.open)
        h = float(row.high)
        l = float(row.low)
        c = float(row.close)
        v = float(getattr(row, "volume", 0.0) or 0.0)

        for sym in keys_to_store:
            rows.append((sym, "1m", ts, o, h, l, c, v))

    cur = conn.cursor()
    cur.executemany("""
        INSERT INTO bars (symbol, tf, ts, o, h, l, c, v) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT (symbol, tf, ts) DO UPDATE SET
            o = excluded.o, h = excluded.h, l = excluded.l, c = excluded.c, v = excluded.v
    """, rows)
    conn.commit()
    conn.close()
    return len(clean_df)

def store_in_ant_trader_db(clean_df: pd.DataFrame, raw_symbol: str):
    """Stores candles into NASDAQ-US-ANT-TRADER's quant_dashboard.sqlite3."""
    if clean_df.empty or not ANT_TRADER_DB.parent.exists():
        return 0

    conn = sqlite3.connect(ANT_TRADER_DB, timeout=30)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("""
        CREATE TABLE IF NOT EXISTS candles (
            symbol TEXT NOT NULL,
            interval_minutes INTEGER NOT NULL,
            time_key TEXT NOT NULL,
            open REAL NOT NULL,
            high REAL NOT NULL,
            low REAL NOT NULL,
            close REAL NOT NULL,
            volume REAL,
            turnover REAL,
            source TEXT NOT NULL DEFAULT 'moomoo_opend',
            PRIMARY KEY (symbol, interval_minutes, time_key)
        );
    """)

    rows = []
    for row in clean_df.itertuples():
        time_str = pd.Timestamp(row.time_key).isoformat()
        o = float(row.open)
        h = float(row.high)
        l = float(row.low)
        c = float(row.close)
        v = float(getattr(row, "volume", 0.0) or 0.0)
        turnover = float(getattr(row, "turnover", 0.0) or 0.0)
        rows.append((raw_symbol, 1, time_str, o, h, l, c, v, turnover, "moomoo_opend"))

    cur = conn.cursor()
    cur.executemany("""
        INSERT OR REPLACE INTO candles
        (symbol, interval_minutes, time_key, open, high, low, close, volume, turnover, source)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """, rows)
    conn.commit()
    conn.close()
    return len(clean_df)

def main():
    print(f"Connecting to Moomoo OpenD on 127.0.0.1:11111...")
    quote = OpenQuoteContext(host="127.0.0.1", port=11111)

    total_candles = 0

    try:
        for idx, symbol in enumerate(SYMBOLS, 1):
            print(f"[{idx}/{len(SYMBOLS)}] Fetching 1000 x 1m candles for {symbol}...")
            
            # Rate limit backoff loop
            attempts = 0
            raw_data = None
            while attempts < 3:
                attempts += 1
                ret, data, page_key = quote.request_history_kline(
                    code=symbol,
                    start="",
                    end="",
                    ktype=KLType.K_1M,
                    autype=AuType.QFQ,
                    fields=[KL_FIELD.ALL],
                    max_count=1000,
                    extended_time=False,
                )
                if ret == RET_OK:
                    raw_data = data
                    break
                else:
                    err_msg = str(data)
                    print(f"  Attempt {attempts} failed: {err_msg}")
                    if "frequency" in err_msg.lower() or "limit" in err_msg.lower():
                        print("  Rate limit encountered, backing off 3.0s...")
                        time.sleep(3.0)
                    else:
                        time.sleep(1.0)

            if raw_data is None or raw_data.empty:
                print(f"  FAILED to fetch {symbol}")
                continue

            raw_count = len(raw_data)
            clean_df = clean_candle_frame(raw_data)
            clean_count = len(clean_df)

            # Store in TradeStation
            n_term = store_in_terminal_db(clean_df, symbol)
            # Store in ANT-TRADER
            n_ant = store_in_ant_trader_db(clean_df, symbol)

            stale_dropped = raw_count - clean_count
            print(f"  {symbol}: {clean_count} clean 1m candles stored (dropped {stale_dropped} junk/stale). Oldest: {clean_df['time_key'].iloc[0]} -> Latest: {clean_df['time_key'].iloc[-1]}")
            total_candles += clean_count

            # Pacing: sleep 0.6s to strictly respect OpenD rate limits
            time.sleep(0.6)

    finally:
        quote.close()

    print(f"\nSUCCESS: Ingested {total_candles} clean 1-minute candles across {len(SYMBOLS)} symbols.")

if __name__ == "__main__":
    main()
