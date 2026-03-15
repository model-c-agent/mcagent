use serde::{Deserialize, Serialize};

/// Budget constraints for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub token_input_limit: Option<u64>,
    pub token_output_limit: Option<u64>,
    pub cpu_seconds: Option<f64>,
    pub memory_mb_seconds: Option<f64>,
    pub wall_clock_seconds: Option<u64>,
    pub api_calls: Option<u64>,
    pub work_hours: Option<f64>,
}

/// Tracks resource usage against a budget.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetUsage {
    pub input_tokens_used: u64,
    pub output_tokens_used: u64,
    pub cpu_seconds_used: f64,
    pub memory_mb_seconds_used: f64,
    pub wall_clock_seconds_used: u64,
    pub api_calls_used: u64,
    pub work_hours_used: f64,
    pub started_at: Option<u64>,
}

impl BudgetUsage {
    pub fn record_api_call(&mut self) {
        self.api_calls_used += 1;
    }

    pub fn record_tokens(&mut self, input: u64, output: u64) {
        self.input_tokens_used += input;
        self.output_tokens_used += output;
    }

    pub fn record_compute(&mut self, cpu_secs: f64, mem_mb_secs: f64) {
        self.cpu_seconds_used += cpu_secs;
        self.memory_mb_seconds_used += mem_mb_secs;
    }

    /// Compute work hours from current usage using default conversion weights.
    /// Default: 1 work hour = ~10,000 tokens + ~60 API calls + ~3600 CPU-seconds.
    pub fn compute_work_hours(&self) -> f64 {
        let token_hours =
            (self.input_tokens_used + self.output_tokens_used) as f64 / 10_000.0;
        let api_hours = self.api_calls_used as f64 / 60.0;
        let cpu_hours = self.cpu_seconds_used / 3600.0;
        // Take the maximum dimension as the work hours estimate
        token_hours.max(api_hours).max(cpu_hours)
    }
}

/// Status of budget consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BudgetStatus {
    WithinBudget { usage_percent: f64 },
    Warning { usage_percent: f64, dimension: String },
    Exceeded { dimension: String, limit: f64, actual: f64 },
}

/// Check budget against usage. Returns the most critical status.
pub fn check_budget(budget: &Budget, usage: &BudgetUsage) -> BudgetStatus {
    let mut worst_percent: f64 = 0.0;
    let mut worst_dimension = String::new();

    let checks: Vec<(&str, Option<f64>, f64)> = vec![
        ("input_tokens", budget.token_input_limit.map(|l| l as f64), usage.input_tokens_used as f64),
        ("output_tokens", budget.token_output_limit.map(|l| l as f64), usage.output_tokens_used as f64),
        ("cpu_seconds", budget.cpu_seconds, usage.cpu_seconds_used),
        ("memory_mb_seconds", budget.memory_mb_seconds, usage.memory_mb_seconds_used),
        ("wall_clock_seconds", budget.wall_clock_seconds.map(|l| l as f64), usage.wall_clock_seconds_used as f64),
        ("api_calls", budget.api_calls.map(|l| l as f64), usage.api_calls_used as f64),
        ("work_hours", budget.work_hours, usage.compute_work_hours()),
    ];

    for (dim, limit, actual) in checks {
        if let Some(limit) = limit {
            if limit <= 0.0 {
                continue;
            }
            let pct = (actual / limit) * 100.0;
            if actual > limit {
                return BudgetStatus::Exceeded {
                    dimension: dim.to_string(),
                    limit,
                    actual,
                };
            }
            if pct > worst_percent {
                worst_percent = pct;
                worst_dimension = dim.to_string();
            }
        }
    }

    if worst_percent >= 80.0 {
        BudgetStatus::Warning {
            usage_percent: worst_percent,
            dimension: worst_dimension,
        }
    } else {
        BudgetStatus::WithinBudget {
            usage_percent: worst_percent,
        }
    }
}

/// Estimate a budget for a task based on complexity.
pub fn estimate_task_budget(complexity: &str) -> Budget {
    match complexity.to_lowercase().as_str() {
        "low" => Budget {
            token_input_limit: Some(3_000),
            token_output_limit: Some(2_000),
            cpu_seconds: Some(60.0),
            memory_mb_seconds: None,
            wall_clock_seconds: Some(300),
            api_calls: Some(5),
            work_hours: Some(0.5),
        },
        "high" => Budget {
            token_input_limit: Some(50_000),
            token_output_limit: Some(30_000),
            cpu_seconds: Some(3600.0),
            memory_mb_seconds: None,
            wall_clock_seconds: Some(7200),
            api_calls: Some(150),
            work_hours: Some(8.0),
        },
        _ => Budget {
            // medium (default)
            token_input_limit: Some(12_000),
            token_output_limit: Some(8_000),
            cpu_seconds: Some(600.0),
            memory_mb_seconds: None,
            wall_clock_seconds: Some(1800),
            api_calls: Some(40),
            work_hours: Some(2.0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_within_budget() {
        let budget = Budget {
            token_input_limit: Some(10_000),
            token_output_limit: Some(5_000),
            cpu_seconds: None,
            memory_mb_seconds: None,
            wall_clock_seconds: None,
            api_calls: Some(100),
            work_hours: None,
        };
        let usage = BudgetUsage {
            input_tokens_used: 1_000,
            api_calls_used: 10,
            ..Default::default()
        };
        assert!(matches!(check_budget(&budget, &usage), BudgetStatus::WithinBudget { .. }));
    }

    #[test]
    fn test_exceeded_budget() {
        let budget = Budget {
            token_input_limit: Some(100),
            token_output_limit: None,
            cpu_seconds: None,
            memory_mb_seconds: None,
            wall_clock_seconds: None,
            api_calls: None,
            work_hours: None,
        };
        let usage = BudgetUsage {
            input_tokens_used: 200,
            ..Default::default()
        };
        assert!(matches!(check_budget(&budget, &usage), BudgetStatus::Exceeded { .. }));
    }

    #[test]
    fn test_warning_budget() {
        let budget = Budget {
            token_input_limit: None,
            token_output_limit: None,
            cpu_seconds: None,
            memory_mb_seconds: None,
            wall_clock_seconds: None,
            api_calls: Some(100),
            work_hours: None,
        };
        let usage = BudgetUsage {
            api_calls_used: 85,
            ..Default::default()
        };
        assert!(matches!(check_budget(&budget, &usage), BudgetStatus::Warning { .. }));
    }

    #[test]
    fn test_estimate_low() {
        let b = estimate_task_budget("low");
        assert_eq!(b.api_calls, Some(5));
    }

    #[test]
    fn test_estimate_high() {
        let b = estimate_task_budget("high");
        assert_eq!(b.api_calls, Some(150));
    }

    #[test]
    fn test_record_api_call() {
        let mut usage = BudgetUsage::default();
        usage.record_api_call();
        usage.record_api_call();
        assert_eq!(usage.api_calls_used, 2);
    }
}
