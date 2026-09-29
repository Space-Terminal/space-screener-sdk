use std::collections::VecDeque;

pub const fn secs(n: i64) -> i64 {
    n * 1_000
}

pub const fn mins(n: i64) -> i64 {
    n * 60_000
}

pub const fn hours(n: i64) -> i64 {
    n * 3_600_000
}

/// Time series of `(ts_ms, value)` that keeps just enough history to answer
/// "what was the value `horizon_ms` ago".
#[derive(Debug, Clone, Default)]
pub struct Series {
    points: VecDeque<(i64, f64)>,
    horizon_ms: i64,
}

impl Series {
    pub fn new(horizon_ms: i64) -> Self {
        Self {
            points: VecDeque::new(),
            horizon_ms,
        }
    }

    /// Out-of-order points are dropped; a point with the same timestamp replaces the last one.
    pub fn push(&mut self, ts_ms: i64, value: f64) {
        match self.points.back_mut() {
            Some(last) if ts_ms < last.0 => return,
            Some(last) if ts_ms == last.0 => last.1 = value,
            _ => self.points.push_back((ts_ms, value)),
        }
        let cutoff = ts_ms - self.horizon_ms;
        // Keep one point at or before the cutoff so `value_at(now - horizon)` still resolves.
        while self.points.len() >= 2 && self.points[1].0 <= cutoff {
            self.points.pop_front();
        }
    }

    pub fn last(&self) -> Option<f64> {
        self.points.back().map(|p| p.1)
    }

    pub fn last_ts(&self) -> Option<i64> {
        self.points.back().map(|p| p.0)
    }

    /// Latest value at or before `ts_ms`; `None` when history starts later.
    pub fn value_at(&self, ts_ms: i64) -> Option<f64> {
        let idx = self.points.partition_point(|p| p.0 <= ts_ms);
        idx.checked_sub(1).map(|i| self.points[i].1)
    }

    pub fn change(&self, now_ms: i64, ago_ms: i64) -> Option<f64> {
        Some(self.last()? - self.value_at(now_ms - ago_ms)?)
    }

    pub fn change_pct(&self, now_ms: i64, ago_ms: i64) -> Option<f64> {
        let past = self.value_at(now_ms - ago_ms)?;
        if past == 0.0 {
            return None;
        }
        Some((self.last()? - past) / past * 100.0)
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_needs_enough_history() {
        let mut s = Series::new(mins(15));
        s.push(mins(0), 100.0);
        s.push(mins(4), 104.0);
        assert_eq!(s.change_pct(mins(4), mins(5)), None);
        s.push(mins(6), 110.0);
        assert_eq!(s.change_pct(mins(6), mins(5)), Some(10.0));
        assert_eq!(s.change(mins(6), mins(5)), Some(10.0));
    }

    #[test]
    fn trims_to_horizon_but_keeps_anchor() {
        let mut s = Series::new(mins(5));
        for m in 0..=20 {
            s.push(mins(m), m as f64);
        }
        assert_eq!(s.value_at(mins(15)), Some(15.0));
        assert_eq!(s.value_at(mins(14)), None);
        assert_eq!(s.len(), 6);
    }

    #[test]
    fn ignores_out_of_order_and_replaces_same_ts() {
        let mut s = Series::new(mins(5));
        s.push(10, 1.0);
        s.push(5, 2.0);
        s.push(10, 3.0);
        assert_eq!(s.len(), 1);
        assert_eq!(s.last(), Some(3.0));
    }
}
