#!/usr/bin/env python3
"""
Runs the exact live strategy using each symbol's saved paper.json parameters
specifically on the LAST LIVE SESSION (2026-11-27).
Tests all 4 dynamic dimensions:
1. Exits & Stops (Static % in paper.json vs Dynamic ATR)
2. Execution Slippage (Static 2 bps vs Real Volatility Spread)
3. Channel Lookback (Saved lookback in paper.json vs Dynamic Adaptive)
4. Asset Allocation (Single symbol vs Watchlist relative ranking)
"""

import json
import sqlite3
from pathlib import Path
import numpy as np
import pandas as pd

ROOT_DIR = Path("/Users/local.local/Projects/TradeStation_repo")
TERMINAL_DB = Path.home() / "Library/Application Support/terminal/bars.sqlite3"
STRATEGIES_DIR = ROOT_DIR / "strategies"

SYMBOLS = ["US.MRNA", "US.ANT", "US.NVDA", "US.MU", "US.APP", "US.OMH", "US.SNDK"]

def load_symbol_paper_config(symbol):
    clean_sym = symbol.replace(".", "-")
    p = STRATEGIES_DIR / clean_sym / "paper.json"
    if p.exists():
        with open(p) as f:
            return json.load(f)
    return None

def load_last_session_bars(symbol):
    conn = sqlite3.connect(TERMINAL_DB)
    # The last session in the database is 2026-11-27 (Black Friday session)
    df = pd.read_sql_query(
        """
        SELECT ts, o as open, h as high, l as low, c as close, v as volume
        FROM bars
        WHERE symbol = ? AND tf = '1m' AND date(ts, 'unixepoch') = '2026-11-27'
        ORDER BY ts ASC
        """,
        conn,
        params=(symbol,),
    )
    conn.close()
    if not df.empty:
        df["datetime"] = pd.to_datetime(df["ts"], unit="s", utc=True)
        df.sort_values("ts", inplace=True)
        df.reset_index(drop=True, inplace=True)
    return df

def aggregate_candles(df_1m, interval_min):
    if interval_min <= 1 or df_1m.empty:
        return df_1m.copy()
    
    interval_sec = interval_min * 60
    buckets = []
    
    # Bucket timestamps aligned with interval
    ts = df_1m["ts"].values
    o = df_1m["open"].values
    h = df_1m["high"].values
    l = df_1m["low"].values
    c = df_1m["close"].values
    v = df_1m["volume"].values
    
    current_bucket = None
    b_o, b_h, b_l, b_c, b_v = 0.0, -1e9, 1e9, 0.0, 0.0
    
    for i in range(len(df_1m)):
        bucket = ts[i] - (ts[i] % interval_sec)
        if current_bucket is None:
            current_bucket = bucket
            b_o, b_h, b_l, b_c, b_v = o[i], h[i], l[i], c[i], v[i]
        elif bucket == current_bucket:
            b_h = max(b_h, h[i])
            b_l = min(b_l, l[i])
            b_c = c[i]
            b_v += v[i]
        else:
            buckets.append({"ts": current_bucket, "open": b_o, "high": b_h, "low": b_l, "close": b_c, "volume": b_v})
            current_bucket = bucket
            b_o, b_h, b_l, b_c, b_v = o[i], h[i], l[i], c[i], v[i]
            
    if current_bucket is not None:
        buckets.append({"ts": current_bucket, "open": b_o, "high": b_h, "low": b_l, "close": b_c, "volume": b_v})
        
    df_agg = pd.DataFrame(buckets)
    df_agg["datetime"] = pd.to_datetime(df_agg["ts"], unit="s", utc=True)
    return df_agg

def compute_atr(df, period=14):
    h, l, c = df["high"].values, df["low"].values, df["close"].values
    n = len(df)
    if n == 0: return np.array([])
    tr = np.zeros(n)
    tr[0] = h[0] - l[0]
    for i in range(1, n):
        tr[i] = max(h[i] - l[i], abs(h[i] - c[i-1]), abs(l[i] - c[i-1]))
    atr = pd.Series(tr).ewm(alpha=1.0/min(period, max(1, n)), adjust=False).mean().values
    return atr

def compute_grid_tiers(highs, lows, lookback, intervals):
    n = len(highs)
    top = np.full(n, np.nan)
    bottom = np.full(n, np.nan)
    upper_mid = np.full(n, np.nan)
    lower_mid = np.full(n, np.nan)
    
    lb = max(2, min(lookback, n - 1))
    for i in range(lb, n):
        prior_h = np.max(highs[i-lb:i])
        prior_l = np.min(lows[i-lb:i])
        rng = prior_h - prior_l
        top[i] = prior_h
        bottom[i] = prior_l
        lower_mid[i] = prior_l + rng * 0.25
        upper_mid[i] = prior_l + rng * 0.50
        
    return top, bottom, upper_mid, lower_mid

def run_simulation(df_bars, params, interval, 
                   use_dynamic_atr=False,
                   use_dynamic_spread=False,
                   use_dynamic_lookback=False,
                   capital=100_000.0,
                   allocation=0.95):
    
    if df_bars.empty:
        return {"trades": 0, "win_rate": 0.0, "pnl": 0.0, "ret_pct": 0.0, "pf": 0.0, "max_dd": 0.0}
    
    # 1. Aggregate bars to symbol's exact timeframe
    df = aggregate_candles(df_bars, interval)
    n = len(df)
    if n < 4:
        return {"trades": 0, "win_rate": 0.0, "pnl": 0.0, "ret_pct": 0.0, "pf": 0.0, "max_dd": 0.0}
        
    close = df["close"].values
    high = df["high"].values
    low = df["low"].values
    open_p = df["open"].values
    ts = df["ts"].values
    
    atrs = compute_atr(df, 14)
    
    lookback = params.get("grid_lookback", 45)
    intervals = params.get("grid_intervals", 4)
    tp_pct = params.get("take_profit_pct", 0.01)
    sl_pct = params.get("stop_loss_pct", 0.1324)
    trailing_act = params.get("trailing_activation_pct", 0.0)
    trailing_stop = params.get("trailing_stop_pct", 0.0)
    open_delay_min = params.get("open_delay_minutes", 0)
    exit_mode = params.get("exit_mode", "any_grid_cross_down")
    
    # Dynamic lookback adjustment
    if use_dynamic_lookback:
        # Scale lookback dynamically with short-term volatility
        ret = pd.Series(close).pct_change().abs().values
        med = np.nanmedian(ret) or 0.001
        lookback = int(np.clip(lookback * (0.6 if np.mean(ret[-5:]) > med else 1.3), 3, 40))
        
    top, bottom, upper_mid, lower_mid = compute_grid_tiers(high, low, lookback, intervals)
    
    cash = capital
    pos_qty = 0.0
    entry_p = 0.0
    peak_p = 0.0
    trades = []
    
    start_bar = min(lookback + 1, n - 1)
    
    for i in range(start_bar, n):
        curr_p = close[i]
        atr = atrs[i] if i < len(atrs) else (curr_p * 0.01)
        
        # Slippage rate
        if use_dynamic_spread:
            bar_spread = (high[i] - low[i]) / curr_p
            slip_rate = max(0.0001, min(0.0015, bar_spread * 0.25))
        else:
            slip_rate = 0.0002 # 2 bps
            
        # Session open delay check
        # First bar on 2026-11-27 is 09:31 EST (14:31 UTC).
        minutes_from_open = (ts[i] - ts[0]) // 60
        entry_allowed = minutes_from_open >= open_delay_min
        
        # 1. Evaluate Exits
        if pos_qty > 0.0:
            if curr_p > peak_p:
                peak_p = curr_p
                
            exit_reason = None
            
            if use_dynamic_atr:
                # Dynamic ATR exit
                tp_target = entry_p + (atr * 2.5)
                sl_target = entry_p - (atr * 1.8)
                t_act = entry_p + (atr * 1.5)
                t_stop = peak_p - (atr * 1.0)
                
                if curr_p >= tp_target:
                    exit_reason = "TP_ATR"
                elif curr_p <= sl_target:
                    exit_reason = "SL_ATR"
                elif peak_p >= t_act and curr_p <= t_stop:
                    exit_reason = "TRAILING_ATR"
            else:
                # Exact static params from paper.json
                if tp_pct > 0.0 and curr_p >= entry_p * (1.0 + tp_pct):
                    exit_reason = "TP_STATIC"
                elif sl_pct > 0.0 and curr_p <= entry_p * (1.0 - sl_pct):
                    exit_reason = "SL_STATIC"
                elif trailing_act > 0.0 and trailing_stop > 0.0:
                    if peak_p >= entry_p * (1.0 + trailing_act) and curr_p <= peak_p * (1.0 - trailing_stop):
                        exit_reason = "TRAILING_STATIC"
                        
            # Grid breakdown exit
            if not exit_reason:
                if exit_mode == "bottom_breakdown" and curr_p < bottom[i]:
                    exit_reason = "BOTTOM_BREAKDOWN"
                elif exit_mode == "lower_half_breakdown" and curr_p < lower_mid[i]:
                    exit_reason = "LOWER_MID_BREAKDOWN"
                elif exit_mode == "any_grid_cross_down" and curr_p < upper_mid[i]:
                    exit_reason = "GRID_CROSS_DOWN"
                    
            # Session close exit at end of day
            if not exit_reason and i == n - 1:
                exit_reason = "SESSION_CLOSE"
                
            if exit_reason:
                fill_exit = curr_p * (1.0 - slip_rate)
                net = (fill_exit - entry_p) * pos_qty
                ret_pct = ((fill_exit - entry_p) / entry_p) * 100.0
                cash += pos_qty * fill_exit
                trades.append({
                    "entry": entry_p,
                    "exit": fill_exit,
                    "qty": pos_qty,
                    "net_pnl": net,
                    "ret_pct": ret_pct,
                    "reason": exit_reason,
                    "slip_bps": slip_rate * 10_000,
                })
                pos_qty = 0.0
                entry_p = 0.0
                peak_p = 0.0
                
        # 2. Evaluate Entries
        elif pos_qty == 0.0 and entry_allowed and i < n - 1:
            prev_c = close[i-1]
            # Standard grid cross up
            if not np.isnan(upper_mid[i]) and prev_c <= upper_mid[i] and curr_p > upper_mid[i]:
                fill_entry = curr_p * (1.0 + slip_rate)
                spend = cash * allocation
                qty = np.floor(spend / fill_entry)
                if qty > 0:
                    cash -= qty * fill_entry
                    pos_qty = qty
                    entry_p = fill_entry
                    peak_p = fill_entry

    if not trades:
        return {"trades": 0, "win_rate": 0.0, "pnl": 0.0, "ret_pct": 0.0, "pf": 0.0, "max_dd": 0.0, "slip_bps": 2.0}
        
    wins = [t["net_pnl"] for t in trades if t["net_pnl"] > 0]
    losses = [abs(t["net_pnl"]) for t in trades if t["net_pnl"] < 0]
    win_rate = (len(wins) / len(trades)) * 100.0
    total_pnl = sum(t["net_pnl"] for t in trades)
    tot_win = sum(wins)
    tot_loss = sum(losses)
    pf = (tot_win / tot_loss) if tot_loss > 0 else (99.0 if tot_win > 0 else 0.0)
    
    eq = [capital]
    cur = capital
    for t in trades:
        cur += t["net_pnl"]
        eq.append(cur)
    peaks = np.maximum.accumulate(eq)
    dds = (peaks - eq) / peaks * 100.0
    
    return {
        "trades": len(trades),
        "win_rate": round(win_rate, 1),
        "pnl": round(total_pnl, 2),
        "ret_pct": round((total_pnl / capital) * 100.0, 2),
        "pf": round(pf, 2),
        "max_dd": round(np.max(dds), 2),
        "slip_bps": round(np.mean([t["slip_bps"] for t in trades]), 1),
    }

def main():
    print("=" * 85)
    print(" LIVE SESSION TEST: USING EACH SYMBOL'S SAVED 'paper.json' CONFIGURATION")
    print(" Session Date: 2026-11-27 (Most Recent Live Market Session in Database)")
    print("=" * 85)
    
    print("\nSaved Configurations from disk (strategies/*/paper.json):")
    configs = {}
    for sym in SYMBOLS:
        cfg = load_symbol_paper_config(sym)
        if cfg:
            configs[sym] = cfg
            p = cfg["params"]
            print(f"  • {sym:<8}: Interval {cfg['interval']:2}m | Lookback {p.get('grid_lookback', 45):2} | "
                  f"Tiers {p.get('grid_intervals', 4):2} | Delay {p.get('open_delay_minutes', 0):2}m | "
                  f"TP {p.get('take_profit_pct', 0.0)*100:.2f}% | SL {p.get('stop_loss_pct', 0.0)*100:.2f}% | "
                  f"Exit: {p.get('exit_mode', 'default')}")

    results_baseline = {}
    results_dyn_exits = {}
    results_dyn_slippage = {}
    results_dyn_lookback = {}
    
    print("\n" + "-" * 85)
    print(" 1. EXACT BASELINE: RUNNING EACH SYMBOL WITH ITS SAVED LIVE PARAMETERS (2026-11-27)")
    print("-" * 85)
    print(f"  {'Symbol':<10} | {'Interval':<8} | {'Trades':<8} | {'Win Rate':<10} | {'PnL ($)':<12} | {'Return %':<10} | {'Profit Factor':<13} | {'Max DD %':<8}")
    print(f"  {'-'*10}-|-{'-'*8}-|-{'-'*8}-|-{'-'*10}-|-{'-'*12}-|-{'-'*10}-|-{'-'*13}-|-{'-'*8}")
    
    for sym in SYMBOLS:
        cfg = configs.get(sym)
        if not cfg: continue
        df = load_last_session_bars(sym)
        res = run_simulation(df, cfg["params"], cfg["interval"], use_dynamic_atr=False, use_dynamic_spread=False)
        results_baseline[sym] = res
        print(f"  {sym:<10} | {cfg['interval']:<8}m | {res['trades']:<8} | {res['win_rate']:<9}% | ${res['pnl']:<11} | {res['ret_pct']:<9}% | {res['pf']:<13} | {res['max_dd']:<8}%")

    print("\n" + "-" * 85)
    print(" 2. DYNAMIC TEST 1: VOLATILITY-SCALED ATR EXITS (vs. Saved Fixed % in paper.json)")
    print("-" * 85)
    print(f"  {'Symbol':<10} | {'Baseline PnL':<15} | {'Dynamic ATR PnL':<16} | {'Baseline WR':<12} | {'ATR WR':<10} | {'Max DD Improvement':<18}")
    print(f"  {'-'*10}-|-{'-'*15}-|-{'-'*16}-|-{'-'*12}-|-{'-'*10}-|-{'-'*18}")
    for sym in SYMBOLS:
        cfg = configs.get(sym)
        if not cfg: continue
        df = load_last_session_bars(sym)
        res_atr = run_simulation(df, cfg["params"], cfg["interval"], use_dynamic_atr=True, use_dynamic_spread=False)
        results_dyn_exits[sym] = res_atr
        base = results_baseline[sym]
        dd_delta = base['max_dd'] - res_atr['max_dd']
        dd_str = f"-{dd_delta:.2f}% DD" if dd_delta > 0 else f"+{abs(dd_delta):.2f}% DD"
        print(f"  {sym:<10} | ${base['pnl']:<14} | ${res_atr['pnl']:<15} | {base['win_rate']:<11}% | {res_atr['win_rate']:<9}% | {dd_str:<18}")

    print("\n" + "-" * 85)
    print(" 3. DYNAMIC TEST 2: REAL VOLATILITY SPREAD SLIPPAGE (vs. Hardcoded 2 bps)")
    print("-" * 85)
    print(f"  {'Symbol':<10} | {'Static (2 bps)':<15} | {'Real Spread PnL':<16} | {'Avg Spread bps':<15} | {'Spread Drag Impact':<20}")
    print(f"  {'-'*10}-|-{'-'*15}-|-{'-'*16}-|-{'-'*15}-|-{'-'*20}")
    for sym in SYMBOLS:
        cfg = configs.get(sym)
        if not cfg: continue
        df = load_last_session_bars(sym)
        res_slip = run_simulation(df, cfg["params"], cfg["interval"], use_dynamic_atr=False, use_dynamic_spread=True)
        results_dyn_slippage[sym] = res_slip
        base = results_baseline[sym]
        drag = base['pnl'] - res_slip['pnl']
        print(f"  {sym:<10} | ${base['pnl']:<14} | ${res_slip['pnl']:<15} | {res_slip['slip_bps']:<14} bps | -${drag:<18.2f}")

    print("\n" + "-" * 85)
    print(" 4. DYNAMIC TEST 3: ADAPTIVE LOOKBACK (vs. Saved Fixed Lookback in paper.json)")
    print("-" * 85)
    print(f"  {'Symbol':<10} | {'Saved Lookback':<15} | {'Baseline PnL':<15} | {'Adaptive Lookback PnL':<22} | {'Lookback Alpha Delta':<20}")
    print(f"  {'-'*10}-|-{'-'*15}-|-{'-'*15}-|-{'-'*22}-|-{'-'*20}")
    for sym in SYMBOLS:
        cfg = configs.get(sym)
        if not cfg: continue
        df = load_last_session_bars(sym)
        res_lb = run_simulation(df, cfg["params"], cfg["interval"], use_dynamic_atr=False, use_dynamic_spread=False, use_dynamic_lookback=True)
        results_dyn_lookback[sym] = res_lb
        base = results_baseline[sym]
        diff = res_lb['pnl'] - base['pnl']
        sign = "+" if diff >= 0 else ""
        print(f"  {sym:<10} | {cfg['params'].get('grid_lookback', 45):<15} | ${base['pnl']:<14} | ${res_lb['pnl']:<21} | {sign}${diff:<19.2f}")

    print("\n" + "-" * 85)
    print(" 5. DYNAMIC TEST 4: DYNAMIC ASSET ALLOCATION IN THE LAST SESSION (2026-11-27)")
    print("-" * 85)
    sorted_baseline = sorted(results_baseline.items(), key=lambda x: x[1]['pnl'], reverse=True)
    best_sym, best_res = sorted_baseline[0]
    worst_sym, worst_res = sorted_baseline[-1]
    avg_pnl = np.mean([r['pnl'] for r in results_baseline.values()])
    
    print(f"  • Best Performing Symbol:  {best_sym} with +${best_res['pnl']:.2f} ({best_res['ret_pct']}%)")
    print(f"  • Worst Performing Symbol: {worst_sym} with -${abs(worst_res['pnl']):.2f} ({worst_res['ret_pct']}%)")
    print(f"  • Static Random Pick (Average across all saved configs): ${avg_pnl:.2f}")
    print(f"  • Dynamic Selection Advantage (allocating to {best_sym}): +${best_res['pnl'] - avg_pnl:.2f}")
    print("=" * 85)

if __name__ == "__main__":
    main()
