use serde::Serialize;
use serde_json::Value;

use crate::adapter::HealthContract;

/// Overall health status of an adapter's output.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Broken,
}

/// Result of a single health check.
#[derive(Debug, Serialize, Clone)]
pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

/// Full health report for an adapter run.
#[derive(Debug, Serialize)]
pub struct HealthReport {
    pub adapter: String,
    pub status: HealthStatus,
    pub checks: Vec<CheckResult>,
}

/// Validate adapter output rows against a health contract.
/// Rows are JSON objects (from .claw.js or YAML pipeline output).
pub fn validate(adapter_name: &str, health: &HealthContract, rows: &[Value]) -> HealthReport {
    let mut checks = Vec::new();

    // Check min_rows
    if let Some(min) = health.min_rows {
        let passed = rows.len() >= min;
        checks.push(CheckResult {
            name: "min_rows".to_string(),
            passed,
            message: if passed {
                format!("got {} rows (min: {})", rows.len(), min)
            } else {
                format!("only {} rows (min: {})", rows.len(), min)
            },
        });
    }

    // Check non_empty columns
    if let Some(ref columns) = health.non_empty {
        for col in columns {
            let empty_count = rows
                .iter()
                .filter(|r| is_value_empty(r.get(col)))
                .count();
            let passed = empty_count == 0;
            checks.push(CheckResult {
                name: format!("non_empty:{}", col),
                passed,
                message: if passed {
                    format!("all rows have non-empty '{}'", col)
                } else {
                    format!("{}/{} rows have empty '{}'", empty_count, rows.len(), col)
                },
            });
        }
    }

    // Determine overall status
    let failed_count = checks.iter().filter(|c| !c.passed).count();
    let status = if checks.is_empty() || failed_count == 0 {
        HealthStatus::Healthy
    } else if failed_count == checks.len() {
        HealthStatus::Broken
    } else {
        HealthStatus::Degraded
    };

    HealthReport {
        adapter: adapter_name.to_string(),
        status,
        checks,
    }
}

/// Check if a JSON value is "empty" for health validation purposes.
fn is_value_empty(value: Option<&Value>) -> bool {
    match value {
        None => true,
        Some(Value::Null) => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        _ => false, // numbers, bools are never "empty"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn contract(min_rows: Option<usize>, non_empty: Option<Vec<String>>) -> HealthContract {
        HealthContract {
            min_rows,
            non_empty,
        }
    }

    #[test]
    fn validate_healthy_output() {
        let health = contract(Some(2), Some(vec!["title".to_string()]));
        let rows = vec![
            json!({"rank": 1, "title": "Hello"}),
            json!({"rank": 2, "title": "World"}),
        ];
        let report = validate("test/check", &health, &rows);
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.checks.iter().all(|c| c.passed));
    }

    #[test]
    fn validate_too_few_rows() {
        let health = contract(Some(5), None);
        let rows = vec![json!({"rank": 1})];
        let report = validate("test/check", &health, &rows);
        assert_eq!(report.status, HealthStatus::Broken);
        let check = report.checks.iter().find(|c| c.name == "min_rows").unwrap();
        assert!(!check.passed);
        assert!(check.message.contains("only 1 rows"));
    }

    #[test]
    fn validate_empty_column_detected() {
        let health = contract(None, Some(vec!["title".to_string()]));
        let rows = vec![
            json!({"title": "Good"}),
            json!({"title": ""}),
            json!({"title": "Also good"}),
        ];
        let report = validate("test/check", &health, &rows);
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "non_empty:title")
            .unwrap();
        assert!(!check.passed);
        assert!(check.message.contains("1/3"));
    }

    #[test]
    fn validate_null_counts_as_empty() {
        let health = contract(None, Some(vec!["title".to_string()]));
        let rows = vec![json!({"title": null}), json!({"other": "value"})];
        let report = validate("test/check", &health, &rows);
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "non_empty:title")
            .unwrap();
        assert!(!check.passed);
        assert!(check.message.contains("2/2"));
    }

    #[test]
    fn validate_numbers_are_never_empty() {
        let health = contract(None, Some(vec!["score".to_string()]));
        let rows = vec![json!({"score": 0}), json!({"score": 42})];
        let report = validate("test/check", &health, &rows);
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "non_empty:score")
            .unwrap();
        assert!(check.passed);
    }

    #[test]
    fn validate_no_contract_is_healthy() {
        let health = contract(None, None);
        let rows = vec![json!({"x": "y"})];
        let report = validate("test/check", &health, &rows);
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.checks.is_empty());
    }

    #[test]
    fn validate_degraded_partial_failure() {
        let health = contract(Some(1), Some(vec!["title".to_string()]));
        let rows = vec![json!({"title": ""})];
        let report = validate("test/check", &health, &rows);
        assert_eq!(report.status, HealthStatus::Degraded);
    }

    #[test]
    fn is_value_empty_edge_cases() {
        assert!(is_value_empty(None));
        assert!(is_value_empty(Some(&json!(null))));
        assert!(is_value_empty(Some(&json!(""))));
        assert!(is_value_empty(Some(&json!("  "))));
        assert!(is_value_empty(Some(&json!([]))));
        assert!(is_value_empty(Some(&json!({}))));
        assert!(!is_value_empty(Some(&json!(0))));
        assert!(!is_value_empty(Some(&json!(false))));
        assert!(!is_value_empty(Some(&json!("hello"))));
        assert!(!is_value_empty(Some(&json!([1]))));
    }
}
