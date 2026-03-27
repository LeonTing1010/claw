use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;

static TEMPLATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{\{\s*(.+?)\s*\}\}").unwrap());

pub struct TemplateContext {
    pub args: HashMap<String, Value>,
    pub item: Option<Value>,
}

/// Render all `${{ expr }}` placeholders in `tmpl`.
pub fn render(tmpl: &str, ctx: &TemplateContext) -> String {
    TEMPLATE_RE
        .replace_all(tmpl, |caps: &regex::Captures| {
            let expr = caps[1].trim();
            evaluate_expr(expr, ctx)
        })
        .into_owned()
}

/// Evaluate a single expression such as `args.limit` or `args.prompt | json`.
fn evaluate_expr(expr: &str, ctx: &TemplateContext) -> String {
    let mut parts = expr.splitn(2, '|');
    let path = parts.next().unwrap().trim();
    let filter = parts.next().map(|f| f.trim());

    let value = resolve_path(path, ctx);
    format_value(&value, filter)
}

/// Walk a dotted path like `args.limit` or `item.title` against the context.
fn resolve_path(path: &str, ctx: &TemplateContext) -> Value {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.is_empty() {
        return Value::Null;
    }

    let root = match segments[0] {
        "args" => Value::Object(
            ctx.args
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        "item" => match &ctx.item {
            Some(v) => v.clone(),
            None => return Value::Null,
        },
        _ => return Value::Null,
    };

    let mut current = root;
    for &seg in &segments[1..] {
        current = match current {
            Value::Object(ref map) => match map.get(seg) {
                Some(v) => v.clone(),
                None => return Value::Null,
            },
            Value::Array(ref arr) => {
                if let Ok(idx) = seg.parse::<usize>() {
                    match arr.get(idx) {
                        Some(v) => v.clone(),
                        None => return Value::Null,
                    }
                } else {
                    return Value::Null;
                }
            }
            _ => return Value::Null,
        };
    }

    current
}

/// Convert a resolved JSON value to its string representation, optionally
/// applying a pipe filter (currently only `json` is supported).
fn format_value(value: &Value, filter: Option<&str>) -> String {
    match filter {
        Some("json") => serde_json::to_string(value).unwrap_or_default(),
        _ => value_to_string(value),
    }
}

/// Default rendering rules for a JSON value.
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_ctx(args: Vec<(&str, Value)>, item: Option<Value>) -> TemplateContext {
        TemplateContext {
            args: args
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            item,
        }
    }

    #[test]
    fn render_args_substitution() {
        let ctx = make_ctx(vec![("limit", json!(5))], None);
        assert_eq!(render("limit: ${{ args.limit }}", &ctx), "limit: 5");
    }

    #[test]
    fn render_item_substitution() {
        let ctx = make_ctx(vec![], Some(json!({"title": "Hello"})));
        assert_eq!(render("${{ item.title }}", &ctx), "Hello");
    }

    #[test]
    fn render_passthrough() {
        let ctx = make_ctx(vec![], None);
        assert_eq!(
            render("https://bilibili.com", &ctx),
            "https://bilibili.com"
        );
    }

    #[test]
    fn render_multiple_placeholders() {
        let ctx = make_ctx(
            vec![("name", json!("world")), ("count", json!(42))],
            None,
        );
        assert_eq!(
            render("hello ${{ args.name }}, count=${{ args.count }}", &ctx),
            "hello world, count=42"
        );
    }

    #[test]
    fn render_whole_string_template() {
        let ctx = make_ctx(vec![("limit", json!(5))], None);
        assert_eq!(render("${{ args.limit }}", &ctx), "5");
    }

    #[test]
    fn render_missing_key_returns_empty() {
        let ctx = make_ctx(vec![], None);
        let result = render("${{ args.nonexistent }}", &ctx);
        assert_eq!(result, "");
    }
}
