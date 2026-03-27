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

/// Format and print to stdout.
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
    fn format_table_empty_rows() {
        let columns = vec!["id".to_string(), "status".to_string()];
        let rows: Vec<HashMap<String, String>> = vec![];

        let output = format_table(&columns, &rows);
        assert!(!output.is_empty(), "output should not be empty");
        assert!(output.contains("id"), "output should contain 'id'");
        assert!(output.contains("status"), "output should contain 'status'");
    }
}
