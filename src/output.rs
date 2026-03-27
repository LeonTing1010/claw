use std::collections::HashMap;

use tabled::{builder::Builder, settings::Style};

/// Format rows as a table string (for testing).
pub fn format_table(columns: &[String], rows: &[HashMap<String, String>]) -> String {
    let mut builder = Builder::default();
    // Add header row
    builder.push_record(columns.iter().map(|c| c.as_str()));
    // Add data rows
    for row in rows {
        let record: Vec<&str> = columns
            .iter()
            .map(|col| row.get(col).map(|s| s.as_str()).unwrap_or(""))
            .collect();
        builder.push_record(record);
    }
    builder.build().with(Style::rounded()).to_string()
}

/// Format rows as JSON array.
pub fn format_json(columns: &[String], rows: &[HashMap<String, String>]) -> String {
    let filtered: Vec<serde_json::Map<String, serde_json::Value>> = rows
        .iter()
        .map(|row| {
            let mut map = serde_json::Map::new();
            for col in columns {
                let val = row.get(col).cloned().unwrap_or_default();
                map.insert(col.clone(), serde_json::Value::String(val));
            }
            map
        })
        .collect();
    serde_json::to_string_pretty(&filtered).unwrap_or_default()
}

/// Format rows as CSV (RFC 4180 basic).
pub fn format_csv(columns: &[String], rows: &[HashMap<String, String>]) -> String {
    let mut lines = Vec::with_capacity(rows.len() + 1);
    lines.push(columns.join(","));
    for row in rows {
        let vals: Vec<String> = columns
            .iter()
            .map(|col| {
                let val = row.get(col).cloned().unwrap_or_default();
                if val.contains(',') || val.contains('"') || val.contains('\n') {
                    format!("\"{}\"", val.replace('"', "\"\""))
                } else {
                    val
                }
            })
            .collect();
        lines.push(vals.join(","));
    }
    lines.join("\n")
}

/// Print rows in the specified format.
pub fn print_output(
    columns: &[String],
    rows: &[HashMap<String, String>],
    format: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match format {
        "table" => println!("{}", format_table(columns, rows)),
        "json" => println!("{}", format_json(columns, rows)),
        "csv" => println!("{}", format_csv(columns, rows)),
        other => return Err(format!("unknown format: {} (expected: table, json, csv)", other).into()),
    }
    Ok(())
}

/// Format and print to stdout (table format).
pub fn print_table(columns: &[String], rows: &[HashMap<String, String>]) {
    println!("{}", format_table(columns, rows));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_table_basic() {
        let columns = vec!["title".to_string(), "views".to_string()];
        let mut row = HashMap::new();
        row.insert("title".to_string(), "Hello".to_string());
        row.insert("views".to_string(), "1000".to_string());
        let rows = vec![row];

        let output = format_table(&columns, &rows);
        assert!(output.contains("title"), "output should contain 'title'");
        assert!(output.contains("views"), "output should contain 'views'");
        assert!(output.contains("Hello"), "output should contain 'Hello'");
        assert!(output.contains("1000"), "output should contain '1000'");
    }

    #[test]
    fn format_table_missing_column() {
        let columns = vec!["name".to_string(), "age".to_string()];
        let mut row = HashMap::new();
        row.insert("name".to_string(), "Alice".to_string());
        // "age" is intentionally missing
        let rows = vec![row];

        let output = format_table(&columns, &rows);
        assert!(output.contains("Alice"), "output should contain 'Alice'");
        // Should not panic and should still produce valid output
        assert!(output.contains("name"), "output should contain 'name'");
        assert!(output.contains("age"), "output should contain 'age'");
    }

    #[test]
    fn format_json_produces_valid_json() {
        let columns = vec!["title".to_string(), "views".to_string()];
        let mut row = HashMap::new();
        row.insert("title".to_string(), "Hello".to_string());
        row.insert("views".to_string(), "1000".to_string());
        let rows = vec![row];
        let json_str = format_json(&columns, &rows);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["title"], "Hello");
        assert_eq!(parsed[0]["views"], "1000");
    }

    #[test]
    fn format_csv_has_correct_headers() {
        let columns = vec!["name".to_string(), "age".to_string()];
        let mut row = HashMap::new();
        row.insert("name".to_string(), "Alice".to_string());
        row.insert("age".to_string(), "30".to_string());
        let rows = vec![row];
        let csv_str = format_csv(&columns, &rows);
        let lines: Vec<&str> = csv_str.lines().collect();
        assert_eq!(lines[0], "name,age");
        assert_eq!(lines[1], "Alice,30");
    }

    #[test]
    fn format_csv_quotes_commas() {
        let columns = vec!["title".to_string()];
        let mut row = HashMap::new();
        row.insert("title".to_string(), "hello, world".to_string());
        let rows = vec![row];
        let csv_str = format_csv(&columns, &rows);
        assert!(csv_str.contains("\"hello, world\""));
    }

    #[test]
    fn format_table_empty_rows() {
        let columns = vec!["id".to_string(), "status".to_string()];
        let rows: Vec<HashMap<String, String>> = vec![];

        let output = format_table(&columns, &rows);
        assert!(!output.is_empty(), "output should not be empty");
        assert!(output.contains("id"), "output should contain 'id'");
        assert!(output.contains("status"), "output should contain 'status'");
    }
}
