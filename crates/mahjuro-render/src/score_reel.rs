//! Odometer-style score reel animation state.
//!
//! Gameplay draws the score in **2D**; this module is still driven for cascade
//! timing (`set_score`, `is_animating`) so HUD updates can wait for digit rolls.

use std::time::Instant;

const SPIN_DURATION: f32 = 0.22;
const CASCADE_DELAY: f32 = 0.04;

#[derive(Clone, Debug)]
struct ReelColumn {
    current: u8,
    prev: u8,
    spin_start: Option<Instant>,
}

impl ReelColumn {
    fn new(digit: u8) -> Self {
        Self {
            current: digit,
            prev: digit,
            spin_start: None,
        }
    }

    fn spin_to(&mut self, new_digit: u8, start: Instant) {
        if new_digit == self.current {
            return;
        }
        self.prev = self.current;
        self.current = new_digit;
        self.spin_start = Some(start);
    }

    fn is_spinning(&self, now: Instant) -> bool {
        self.spin_start
            .map(|t| now.saturating_duration_since(t).as_secs_f32() < SPIN_DURATION)
            .unwrap_or(false)
    }
}

pub struct ScoreReel {
    columns: Vec<ReelColumn>,
    displayed: u64,
    min_columns: usize,
}

impl ScoreReel {
    pub fn new() -> Self {
        Self {
            columns: vec![ReelColumn::new(0)],
            displayed: 0,
            min_columns: 1,
        }
    }

    pub fn reset_for_target(&mut self, target: u64) {
        let n = digits_for(target).max(1);
        self.min_columns = n;
        self.columns = (0..n).map(|_| ReelColumn::new(0)).collect();
        self.displayed = 0;
    }

    pub fn set_score(&mut self, score: u64, now: Instant) {
        let needed = digits_for(score).max(self.min_columns);
        if needed > self.columns.len() {
            let extra = needed - self.columns.len();
            for _ in 0..extra {
                self.columns.push(ReelColumn::new(0));
            }
        }

        if score == self.displayed {
            return;
        }

        let new_digits = decompose(score, self.columns.len());
        let mut carry_delay = 0.0_f32;
        for (column, &new_d) in self.columns.iter_mut().zip(new_digits.iter()) {
            if new_d != column.current {
                let spin_at = now + std::time::Duration::from_secs_f32(carry_delay);
                column.spin_to(new_d, spin_at);
                carry_delay += CASCADE_DELAY;
            }
        }

        self.displayed = score;
    }

    pub fn snap(&mut self, score: u64) {
        if score == self.displayed && !self.columns.is_empty() {
            return;
        }
        let n = digits_for(score).max(self.min_columns).max(1);
        let digits = decompose(score, n);
        self.columns = digits.iter().map(|&d| ReelColumn::new(d)).collect();
        self.displayed = score;
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.columns.iter().any(|c| c.is_spinning(now))
    }
}

impl Default for ScoreReel {
    fn default() -> Self {
        Self::new()
    }
}

fn decompose(score: u64, n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n);
    let mut s = score;
    for _ in 0..n {
        v.push((s % 10) as u8);
        s /= 10;
    }
    v
}

fn digits_for(score: u64) -> usize {
    if score == 0 {
        1
    } else {
        score.ilog10() as usize + 1
    }
}
