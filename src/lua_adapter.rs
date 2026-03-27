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
pub async fn execute_lua_adapter(
    path: &str,
    client: CdpClient,
    args: HashMap<String, Value>,
) -> Result<(Vec<String>, Vec<HashMap<String, String>>), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(path)?;

    // Run the Lua execution in block_in_place since Lua is sync
    let result = tokio::task::spawn_blocking(move || {
        let lua = Lua::new();

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
fn json_to_lua_value(lua: &Lua, value: &Value) -> LuaResult<LuaValue> {
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
