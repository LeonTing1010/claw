//! Lua adapter runtime — execute .lua adapters with full page API.
//!
//! Lua adapters return a table with `site`, `name`, `columns`, and `run(page, args)`.
//! The `page` object exposes CDP scalpels via synchronous Lua functions that
//! internally bridge to async CDP calls via `block_in_place`.

use std::collections::HashMap;

use mlua::prelude::*;
use serde_json::Value;

use crate::cdp::CdpClient;

/// Metadata parsed from a Lua adapter's return table.
#[allow(dead_code)]
pub struct LuaAdapterMeta {
    pub site: String,
    pub name: String,
    pub description: Option<String>,
    pub columns: Vec<String>,
}

/// Bridge: call an async CDP method from sync Lua context.
fn block_async<F, T>(f: F) -> Result<T, LuaError>
where
    F: std::future::Future<Output = Result<T, Box<dyn std::error::Error>>>,
{
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(f)
            .map_err(|e| LuaError::external(e.to_string()))
    })
}

/// Execute a Lua adapter file, returning columns and structured rows.
/// `depth` tracks recursive adapter calls for `claw.run()`.
pub async fn execute_lua_adapter(
    path: &str,
    client: CdpClient,
    args: HashMap<String, Value>,
    depth: u8,
) -> Result<(Vec<String>, Vec<HashMap<String, String>>), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(path)?;

    // Run the Lua execution in block_in_place since Lua is sync
    let result = tokio::task::spawn_blocking(move || {
        let lua = Lua::new();

        // Create the `claw` table with run() for adapter composition
        let claw = create_claw_table(&lua, client.clone(), depth)?;
        lua.globals().set("claw", claw)?;

        // Create the `page` table with all CDP methods
        let page = create_page_table(&lua, client)?;
        lua.globals().set("page", page)?;

        // Create the `args` table
        let args_table = lua.create_table()?;
        for (k, v) in &args {
            match v {
                Value::String(s) => args_table.set(k.as_str(), s.as_str())?,
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        args_table.set(k.as_str(), i)?;
                    } else if let Some(f) = n.as_f64() {
                        args_table.set(k.as_str(), f)?;
                    }
                }
                Value::Bool(b) => args_table.set(k.as_str(), *b)?,
                _ => args_table.set(k.as_str(), v.to_string())?,
            }
        }
        lua.globals().set("args", args_table)?;

        // Add helper: split(str, sep) -> table
        lua.globals().set(
            "split",
            lua.create_function(|lua, (s, sep): (String, String)| {
                let parts: Vec<&str> = s.split(&sep).map(|p| p.trim()).collect();
                let table = lua.create_table()?;
                for (i, part) in parts.iter().enumerate() {
                    table.set(i + 1, *part)?;
                }
                Ok(table)
            })?,
        )?;

        // Execute the adapter source
        let adapter_table: LuaTable = lua.load(&source).eval()?;

        // Extract columns
        let columns: Vec<String> = {
            let cols: LuaTable = adapter_table.get("columns")?;
            let mut v = Vec::new();
            for pair in cols.pairs::<i64, String>() {
                let (_, col) = pair?;
                v.push(col);
            }
            v
        };

        // Call the run function
        let run_fn: LuaFunction = adapter_table.get("run")?;
        let page_ref: LuaTable = lua.globals().get("page")?;
        let args_ref: LuaTable = lua.globals().get("args")?;
        let result: LuaTable = run_fn.call((page_ref, args_ref))?;

        // Convert result table to rows
        let rows = lua_table_to_rows(&result, &columns)?;

        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((columns, rows))
    })
    .await
    .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?
    .map_err(|e| -> Box<dyn std::error::Error> { e })?;

    Ok(result)
}

/// Execute an inline Lua script from a YAML adapter's `run:` field.
/// The script body is wrapped in `function(page, args) ... end` and called.
/// `depth` tracks recursive adapter calls for `claw.run()`.
pub async fn execute_inline_lua(
    script: &str,
    columns: &[String],
    client: CdpClient,
    args: HashMap<String, Value>,
    depth: u8,
) -> Result<Vec<HashMap<String, String>>, Box<dyn std::error::Error>> {
    let script = script.to_string();
    let columns = columns.to_vec();

    let result = tokio::task::spawn_blocking(move || {
        let lua = Lua::new();

        // Create the `claw` table with run() for adapter composition
        let claw = create_claw_table(&lua, client.clone(), depth)?;
        lua.globals().set("claw", claw)?;

        let page = create_page_table(&lua, client)?;
        lua.globals().set("page", page)?;

        let args_table = lua.create_table()?;
        for (k, v) in &args {
            match v {
                Value::String(s) => args_table.set(k.as_str(), s.as_str())?,
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        args_table.set(k.as_str(), i)?;
                    } else if let Some(f) = n.as_f64() {
                        args_table.set(k.as_str(), f)?;
                    }
                }
                Value::Bool(b) => args_table.set(k.as_str(), *b)?,
                _ => args_table.set(k.as_str(), v.to_string())?,
            }
        }
        lua.globals().set("args", args_table)?;

        lua.globals().set(
            "split",
            lua.create_function(|lua, (s, sep): (String, String)| {
                let parts: Vec<&str> = s.split(&sep).map(|p| p.trim()).collect();
                let table = lua.create_table()?;
                for (i, part) in parts.iter().enumerate() {
                    table.set(i + 1, *part)?;
                }
                Ok(table)
            })?,
        )?;

        // Wrap the script in a function and call it with page and args
        let wrapped = format!(
            "local __fn = function(page, args)\n{}\nend\nreturn __fn(page, args)",
            script
        );
        let result: LuaTable = lua.load(&wrapped).eval()?;
        let rows = lua_table_to_rows(&result, &columns)?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(rows)
    })
    .await
    .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?
    .map_err(|e| -> Box<dyn std::error::Error> { e })?;

    Ok(result)
}

/// Parse just the metadata from a Lua adapter (without executing run()).
#[allow(dead_code)]
pub fn parse_lua_metadata(path: &str) -> Result<LuaAdapterMeta, Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(path)?;
    let lua = Lua::new();

    lua.globals().set("page", lua.create_table()?)?;
    lua.globals().set("args", lua.create_table()?)?;
    lua.globals().set(
        "split",
        lua.create_function(|lua, (s, sep): (String, String)| {
            let parts: Vec<&str> = s.split(&sep).collect();
            let table = lua.create_table()?;
            for (i, part) in parts.iter().enumerate() {
                table.set(i + 1, *part)?;
            }
            Ok(table)
        })?,
    )?;

    let adapter_table: LuaTable = lua.load(&source).eval()?;

    Ok(LuaAdapterMeta {
        site: adapter_table.get("site")?,
        name: adapter_table.get("name")?,
        description: adapter_table.get("description").ok(),
        columns: {
            let cols: LuaTable = adapter_table.get("columns")?;
            let mut v = Vec::new();
            for pair in cols.pairs::<i64, String>() {
                v.push(pair?.1);
            }
            v
        },
    })
}

/// Convert a Lua result table to rows.
fn lua_table_to_rows(
    result: &LuaTable,
    columns: &[String],
) -> Result<Vec<HashMap<String, String>>, Box<dyn std::error::Error + Send + Sync>> {
    let mut rows = Vec::new();

    if result.get::<LuaTable>(1).is_ok() {
        // Array of tables
        for pair in result.pairs::<i64, LuaTable>() {
            let (_, row_table) = pair?;
            let mut row = HashMap::new();
            for col in columns {
                let val: String = row_table.get(col.as_str()).unwrap_or_default();
                row.insert(col.clone(), val);
            }
            rows.push(row);
        }
    } else {
        // Single table
        let mut row = HashMap::new();
        for col in columns {
            let val: String = result.get(col.as_str()).unwrap_or_default();
            row.insert(col.clone(), val);
        }
        rows.push(row);
    }

    Ok(rows)
}

/// Create the `claw` table with adapter composition methods.
/// `claw.run(site, name, args?)` calls another adapter and returns its rows as a Lua table.
fn create_claw_table(lua: &Lua, client: CdpClient, depth: u8) -> LuaResult<LuaTable> {
    let claw = lua.create_table()?;

    let c = client;
    let d = depth;
    claw.set(
        "run",
        lua.create_function(
            move |lua, (site, name, args_opt): (String, String, Option<LuaTable>)| {
                let client = c.clone();

                // Convert Lua args table to HashMap<String, Value>
                let mut args_map = HashMap::new();
                if let Some(args_table) = args_opt {
                    for pair in args_table.pairs::<String, LuaValue>() {
                        let (k, v) = pair?;
                        let json_val = match &v {
                            LuaValue::String(s) => Value::String(
                                s.to_str()
                                    .map_err(|e| LuaError::external(e.to_string()))?
                                    .to_string(),
                            ),
                            LuaValue::Integer(i) => Value::Number((*i).into()),
                            LuaValue::Number(f) => Value::Number(
                                serde_json::Number::from_f64(*f)
                                    .ok_or_else(|| LuaError::external("invalid float"))?,
                            ),
                            LuaValue::Boolean(b) => Value::Bool(*b),
                            _ => continue, // skip non-scalar values
                        };
                        args_map.insert(k, json_val);
                    }
                }

                block_async(async {
                    let (_, rows) =
                        crate::adapter::run_adapter(&client, &site, &name, args_map, d + 1)
                            .await
                            .map_err(|e| -> Box<dyn std::error::Error> { e })?;
                    // Convert Vec<HashMap<String, String>> → JSON array → Lua table
                    let json_rows: Vec<Value> = rows
                        .iter()
                        .map(|r| {
                            let obj: serde_json::Map<String, Value> = r
                                .iter()
                                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                                .collect();
                            Value::Object(obj)
                        })
                        .collect();
                    json_to_lua_value(lua, &Value::Array(json_rows))
                        .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })
                })
            },
        )?,
    )?;

    Ok(claw)
}

/// Create the `page` table with CDP methods.
/// Uses sync Lua functions that bridge to async CDP via block_in_place.
fn create_page_table(lua: &Lua, client: CdpClient) -> LuaResult<LuaTable> {
    let page = lua.create_table()?;

    // Macro for simple void methods: page:method(arg) -> nil
    macro_rules! cdp_void {
        ($name:expr, $c:expr, |$client:ident, $a:ident: $t:ty| $body:expr) => {{
            let c = $c.clone();
            page.set(
                $name,
                lua.create_function(move |_lua, $a: $t| {
                    let $client = c.clone();
                    block_async(async { $body })
                })?,
            )?;
        }};
    }

    // Macro for methods returning a value: page:method(arg) -> lua_value
    macro_rules! cdp_value {
        ($name:expr, $c:expr, |$lua_:ident, $client:ident, $a:ident: $t:ty| $body:expr) => {{
            let c = $c.clone();
            page.set(
                $name,
                lua.create_function(move |$lua_, $a: $t| {
                    let $client = c.clone();
                    block_async(async { $body })
                })?,
            )?;
        }};
    }

    cdp_void!("goto", client, |c, url: String| c.navigate(&url).await);

    // page:wait(seconds) — pure sleep, no CDP needed
    page.set(
        "wait",
        lua.create_function(|_lua, secs: f64| {
            block_async(async {
                tokio::time::sleep(std::time::Duration::from_secs_f64(secs)).await;
                Ok(())
            })
        })?,
    )?;

    cdp_value!("evaluate", client, |lua, c, js: String| {
        let result = c.evaluate(&js).await?;
        json_to_lua_value(lua, &result).map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })
    });

    cdp_void!("click_text", client, |c, text: String| {
        c.click_text(&text).await
    });
    cdp_void!("click_selector", client, |c, sel: String| {
        c.click_selector(&sel).await
    });

    // page:type_into(selector, text) — two args
    {
        let c = client.clone();
        page.set(
            "type_into",
            lua.create_function(move |_lua, (sel, text): (String, String)| {
                let c = c.clone();
                block_async(async { c.type_into(&sel, &text).await })
            })?,
        )?;
    }

    // page:upload(selector, files_string_or_table)
    {
        let c = client.clone();
        page.set(
            "upload",
            lua.create_function(move |_lua, (sel, files): (String, LuaValue)| {
                let c = c.clone();
                let paths: Vec<String> = match files {
                    LuaValue::String(s) => s
                        .to_str()
                        .map_err(|e| LuaError::external(e.to_string()))?
                        .split(',')
                        .map(|p| p.trim().to_string())
                        .collect(),
                    LuaValue::Table(t) => {
                        let mut v = Vec::new();
                        for pair in t.pairs::<i64, String>() {
                            v.push(pair?.1);
                        }
                        v
                    }
                    _ => return Err(LuaError::external("files must be string or table")),
                };
                block_async(async {
                    let refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
                    c.upload_files(&sel, &refs).await
                })
            })?,
        )?;
    }

    cdp_void!("screenshot", client, |c, path: String| {
        c.screenshot(&path).await
    });
    cdp_void!("hover", client, |c, sel: String| {
        c.hover_selector(&sel).await
    });
    cdp_void!("scroll", client, |c, sel: String| {
        c.scroll_into_view(&sel).await
    });

    // page:press_key(key, modifiers?)
    {
        let c = client.clone();
        page.set(
            "press_key",
            lua.create_function(move |_lua, (key, mods): (String, Option<u32>)| {
                let c = c.clone();
                block_async(async { c.press_key(&key, mods.unwrap_or(0)).await })
            })?,
        )?;
    }

    // page:select(selector, value)
    {
        let c = client.clone();
        page.set(
            "select",
            lua.create_function(move |_lua, (sel, val): (String, String)| {
                let c = c.clone();
                block_async(async { c.select_option(&sel, &val).await })
            })?,
        )?;
    }

    // page:find(text, role?)
    {
        let c = client.clone();
        page.set(
            "find",
            lua.create_function(move |lua, (text, role): (String, Option<String>)| {
                let c = c.clone();
                block_async(async {
                    let result = c.find_elements(&text, role.as_deref()).await?;
                    json_to_lua_value(lua, &result)
                        .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })
                })
            })?,
        )?;
    }

    // page:wait_for_selector(selector, timeout?)
    {
        let c = client.clone();
        page.set(
            "wait_for_selector",
            lua.create_function(move |_lua, (sel, t): (String, Option<f64>)| {
                let c = c.clone();
                block_async(async { c.wait_for_selector(&sel, t.unwrap_or(10.0)).await })
            })?,
        )?;
    }

    // page:wait_for_text(text, timeout?)
    {
        let c = client.clone();
        page.set(
            "wait_for_text",
            lua.create_function(move |_lua, (text, t): (String, Option<f64>)| {
                let c = c.clone();
                block_async(async { c.wait_for_text(&text, t.unwrap_or(10.0)).await })
            })?,
        )?;
    }

    // page:wait_for_url(pattern, timeout?)
    {
        let c = client.clone();
        page.set(
            "wait_for_url",
            lua.create_function(move |_lua, (pat, t): (String, Option<f64>)| {
                let c = c.clone();
                block_async(async { c.wait_for_url(&pat, t.unwrap_or(10.0)).await })
            })?,
        )?;
    }

    cdp_void!("assert_selector", client, |c, sel: String| {
        c.assert_selector(&sel).await
    });
    cdp_void!("assert_text", client, |c, text: String| {
        c.assert_text(&text).await
    });

    // page:page_info()
    {
        let c = client.clone();
        page.set(
            "page_info",
            lua.create_function(move |lua, ()| {
                let c = c.clone();
                block_async(async {
                    let info = c.get_page_info().await?;
                    json_to_lua_value(lua, &info)
                        .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })
                })
            })?,
        )?;
    }

    Ok(page)
}

/// Convert a serde_json::Value to a Lua value.
pub fn json_to_lua_value(lua: &Lua, value: &Value) -> LuaResult<LuaValue> {
    match value {
        Value::Null => Ok(LuaValue::Nil),
        Value::Bool(b) => Ok(LuaValue::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(LuaValue::Integer(i))
            } else {
                Ok(LuaValue::Number(n.as_f64().unwrap_or(0.0)))
            }
        }
        Value::String(s) => Ok(LuaValue::String(lua.create_string(s)?)),
        Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua_value(lua, v)?)?;
            }
            Ok(LuaValue::Table(table))
        }
        Value::Object(obj) => {
            let table = lua.create_table()?;
            for (k, v) in obj {
                table.set(k.as_str(), json_to_lua_value(lua, v)?)?;
            }
            Ok(LuaValue::Table(table))
        }
    }
}

/// Convert a Lua value back to serde_json::Value.
pub fn lua_value_to_json(value: &LuaValue) -> Value {
    match value {
        LuaValue::Nil => Value::Null,
        LuaValue::Boolean(b) => Value::Bool(*b),
        LuaValue::Integer(i) => Value::Number((*i).into()),
        LuaValue::Number(f) => Value::Number(
            serde_json::Number::from_f64(*f).unwrap_or_else(|| serde_json::Number::from(0)),
        ),
        LuaValue::String(s) => Value::String(s.to_string_lossy().to_string()),
        LuaValue::Table(t) => {
            let len = t.raw_len();
            if len > 0 {
                let mut arr = Vec::with_capacity(len);
                for i in 1..=len {
                    match t.raw_get::<LuaValue>(i) {
                        Ok(v) => arr.push(lua_value_to_json(&v)),
                        Err(_) => arr.push(Value::Null),
                    }
                }
                Value::Array(arr)
            } else {
                let mut map = serde_json::Map::new();
                for (k, v) in t.pairs::<String, LuaValue>().flatten() {
                    map.insert(k, lua_value_to_json(&v));
                }
                if map.is_empty() {
                    Value::Array(vec![])
                } else {
                    Value::Object(map)
                }
            }
        }
        _ => Value::Null,
    }
}

/// Lua helper functions available in transform steps.
const TRANSFORM_HELPERS: &str = r#"
function sort_by(tbl, field, order)
    table.sort(tbl, function(a, b)
        local va, vb = a[field], b[field]
        if va == nil then return false end
        if vb == nil then return true end
        if order == "desc" then
            return va > vb
        else
            return va < vb
        end
    end)
    return tbl
end

function limit(tbl, n)
    local result = {}
    for i = 1, math.min(n, #tbl) do
        result[i] = tbl[i]
    end
    return result
end

function pick(tbl, ...)
    local fields = {...}
    local result = {}
    for i, item in ipairs(tbl) do
        local row = {}
        for _, f in ipairs(fields) do
            row[f] = item[f]
        end
        result[i] = row
    end
    return result
end

function group_by(tbl, field)
    local groups = {}
    local order = {}
    for _, item in ipairs(tbl) do
        local key = tostring(item[field])
        if not groups[key] then
            groups[key] = {}
            order[#order + 1] = key
        end
        local g = groups[key]
        g[#g + 1] = item
    end
    local result = {}
    for _, key in ipairs(order) do
        result[#result + 1] = { key = key, items = groups[key], count = #groups[key] }
    end
    return result
end

function unique_by(tbl, field)
    local seen = {}
    local result = {}
    for _, item in ipairs(tbl) do
        local key = tostring(item[field])
        if not seen[key] then
            seen[key] = true
            result[#result + 1] = item
        end
    end
    return result
end
"#;

/// Execute a Lua transform on pipeline data.
/// The script receives `data` (array of JSON values) and `args` as globals,
/// plus helpers: sort_by, limit, pick, group_by, unique_by.
pub fn execute_transform(
    script: &str,
    data: &[Value],
    args: &HashMap<String, Value>,
) -> Result<Vec<Value>, Box<dyn std::error::Error + Send + Sync>> {
    let lua = Lua::new();

    let data_lua = json_to_lua_value(&lua, &Value::Array(data.to_vec()))?;
    lua.globals().set("data", data_lua)?;

    let args_table = lua.create_table()?;
    for (k, v) in args {
        let lua_val = json_to_lua_value(&lua, v)?;
        args_table.set(k.as_str(), lua_val)?;
    }
    lua.globals().set("args", args_table)?;

    lua.load(TRANSFORM_HELPERS).exec()?;

    let result: LuaValue = lua.load(script).eval()?;
    let json_result = lua_value_to_json(&result);
    match json_result {
        Value::Array(arr) => Ok(arr),
        other => Ok(vec![other]),
    }
}
