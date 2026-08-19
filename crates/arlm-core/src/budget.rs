use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use arlm_llm::UsageSummary;
use arlm_llm::pricing::PricingTable;

/// Budget tracker for USD cost, token count, errors, and time.
#[derive(Debug)]
pub struct RunBudget {
    cost: CostBudget,
    tokens: TokenBudget,
    errors: ErrorBudget,
    time: TimeBudget,
    pricing: PricingTable,
}

impl RunBudget {
    #[must_use]
    pub fn new(max_usd: f64, max_tokens: u64, max_errors: u32, timeout_ms: u64) -> Self {
        Self {
            cost: CostBudget::new(max_usd),
            tokens: TokenBudget::new(max_tokens),
            errors: ErrorBudget::new(max_errors),
            time: TimeBudget::new(timeout_ms),
            pricing: PricingTable::new(),
        }
    }

    /// Check if the budget is exceeded. Returns `Err` if any limit is hit.
    ///
    /// # Errors
    ///
    /// Returns an error describing which budget limit was exceeded.
    pub fn check(&self) -> anyhow::Result<()> {
        self.cost.check()?;
        self.tokens.check()?;
        self.errors.check()?;
        self.time.check()?;
        Ok(())
    }

    /// Record an LLM call's usage.
    pub fn record_call(&self, model: &str, usage: &UsageSummary) {
        let cost = self.pricing.estimate_cost(model, usage);
        self.cost.spend(cost);
        self.tokens.add(usage.total_tokens);
    }

    /// Record an error.
    pub fn record_error(&self) {
        self.errors.add_one();
    }

    /// Get a summary of remaining budget.
    #[must_use]
    pub fn summary(&self) -> BudgetSummary {
        BudgetSummary {
            budget_remaining: self.cost.remaining(),
            tokens_remaining: self.tokens.remaining(),
            errors_remaining: self.errors.remaining(),
            time_remaining_ms: self.time.remaining_ms(),
        }
    }
}

/// Cost budget in USD using atomic CAS loop for correct f64 addition.
#[derive(Debug)]
struct CostBudget {
    spent_bits: AtomicU64,
    max: f64,
}

impl CostBudget {
    fn new(max: f64) -> Self {
        Self {
            spent_bits: AtomicU64::new(0),
            max,
        }
    }

    fn spend(&self, amount: f64) {
        loop {
            let current_bits = self.spent_bits.load(Ordering::Relaxed);
            let current = f64::from_bits(current_bits);
            let new_val = current + amount;
            let new_bits = new_val.to_bits();
            if self
                .spent_bits
                .compare_exchange_weak(current_bits, new_bits, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn remaining(&self) -> f64 {
        let spent_bits = self.spent_bits.load(Ordering::Relaxed);
        let spent = f64::from_bits(spent_bits);
        (self.max - spent).max(0.0)
    }

    fn check(&self) -> anyhow::Result<()> {
        if self.remaining() <= 0.0 {
            anyhow::bail!("USD budget exhausted");
        }
        Ok(())
    }
}

/// Token budget.
#[derive(Debug)]
struct TokenBudget {
    used: AtomicU32,
    max: u32,
}

impl TokenBudget {
    fn new(max: u64) -> Self {
        Self {
            used: AtomicU32::new(0),
            #[allow(clippy::cast_possible_truncation)]
            max: max.min(u64::from(u32::MAX)) as u32,
        }
    }

    fn add(&self, tokens: u32) {
        self.used.fetch_add(tokens, Ordering::Relaxed);
    }

    fn remaining(&self) -> u32 {
        self.max.saturating_sub(self.used.load(Ordering::Relaxed))
    }

    fn check(&self) -> anyhow::Result<()> {
        if self.remaining() == 0 {
            anyhow::bail!("token budget exhausted");
        }
        Ok(())
    }
}

/// Error budget.
#[derive(Debug)]
struct ErrorBudget {
    count: AtomicU32,
    max: u32,
}

impl ErrorBudget {
    fn new(max: u32) -> Self {
        Self {
            count: AtomicU32::new(0),
            max,
        }
    }

    fn add_one(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    fn remaining(&self) -> u32 {
        self.max.saturating_sub(self.count.load(Ordering::Relaxed))
    }

    fn check(&self) -> anyhow::Result<()> {
        if self.remaining() == 0 {
            anyhow::bail!("error threshold reached");
        }
        Ok(())
    }
}

/// Time budget.
#[derive(Debug)]
struct TimeBudget {
    start: Instant,
    timeout: Duration,
}

impl TimeBudget {
    fn new(timeout_ms: u64) -> Self {
        Self {
            start: Instant::now(),
            timeout: Duration::from_millis(timeout_ms),
        }
    }

    fn remaining_ms(&self) -> u64 {
        let elapsed = self.start.elapsed();
        if elapsed >= self.timeout {
            0
        } else {
            #[allow(clippy::cast_possible_truncation)]
            #[allow(clippy::cast_possible_truncation)]
            let remaining =
                (self.timeout.checked_sub(elapsed).unwrap_or_default()).as_millis() as u64;
            remaining
        }
    }

    fn check(&self) -> anyhow::Result<()> {
        if self.start.elapsed() >= self.timeout {
            anyhow::bail!("timeout reached");
        }
        Ok(())
    }
}

/// Summary of remaining budget.
#[derive(Debug, Clone)]
pub struct BudgetSummary {
    pub budget_remaining: f64,
    pub tokens_remaining: u32,
    pub errors_remaining: u32,
    pub time_remaining_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_budget_new() {
        let budget = RunBudget::new(1.0, 100_000, 5, 60_000);
        let summary = budget.summary();
        assert!((summary.budget_remaining - 1.0).abs() < f64::EPSILON);
        assert_eq!(summary.tokens_remaining, 100_000);
        assert_eq!(summary.errors_remaining, 5);
        assert!(summary.time_remaining_ms > 0);
    }

    #[test]
    fn test_run_budget_check_passes() {
        let budget = RunBudget::new(1.0, 100_000, 5, 60_000);
        assert!(budget.check().is_ok());
    }

    #[test]
    fn test_run_budget_check_fails_on_timeout() {
        let budget = RunBudget::new(1.0, 100_000, 5, 0);
        std::thread::sleep(Duration::from_millis(10));
        assert!(budget.check().is_err());
    }

    #[test]
    fn test_run_budget_record_call() {
        let budget = RunBudget::new(1.0, 100_000, 5, 60_000);
        let usage = UsageSummary {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
        };
        budget.record_call("gpt-4o", &usage);
        let summary = budget.summary();
        assert!(summary.tokens_remaining < 100_000);
    }

    #[test]
    fn test_run_budget_record_error() {
        let budget = RunBudget::new(1.0, 100_000, 2, 60_000);
        budget.record_error();
        assert_eq!(budget.summary().errors_remaining, 1);
        budget.record_error();
        assert_eq!(budget.summary().errors_remaining, 0);
        assert!(budget.check().is_err());
    }

    #[test]
    fn test_cost_budget_spend() {
        let cb = CostBudget::new(1.0);
        assert!((cb.remaining() - 1.0).abs() < f64::EPSILON);
        cb.spend(0.3);
        assert!((cb.remaining() - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_token_budget() {
        let tb = TokenBudget::new(1000);
        assert_eq!(tb.remaining(), 1000);
        tb.add(300);
        assert_eq!(tb.remaining(), 700);
        tb.add(800);
        assert_eq!(tb.remaining(), 0);
        assert!(tb.check().is_err());
    }

    #[test]
    fn test_error_budget() {
        let eb = ErrorBudget::new(2);
        assert_eq!(eb.remaining(), 2);
        eb.add_one();
        assert_eq!(eb.remaining(), 1);
        eb.add_one();
        assert_eq!(eb.remaining(), 0);
        assert!(eb.check().is_err());
    }

    #[test]
    fn test_time_budget_remaining() {
        let tb = TimeBudget::new(60_000);
        assert!(tb.remaining_ms() > 50_000);
    }

    #[test]
    fn test_time_budget_expired() {
        let tb = TimeBudget::new(0);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(tb.remaining_ms(), 0);
        assert!(tb.check().is_err());
    }
}
