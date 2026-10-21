//! S3A Ledger & Budget Agent
//! Epistemic Invariant: The ledger records cost and meters budgets.
//! The ledger never approves or rejects code; code judgments are human-only.
//! Paid actions at 100% cap receive "deferred_budget" status and queue.
//! Free actions (planning, triage, log review, PR descriptions) never block.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionKind {
    Llm,
    Sandbox,
    Browser,
    R2,
    Free,
}

impl ActionKind {
    pub fn is_paid(&self) -> bool {
        !matches!(self, ActionKind::Free)
    }

    pub fn is_free(&self) -> bool {
        matches!(self, ActionKind::Free)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRates {
    pub cost_per_llm_token: f64,
    pub cost_per_sandbox_second: f64,
    pub cost_per_browser_second: f64,
    pub cost_per_r2_operation: f64,
}

impl Default for CostRates {
    fn default() -> Self {
        Self {
            cost_per_llm_token: 0.000002,    // $2.00 / 1M tokens
            cost_per_sandbox_second: 0.0001, // $0.006 / min
            cost_per_browser_second: 0.0002, // $0.012 / min
            cost_per_r2_operation: 0.00005,  // $0.05 / 1000 ops
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetCapConfig {
    pub daily_cap_usd: f64,
    pub warning_threshold_pct: f64, // e.g. 0.80 for 80%
    pub rates: CostRates,
}

impl Default for BudgetCapConfig {
    fn default() -> Self {
        Self {
            daily_cap_usd: 1.00,
            warning_threshold_pct: 0.80,
            rates: CostRates::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: String,
    pub ts: u64,
    pub task_id: String,
    pub agent_id: String,
    pub kind: ActionKind,
    pub units: f64,
    pub est_cost: f64,
    pub is_free: bool,
    pub cumulative_spent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeferredAction {
    pub id: String,
    pub task_id: String,
    pub agent_id: String,
    pub kind: ActionKind,
    pub units: f64,
    pub est_cost: f64,
    pub reason: String, // "deferred_budget"
    pub queued_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetStatus {
    pub spent_today: f64,
    pub cap: f64,
    pub pct_spent: f64,
    pub warning_80_pct: bool,
    pub cap_reached: bool,
    pub time_until_reset_secs: u64,
    pub deferred_queue_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecordResult {
    Executed(LedgerEntry),
    Deferred(DeferredAction),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ledger {
    pub config: BudgetCapConfig,
    pub entries: Vec<LedgerEntry>,
    pub deferred_queue: Vec<DeferredAction>,
    pub spent_today: f64,
    pub last_reset_day: u64,
}

impl Ledger {
    pub fn new(config: BudgetCapConfig, now_ts: u64) -> Self {
        let day = now_ts / 86400;
        Self {
            config,
            entries: Vec::new(),
            deferred_queue: Vec::new(),
            spent_today: 0.0,
            last_reset_day: day,
        }
    }

    /// Calculate remaining seconds until 00:00 UTC
    pub fn time_until_reset_secs(&self, now_ts: u64) -> u64 {
        let seconds_in_day = 86400u64;
        let current_day_seconds = now_ts % seconds_in_day;
        seconds_in_day.saturating_sub(current_day_seconds)
    }

    /// Checks if a 00:00 UTC day boundary was crossed and resets spent_today
    pub fn check_daily_reset(&mut self, now_ts: u64) -> Vec<DeferredAction> {
        let current_day = now_ts / 86400;
        if current_day > self.last_reset_day {
            self.last_reset_day = current_day;
            self.spent_today = 0.0;
            // Drain deferred queue on reset

            std::mem::take(&mut self.deferred_queue)
        } else {
            Vec::new()
        }
    }

    /// Manual reset for administrative override or testing
    pub fn manual_reset(&mut self, now_ts: u64) -> Vec<DeferredAction> {
        self.last_reset_day = now_ts / 86400;
        self.spent_today = 0.0;
        std::mem::take(&mut self.deferred_queue)
    }

    /// Calculate estimated cost for an action kind and unit quantity
    pub fn calculate_cost(&self, kind: ActionKind, units: f64) -> f64 {
        match kind {
            ActionKind::Free => 0.0,
            ActionKind::Llm => units * self.config.rates.cost_per_llm_token,
            ActionKind::Sandbox => units * self.config.rates.cost_per_sandbox_second,
            ActionKind::Browser => units * self.config.rates.cost_per_browser_second,
            ActionKind::R2 => units * self.config.rates.cost_per_r2_operation,
        }
    }

    /// Returns current budget status metrics
    pub fn status(&self, now_ts: u64) -> BudgetStatus {
        let pct = if self.config.daily_cap_usd > 0.0 {
            (self.spent_today / self.config.daily_cap_usd) * 100.0
        } else {
            100.0
        };
        BudgetStatus {
            spent_today: (self.spent_today * 10000.0).round() / 10000.0,
            cap: self.config.daily_cap_usd,
            pct_spent: (pct * 10.0).round() / 10.0,
            warning_80_pct: pct >= (self.config.warning_threshold_pct * 100.0),
            cap_reached: self.spent_today >= self.config.daily_cap_usd,
            time_until_reset_secs: self.time_until_reset_secs(now_ts),
            deferred_queue_len: self.deferred_queue.len(),
        }
    }

    /// Record an action or defer it if paid and budget is exhausted
    pub fn record_action(
        &mut self,
        now_ts: u64,
        task_id: &str,
        agent_id: &str,
        kind: ActionKind,
        units: f64,
    ) -> RecordResult {
        // First check if 00:00 UTC rollover happened
        self.check_daily_reset(now_ts);

        let est_cost = self.calculate_cost(kind, units);

        // Free actions always proceed regardless of budget
        if kind.is_free() {
            let entry = LedgerEntry {
                id: format!("led_{}_{}", now_ts, self.entries.len() + 1),
                ts: now_ts,
                task_id: task_id.to_string(),
                agent_id: agent_id.to_string(),
                kind,
                units,
                est_cost: 0.0,
                is_free: true,
                cumulative_spent: self.spent_today,
            };
            self.entries.push(entry.clone());
            return RecordResult::Executed(entry);
        }

        // Paid action: check if within budget cap
        if self.spent_today + est_cost <= self.config.daily_cap_usd {
            self.spent_today += est_cost;
            let entry = LedgerEntry {
                id: format!("led_{}_{}", now_ts, self.entries.len() + 1),
                ts: now_ts,
                task_id: task_id.to_string(),
                agent_id: agent_id.to_string(),
                kind,
                units,
                est_cost,
                is_free: false,
                cumulative_spent: self.spent_today,
            };
            self.entries.push(entry.clone());
            RecordResult::Executed(entry)
        } else {
            // Budget cap reached: defer paid action into queue
            let deferred = DeferredAction {
                id: format!("def_{}_{}", now_ts, self.deferred_queue.len() + 1),
                task_id: task_id.to_string(),
                agent_id: agent_id.to_string(),
                kind,
                units,
                est_cost,
                reason: "deferred_budget".to_string(),
                queued_at: now_ts,
            };
            self.deferred_queue.push(deferred.clone());
            RecordResult::Deferred(deferred)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_free_actions_bypass_budget_cap() {
        let mut ledger = Ledger::new(
            BudgetCapConfig {
                daily_cap_usd: 0.001,
                warning_threshold_pct: 0.80,
                rates: CostRates::default(),
            },
            1792100000,
        );

        // Exhaust budget with a paid LLM call (500 tokens * 0.000002 = $0.001)
        let paid = ledger.record_action(1792100000, "TASK-1", "agent-1", ActionKind::Llm, 500.0);
        assert!(matches!(paid, RecordResult::Executed(_)));
        assert!(ledger.status(1792100000).cap_reached);

        // Next paid action gets deferred
        let paid_deferred =
            ledger.record_action(1792100001, "TASK-2", "agent-1", ActionKind::Sandbox, 10.0);
        assert!(matches!(paid_deferred, RecordResult::Deferred(_)));

        // Free action (planning/triage/review) still executes green!
        let free = ledger.record_action(1792100002, "TASK-3", "agent-pm", ActionKind::Free, 1.0);
        assert!(matches!(free, RecordResult::Executed(_)));
        if let RecordResult::Executed(e) = free {
            assert!(e.is_free);
            assert_eq!(e.est_cost, 0.0);
        }
    }

    #[test]
    fn test_80_percent_warning_and_100_percent_defer() {
        let mut ledger = Ledger::new(
            BudgetCapConfig {
                daily_cap_usd: 0.010,
                warning_threshold_pct: 0.80,
                rates: CostRates::default(),
            },
            1792100000,
        );

        // Action bringing spending to 85% ($0.0085)
        // 4250 tokens * 0.000002 = 0.0085
        let res1 = ledger.record_action(1792100000, "TASK-1", "agent-1", ActionKind::Llm, 4250.0);
        assert!(matches!(res1, RecordResult::Executed(_)));

        let status = ledger.status(1792100000);
        assert!(status.warning_80_pct);
        assert!(!status.cap_reached);

        // Action attempting to spend $0.0030 (would reach $0.0115 > $0.010)
        let res2 = ledger.record_action(1792100001, "TASK-1", "agent-1", ActionKind::Browser, 15.0);
        match res2 {
            RecordResult::Deferred(d) => {
                assert_eq!(d.reason, "deferred_budget");
                assert_eq!(d.task_id, "TASK-1");
            }
            _ => panic!("Expected action to be deferred"),
        }

        assert_eq!(ledger.deferred_queue.len(), 1);
        let status2 = ledger.status(1792100001);
        assert_eq!(status2.deferred_queue_len, 1);
    }

    #[test]
    fn test_reset_drains_deferred_queue_and_resumes() {
        let mut ledger = Ledger::new(
            BudgetCapConfig {
                daily_cap_usd: 0.005,
                warning_threshold_pct: 0.80,
                rates: CostRates::default(),
            },
            1792100000,
        );

        // Defer an action
        let _ = ledger.record_action(1792100000, "TASK-1", "agent-1", ActionKind::Llm, 3000.0);
        let _ = ledger.record_action(1792100001, "TASK-1", "agent-1", ActionKind::Sandbox, 10.0);
        assert_eq!(ledger.deferred_queue.len(), 1);

        // Reset budget (simulate 00:00 UTC or manual reset)
        let resumed = ledger.manual_reset(1792100002);
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].task_id, "TASK-1");
        assert_eq!(ledger.deferred_queue.len(), 0);
        assert_eq!(ledger.spent_today, 0.0);
    }

    #[test]
    fn test_time_until_reset_calculation() {
        let ledger = Ledger::new(BudgetCapConfig::default(), 0);
        assert_eq!(ledger.time_until_reset_secs(0), 86400);
        assert_eq!(ledger.time_until_reset_secs(43200), 43200);
        assert_eq!(ledger.time_until_reset_secs(86399), 1);
    }
}
