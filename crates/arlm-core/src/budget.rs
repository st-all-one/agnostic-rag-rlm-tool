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
pub struct CostBudget {
    spent_bits: AtomicU64,
    max: f64,
}

impl CostBudget {
    #[must_use]
    pub fn new(max: f64) -> Self {
        Self {
            spent_bits: AtomicU64::new(0),
            max,
        }
    }

    pub fn spend(&self, amount: f64) {
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
    #[must_use]
    pub fn remaining(&self) -> f64 {
        let spent_bits = self.spent_bits.load(Ordering::Relaxed);
        let spent = f64::from_bits(spent_bits);
        (self.max - spent).max(0.0)
    }

    /// Check whether the USD budget is exhausted.
    ///
    /// # Errors
    ///
    /// Returns an error if no USD budget remains.
    pub fn check(&self) -> anyhow::Result<()> {
        if self.remaining() <= 0.0 {
            anyhow::bail!("USD budget exhausted");
        }
        Ok(())
    }
}

/// Token budget.
#[derive(Debug)]
pub struct TokenBudget {
    used: AtomicU32,
    max: u32,
}

impl TokenBudget {
    #[must_use]
    pub fn new(max: u64) -> Self {
        Self {
            used: AtomicU32::new(0),
            #[allow(clippy::cast_possible_truncation)]
            max: max.min(u64::from(u32::MAX)) as u32,
        }
    }

    pub fn add(&self, tokens: u32) {
        self.used.fetch_add(tokens, Ordering::Relaxed);
    }

    #[must_use]
    pub fn remaining(&self) -> u32 {
        self.max.saturating_sub(self.used.load(Ordering::Relaxed))
    }

    /// Check whether the token budget is exhausted.
    ///
    /// # Errors
    ///
    /// Returns an error if no token budget remains.
    pub fn check(&self) -> anyhow::Result<()> {
        if self.remaining() == 0 {
            anyhow::bail!("token budget exhausted");
        }
        Ok(())
    }
}

/// Error budget.
#[derive(Debug)]
pub struct ErrorBudget {
    count: AtomicU32,
    max: u32,
}

impl ErrorBudget {
    #[must_use]
    pub fn new(max: u32) -> Self {
        Self {
            count: AtomicU32::new(0),
            max,
        }
    }

    pub fn add_one(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    #[must_use]
    pub fn remaining(&self) -> u32 {
        self.max.saturating_sub(self.count.load(Ordering::Relaxed))
    }

    /// Check whether the error budget is exhausted.
    ///
    /// # Errors
    ///
    /// Returns an error if no error budget remains.
    pub fn check(&self) -> anyhow::Result<()> {
        if self.remaining() == 0 {
            anyhow::bail!("error threshold reached");
        }
        Ok(())
    }
}

/// Time budget.
#[derive(Debug)]
pub struct TimeBudget {
    start: Instant,
    timeout: Duration,
}

impl TimeBudget {
    #[must_use]
    pub fn new(timeout_ms: u64) -> Self {
        Self {
            start: Instant::now(),
            timeout: Duration::from_millis(timeout_ms),
        }
    }

    #[must_use]
    pub fn remaining_ms(&self) -> u64 {
        let elapsed = self.start.elapsed();
        if elapsed >= self.timeout {
            0
        } else {
            #[allow(clippy::cast_possible_truncation)]
            let remaining =
                (self.timeout.checked_sub(elapsed).unwrap_or_default()).as_millis() as u64;
            remaining
        }
    }

    /// Check whether the time budget is exhausted.
    ///
    /// # Errors
    ///
    /// Returns an error if the timeout has elapsed.
    pub fn check(&self) -> anyhow::Result<()> {
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
