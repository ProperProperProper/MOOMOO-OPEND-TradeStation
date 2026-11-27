use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Bar;

/// Chart intervals strictly restricted to 1m, 2m, 3m, 4m, 5m, 6m, 7m, 8m, 9m.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum Timeframe {
    #[default]
    M1,
    M2,
    M3,
    M4,
    M5,
    M6,
    M7,
    M8,
    M9,
}

impl Timeframe {
    pub const ALL: [Timeframe; 9] = [
        Timeframe::M1,
        Timeframe::M2,
        Timeframe::M3,
        Timeframe::M4,
        Timeframe::M5,
        Timeframe::M6,
        Timeframe::M7,
        Timeframe::M8,
        Timeframe::M9,
    ];

    pub fn minutes(self) -> i64 {
        match self {
            Timeframe::M1 => 1,
            Timeframe::M2 => 2,
            Timeframe::M3 => 3,
            Timeframe::M4 => 4,
            Timeframe::M5 => 5,
            Timeframe::M6 => 6,
            Timeframe::M7 => 7,
            Timeframe::M8 => 8,
            Timeframe::M9 => 9,
        }
    }

    /// Nominal duration in seconds.
    pub fn seconds(self) -> i64 {
        self.minutes() * 60
    }

    pub fn label(self) -> &'static str {
        match self {
            Timeframe::M1 => "1m",
            Timeframe::M2 => "2m",
            Timeframe::M3 => "3m",
            Timeframe::M4 => "4m",
            Timeframe::M5 => "5m",
            Timeframe::M6 => "6m",
            Timeframe::M7 => "7m",
            Timeframe::M8 => "8m",
            Timeframe::M9 => "9m",
        }
    }

    pub fn is_intraday(self) -> bool {
        true
    }

    /// Start of the bucket containing `ts` (in UTC seconds).
    pub fn bucket_start(self, ts: i64) -> i64 {
        ts.div_euclid(self.seconds()) * self.seconds()
    }

    /// Aggregate 1-minute bars into this timeframe bucket.
    pub fn resample(self, bars: &[Bar]) -> Vec<Bar> {
        if self == Timeframe::M1 {
            return bars.to_vec();
        }
        let mut out: Vec<Bar> = Vec::with_capacity(bars.len() / (self.minutes() as usize) + 1);
        for b in bars {
            let start = self.bucket_start(b.ts);
            match out.last_mut() {
                Some(cur) if cur.ts == start => {
                    cur.high = cur.high.max(b.high);
                    cur.low = cur.low.min(b.low);
                    cur.close = b.close;
                    cur.volume += b.volume;
                }
                _ => out.push(Bar { ts: start, ..*b }),
            }
        }
        out
    }
}

impl fmt::Display for Timeframe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl FromStr for Timeframe {
    type Err = String;

    /// Accepts 1..9 interval spellings: `1`, `1m`, `2`, `2m`, ... `9`, `9m`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim().to_ascii_lowercase();
        Ok(match t.as_str() {
            "1" | "1m" | "1min" => Timeframe::M1,
            "2" | "2m" | "2min" => Timeframe::M2,
            "3" | "3m" | "3min" => Timeframe::M3,
            "4" | "4m" | "4min" => Timeframe::M4,
            "5" | "5m" | "5min" => Timeframe::M5,
            "6" | "6m" | "6min" => Timeframe::M6,
            "7" | "7m" | "7min" => Timeframe::M7,
            "8" | "8m" | "8min" => Timeframe::M8,
            "9" | "9m" | "9min" => Timeframe::M9,
            // Fallbacks
            "10" | "10m" => Timeframe::M9,
            "15" | "15m" => Timeframe::M9,
            "30" | "30m" => Timeframe::M9,
            "60" | "1h" | "h" => Timeframe::M9,
            "d" | "1d" | "day" => Timeframe::M1,
            "w" | "1w" | "week" => Timeframe::M1,
            "month" | "1mo" | "m" => Timeframe::M1,
            _ => return Err(format!("unknown timeframe '{s}': only intervals 1..9 (1m..9m) are available")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_intervals() {
        assert_eq!("1m".parse::<Timeframe>().unwrap(), Timeframe::M1);
        assert_eq!("2m".parse::<Timeframe>().unwrap(), Timeframe::M2);
        assert_eq!("3m".parse::<Timeframe>().unwrap(), Timeframe::M3);
        assert_eq!("4m".parse::<Timeframe>().unwrap(), Timeframe::M4);
        assert_eq!("5m".parse::<Timeframe>().unwrap(), Timeframe::M5);
        assert_eq!("6m".parse::<Timeframe>().unwrap(), Timeframe::M6);
        assert_eq!("7m".parse::<Timeframe>().unwrap(), Timeframe::M7);
        assert_eq!("8m".parse::<Timeframe>().unwrap(), Timeframe::M8);
        assert_eq!("9m".parse::<Timeframe>().unwrap(), Timeframe::M9);
        assert!("12x".parse::<Timeframe>().is_err());
    }

    #[test]
    fn interval_buckets() {
        assert_eq!(Timeframe::M3.bucket_start(185), 180);
        assert_eq!(Timeframe::M5.bucket_start(305), 300);
        assert_eq!(Timeframe::M7.bucket_start(425), 420);
        assert_eq!(Timeframe::M9.bucket_start(545), 540);
    }

    #[test]
    fn resample_1min_to_3min() {
        let bars: Vec<Bar> =
            (0..9).map(|i| Bar::new(i * 60, i as f64, i as f64 + 1.0, i as f64 - 1.0, i as f64 + 0.5, 10.0)).collect();
        let resampled = Timeframe::M3.resample(&bars);
        assert_eq!(resampled.len(), 3);
        assert_eq!(resampled[0].ts, 0);
        assert_eq!(resampled[0].open, 0.0);
        assert_eq!(resampled[0].close, 2.5);
        assert_eq!(resampled[0].volume, 30.0);
        assert_eq!(resampled[1].ts, 180);
    }
}
