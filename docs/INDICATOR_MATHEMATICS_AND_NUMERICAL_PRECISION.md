# Indicator Mathematics & Numerical Precision Architecture

**Project**: `ProperProperProper/MOOMOO-OPEND-TradeStation`  
**Crate**: [`crates/core/src/indicators`](../crates/core/src/indicators)  
**Status**: 100% Pure Rust Mathematical Implementation  

---

## 1. Mathematical Purity Guarantee

All **151 technical indicators** implemented in `tr-core::indicators` are computed using **100% pure, deterministic mathematics** executed directly in native compiled Rust.

* **Zero External Indicator APIs**: No external calculation services, web hooks, or third-party proprietary charting libraries.
* **Direct Vector Processing**: Every indicator takes raw immutable arrays of OHLCV bar series (`open`, `high`, `low`, `close`, `volume`, `ts`) and computes output vectors of identical length.
* **Deterministic & Bit-Reproducible**: Indicator values are fully deterministic mathematical transformations of the input price time-series.
* **Memory & Performance**: Computations operate with amortized $O(1)$ cost per incoming bar ($\approx \text{few milliseconds}$ across 1,000,000+ candlestick bars).

---

## 2. Core Mathematical Formulations

### A. Moving Averages & Digital Filters ([`ma.rs`](../crates/core/src/indicators/ma.rs))

#### 1. Simple Moving Average (SMA)
For a lookback period $n$ over price series $x$:
$$\text{SMA}_t = \frac{1}{n} \sum_{i=0}^{n-1} x_{t-i}$$

#### 2. Exponential Moving Average (EMA)
Seeded by the SMA of the first $n$ finite bars, then updated recursively:
$$\text{EMA}_t = \alpha \cdot x_t + (1 - \alpha) \cdot \text{EMA}_{t-1}, \quad \text{where } \alpha = \frac{2}{n + 1}$$

#### 3. Wilder's Smoothing (RMA / SMMA)
Used internally as the foundation for RSI, ATR, and ADX:
$$\alpha = \frac{1}{n}, \quad \text{RMA}_t = \frac{1}{n} x_t + \left(1 - \frac{1}{n}\right) \text{RMA}_{t-1}$$

#### 4. Linearly Weighted Moving Average (WMA)
Linear weight $w_k = k$ assigning highest weight $n$ to the most recent bar:
$$\text{WMA}_t = \frac{\sum_{k=1}^n k \cdot x_{t - n + k}}{\frac{n(n + 1)}{2}}$$
*Implemented in $O(1)$ amortized time per bar using a double sliding-window accumulator.*

#### 5. Hull Moving Average (HMA)
Reduces lag while maintaining smoothness by taking difference of weighted averages:
$$\text{HMA}_t = \text{WMA}\left(2 \cdot \text{WMA}\left(x, \lfloor n/2 \rfloor\right) - \text{WMA}(x, n), \; \lfloor\sqrt{n}\rfloor\right)$$

#### 6. Double & Triple EMA (DEMA / TEMA)
Eliminates lag through exponential combinations:
$$\text{DEMA}_t = 2 \cdot \text{EMA}_n(x) - \text{EMA}_n\left(\text{EMA}_n(x)\right)$$
$$\text{TEMA}_t = 3 \cdot \text{EMA}_n - 3 \cdot \text{EMA}_n(\text{EMA}_n) + \text{EMA}_n(\text{EMA}_n(\text{EMA}_n))$$

#### 7. Kaufman Adaptive Moving Average (KAMA)
Dynamically adjusts smoothing speed based on market efficiency:
* **Efficiency Ratio (ER)**:
  $$\text{ER}_t = \frac{|x_t - x_{t-n}|}{\sum_{i=0}^{n-1} |x_{t-i} - x_{t-i-1}|}$$
* **Smoothing Constant ($c$)**:
  $$c_t = \left(\text{ER}_t \cdot \left(\frac{2}{2+1} - \frac{2}{30+1}\right) + \frac{2}{30+1}\right)^2$$
* **Update**:
  $$\text{KAMA}_t = \text{KAMA}_{t-1} + c_t \cdot (x_t - \text{KAMA}_{t-1})$$

#### 8. John Ehlers Digital Signal Processing (DSP) Filters
Higher-order digital filters with exact pole placement:
* **SuperSmoother (2-Pole Filter)**:
  $$a_1 = \exp\left(-\frac{\sqrt{2}\pi}{n}\right), \quad b_1 = 2 a_1 \cos\left(\frac{\sqrt{2}\pi}{n}\right)$$
  $$c_1 = 1 - b_1 + a_1^2, \quad c_2 = b_1, \quad c_3 = -a_1^2$$
  $$y_t = c_1 \cdot \frac{x_t + x_{t-1}}{2} + c_2 \cdot y_{t-1} + c_3 \cdot y_{t-2}$$

---

### B. Statistical Dispersion & Volatility ([`stats.rs`](../crates/core/src/indicators/stats.rs) & [`volatility.rs`](../crates/core/src/indicators/volatility.rs))

#### 1. Population Standard Deviation ($\sigma$)
Computed with bounded float precision:
$$\sigma_t = \sqrt{\frac{\sum_{i=0}^{n-1} x_{t-i}^2 - \frac{\left(\sum_{i=0}^{n-1} x_{t-i}\right)^2}{n}}{n}}$$

#### 2. Bollinger Bands (BB)
$$\text{Basis}_t = \text{SMA}_n(x)$$
$$\text{Upper}_t = \text{Basis}_t + k \cdot \sigma_t, \quad \text{Lower}_t = \text{Basis}_t - k \cdot \sigma_t$$

#### 3. Average True Range (ATR)
* **True Range (TR)**:
  $$\text{TR}_t = \max\left(H_t - L_t, \; |H_t - C_{t-1}|, \; |L_t - C_{t-1}|\right)$$
* **ATR**:
  $$\text{ATR}_t = \text{RMA}_n(\text{TR})$$

#### 4. Keltner Channels
$$\text{Center}_t = \text{EMA}_n(\text{Close})$$
$$\text{Upper}_t = \text{Center}_t + m \cdot \text{ATR}_n, \quad \text{Lower}_t = \text{Center}_t - m \cdot \text{ATR}_n$$

#### 5. Donchian Channels
$$\text{Upper}_t = \max_{0 \le i < n}(H_{t-i}), \quad \text{Lower}_t = \min_{0 \le i < n}(L_{t-i}), \quad \text{Mid}_t = \frac{\text{Upper}_t + \text{Lower}_t}{2}$$

---

### C. Momentum & Oscillators ([`momentum.rs`](../crates/core/src/indicators/momentum.rs))

#### 1. Relative Strength Index (RSI)
* Price change: $\Delta_t = C_t - C_{t-1}$
* Positive gains: $U_t = \max(\Delta_t, 0)$
* Negative losses: $D_t = \max(-\Delta_t, 0)$
* Smoothed values: $\bar{U}_t = \text{RMA}_n(U), \; \bar{D}_t = \text{RMA}_n(D)$
* Relative Strength and RSI:
  $$\text{RS}_t = \frac{\bar{U}_t}{\bar{D}_t}, \quad \text{RSI}_t = 100 - \frac{100}{1 + \text{RS}_t}$$

#### 2. Moving Average Convergence Divergence (MACD)
$$\text{MACD}_t = \text{EMA}_{\text{fast}}(C) - \text{EMA}_{\text{slow}}(C)$$
$$\text{Signal}_t = \text{EMA}_{\text{signal}}(\text{MACD})$$
$$\text{Histogram}_t = \text{MACD}_t - \text{Signal}_t$$

#### 3. Stochastic Oscillator (%K, %D)
$$\%K_t = 100 \cdot \frac{C_t - \min_n(L)}{\max_n(H) - \min_n(L)}$$
$$\%D_t = \text{SMA}_k(\%K)$$

#### 4. Commodity Channel Index (CCI)
* Typical price: $M_t = \frac{H_t + L_t + C_t}{3}$
* Mean Absolute Deviation (MAD):
  $$\text{MAD}_t = \frac{1}{n} \sum_{i=0}^{n-1} |M_{t-i} - \text{SMA}_n(M)|$$
* CCI:
  $$\text{CCI}_t = \frac{M_t - \text{SMA}_n(M)}{0.015 \cdot \text{MAD}_t}$$

---

### D. Volume Dynamics & Session Levels ([`volume.rs`](../crates/core/src/indicators/volume.rs) & [`levels.rs`](../crates/core/src/indicators/levels.rs))

#### 1. Volume-Weighted Average Price (VWAP)
Calculated continuously with deterministic session boundary resets:
$$\text{VWAP}_t = \frac{\sum_{\tau = \text{session\_start}}^t P_{\text{typical},\tau} \cdot V_\tau}{\sum_{\tau = \text{session\_start}}^t V_\tau}, \quad \text{where } P_{\text{typical}} = \frac{H + L + C}{3}$$
*Standard deviations $\pm 1\sigma, \pm 2\sigma, \pm 3\sigma$ are computed from volume-weighted variance.*

#### 2. On-Balance Volume (OBV)
$$\text{OBV}_t = \begin{cases} 
\text{OBV}_{t-1} + V_t & \text{if } C_t > C_{t-1} \\
\text{OBV}_{t-1} - V_t & \text{if } C_t < C_{t-1} \\
\text{OBV}_{t-1} & \text{if } C_t = C_{t-1}
\end{cases}$$

#### 3. Chaikin Money Flow (CMF)
* Money Flow Multiplier:
  $$\text{MFM}_t = \frac{(C_t - L_t) - (H_t - C_t)}{H_t - L_t}$$
* Money Flow Volume: $\text{MFV}_t = \text{MFM}_t \cdot V_t$
* CMF:
  $$\text{CMF}_t = \frac{\sum_{i=0}^{n-1} \text{MFV}_{t-i}}{\sum_{i=0}^{n-1} V_{t-i}}$$

---

## 3. High-Performance Numerical Architecture

### A. Catastrophic Cancellation Defense (Zero-Offset Centering)
When calculating the sum of squares ($\sum x^2$) for standard deviation on instruments trading at large nominal prices (e.g. BTC at $\$90,000$, S&P 500 at $5,900$), standard IEEE 754 64-bit floats lose precision due to catastrophic cancellation:
$$(x_i)^2 \approx 8.1 \times 10^9$$
Subtracting large floating-point numbers leaves only a few bits of precision for variance.

**Our Engine's Solution**:
The engine anchors the calculation to a pivot $r$ (the first finite price in the window):
$$y_i = x_i - r$$
All sums ($\sum y_i$, $\sum y_i^2$, $\sum k \cdot y_i$) are calculated over the localized deviations $y_i \approx 0$, preserving all 53 bits of floating-point mantissa precision.

### B. Floating-Point Drift Elimination
Sliding-window algorithms that add the newest point and subtract the oldest point can accumulate floating-point rounding drift over millions of bars.
* **Periodic Re-accumulation**: The sliding accumulators are recalculated exactly once every $n$ bars from scratch, bounding any potential drift to within machine epsilon ($\approx 2.2 \times 10^{-16}$).

### C. Amortized $O(1)$ Time Complexity
Regardless of whether the lookback period is $n=5$ or $n=500$, evaluating the next bar takes constant $O(1)$ time and memory:
* Oldest elements slide out in $O(1)$.
* Newest elements slide in in $O(1)$.
* Memory allocation is done upfront with `Vec::with_capacity(bars.len())`.

### D. Strict NaN Propagation & Warm-Up Alignment
* Every output vector matches the length of the input series exactly.
* Bars prior to window maturity return `f64::NAN`.
* Any NaN gap in input data (e.g. market halt or missing tick) only blanks the windows containing it, resuming automatically once valid data returns.

---

## 4. Indicator Category Breakdown (151 Total)

| Category | Count | Key Mathematical Alphas |
|---|:---:|---|
| **Moving Averages & Smoothing** | 28 | SMA, EMA, WMA, HMA, DEMA, TEMA, KAMA, ALMA, RMA, Ehlers SuperSmoother, Butterworth |
| **Momentum & Oscillators** | 34 | RSI, MACD, Stochastics (%K/%D), CCI, Williams %R, TSI, CMO, Awesome, Accelerator, Ultimate |
| **Volatility & Channels** | 22 | Bollinger Bands, Keltner, Donchian, ATR, Chaikin Volatility, Historical Volatility, Mass Index |
| **Volume & Order Flow** | 18 | VWAP (Session/Anchored), OBV, CMF, MFI, Volume Oscillator, Accumulation/Distribution |
| **Trend & Directional** | 26 | ADX/DMI, Parabolic SAR, Aroon, Supertrend, Vortex, Linear Regression Trend, Detrended Price |
| **Support, Resistance & Levels** | 13 | Pivot Points (Classic, Camarilla, Woodie, Fibonacci), High/Low Rolling Levels, Fib Retracements |
| **Statistical Analysis** | 10 | Rolling Variance, Standard Error, Pearson Correlation, Covariance, Z-Score |
