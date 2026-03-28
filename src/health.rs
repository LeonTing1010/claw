use std::collections::HashMap;

use serde::Serialize;

use crate::adapter::Adapter;

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

/// Validate adapter output against its health contract and schema.
/// If no health/schema is defined, returns Healthy with zero checks.
pub fn validate(adapter: &Adapter, rows: &[HashMap<String, String>]) -> HealthReport {
    let mut checks = Vec::new();
    let adapter_name = format!("{}/{}", adapter.site, adapter.name);

    // Check health contract
    if let Some(ref health) = adapter.health {
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

        if let Some(ref columns) = health.non_empty {
            for col in columns {
                let empty_count = rows
                    .iter()
                    .filter(|r| r.get(col).map_or(true, |v| v.trim().is_empty()))
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
    }

    // Check schema types
    if let Some(ref schema) = adapter.schema {
        for (col, expected_type) in schema {
            let violations = count_type_violations(rows, col, expected_type);
            let passed = violations == 0;
            checks.push(CheckResult {
                name: format!("schema:{}:{}", col, expected_type),
                passed,
                message: if passed {
                    format!("all values in '{}' match type '{}'", col, expected_type)
                } else {
                    format!(
                        "{}/{} values in '{}' don't match type '{}'",
                        violations,
                        rows.len(),
                        col,
                        expected_type
                    )
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
        adapter: adapter_name,
        status,
        checks,
    }
}

fn count_type_violations(
    rows: &[HashMap<String, String>],
    column: &str,
    expected_type: &str,
) -> usize {
    rows.iter()
        .filter(|row| {
            let val = match row.get(column) {
                Some(v) => v.as_str(),
                None => return true,
            };
            !matches_type(val, expected_type)
        })
        .count()
}

fn matches_type(value: &str, type_name: &str) -> bool {
    match type_name {
        "int" => value.parse::<i64>().is_ok(),
        "float" => value.parse::<f64>().is_ok(),
        "string" => true,
        "url" => value.starts_with("http://") || value.starts_with("https://"),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{Adapter, HealthContract};

    fn make_adapter(
        health: Option<HealthContract>,
        schema: Option<HashMap<String, String>>,
    ) -> Adapter {
        Adapter {
            site: "test".to_string(),
            name: "check".to_string(),
            description: None,
            domain: None,
            strategy: None,
            browser: None,
            args: None,
            columns: vec!["rank".to_string(), "title".to_string()],
            pipeline: vec![],
            run: None,
            version: None,
            last_forged: None,
            forged_by: None,
            schema,
            health,
        }
    }

    fn row(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn validate_healthy_output() {
        let adapter = make_adapter(
            Some(HealthContract {
                min_rows: Some(2),
                non_empty: Some(vec!["title".to_string()]),
            }),
            Some(HashMap::from([
                ("rank".to_string(), "int".to_string()),
                ("title".to_string(), "string".to_string()),
            ])),
        );
        let rows = vec![
            row(&[("rank", "1"), ("title", "Hello")]),
            row(&[("rank", "2"), ("title", "World")]),
        ];
        let report = validate(&adapter, &rows);
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.checks.iter().all(|c| c.passed));
    }

    #[test]
    fn validate_too_few_rows() {
        let adapter = make_adapter(
            Some(HealthContract {
                min_rows: Some(5),
                non_empty: None,
            }),
            None,
        );
        let rows = vec![row(&[("rank", "1")])];
        let report = validate(&adapter, &rows);
        assert_eq!(report.status, HealthStatus::Broken);
        let check = report.checks.iter().find(|c| c.name == "min_rows").unwrap();
        assert!(!check.passed);
        assert!(check.message.contains("only 1 rows"));
    }

    #[test]
    fn validate_empty_column_detected() {
        let adapter = make_adapter(
            Some(HealthContract {
                min_rows: None,
                non_empty: Some(vec!["title".to_string()]),
            }),
            None,
        );
        let rows = vec![
            row(&[("title", "Good")]),
            row(&[("title", "")]),
            row(&[("title", "Also good")]),
        ];
        let report = validate(&adapter, &rows);
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "non_empty:title")
            .unwrap();
        assert!(!check.passed);
        assert!(check.message.contains("1/3"));
    }

    #[test]
    fn validate_schema_type_int() {
        let adapter = make_adapter(
            None,
            Some(HashMap::from([("rank".to_string(), "int".to_string())])),
        );
        let rows = vec![row(&[("rank", "1")]), row(&[("rank", "not_a_number")])];
        let report = validate(&adapter, &rows);
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "schema:rank:int")
            .unwrap();
        assert!(!check.passed);
        assert!(check.message.contains("1/2"));
    }

    #[test]
    fn validate_schema_type_url() {
        let adapter = make_adapter(
            None,
            Some(HashMap::from([("link".to_string(), "url".to_string())])),
        );
        let rows = vec![
            row(&[("link", "https://example.com")]),
            row(&[("link", "not-a-url")]),
        ];
        let report = validate(&adapter, &rows);
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "schema:link:url")
            .unwrap();
        assert!(!check.passed);
    }

    #[test]
    fn validate_no_contract_is_healthy() {
        let adapter = make_adapter(None, None);
        let rows = vec![row(&[("x", "y")])];
        let report = validate(&adapter, &rows);
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.checks.is_empty());
    }

    #[test]
    fn validate_degraded_partial_failure() {
        let adapter = make_adapter(
            Some(HealthContract {
                min_rows: Some(1),
                non_empty: Some(vec!["title".to_string()]),
            }),
            None,
        );
        // Meets min_rows (1 row) but fails non_empty (title is empty)
        let rows = vec![row(&[("title", "")])];
        let report = validate(&adapter, &rows);
        assert_eq!(report.status, HealthStatus::Degraded);
    }

    #[test]
    fn matches_type_edge_cases() {
        assert!(matches_type("0", "int"));
        assert!(matches_type("-42", "int"));
        assert!(!matches_type("3.14", "int"));
        assert!(matches_type("3.14", "float"));
        assert!(matches_type("42", "float")); // int is valid float
        assert!(matches_type("anything", "string"));
        assert!(matches_type("http://x.com", "url"));
        assert!(matches_type("https://x.com", "url"));
        assert!(!matches_type("ftp://x.com", "url"));
    }
}
