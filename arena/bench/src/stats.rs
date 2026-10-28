//! Running mean/variance and mbb/game reporting.

#[derive(Clone, Copy, Debug, Default)]
pub struct Acc {
    pub n: u64,
    pub sum: f64,
    pub sumsq: f64,
}

impl Acc {
    pub fn add(&mut self, x: f64) {
        self.n += 1;
        self.sum += x;
        self.sumsq += x * x;
    }
    pub fn mean(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.sum / self.n as f64
        }
    }
    /// Unbiased sample variance.
    pub fn var(&self) -> f64 {
        if self.n < 2 {
            return 0.0;
        }
        let n = self.n as f64;
        ((self.sumsq - self.sum * self.sum / n) / (n - 1.0)).max(0.0)
    }
    pub fn se(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            (self.var() / self.n as f64).sqrt()
        }
    }
}

/// Point estimate in mbb/game (milli big blinds per hand) with standard error.
#[derive(Clone, Copy, Debug, Default)]
pub struct Est {
    pub mbb: f64,
    pub se: f64,
    pub n: u64,
}

impl Est {
    /// `acc` holds chip payoffs; `bb` is the big blind in chips.
    pub fn from_chips(acc: &Acc, bb: f64) -> Est {
        Est {
            mbb: acc.mean() / bb * 1000.0,
            se: acc.se() / bb * 1000.0,
            n: acc.n,
        }
    }
    pub fn ci95(&self) -> (f64, f64) {
        (self.mbb - 1.96 * self.se, self.mbb + 1.96 * self.se)
    }
}

impl std::fmt::Display for Est {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:+.1} ± {:.1}", self.mbb, 1.96 * self.se)
    }
}
