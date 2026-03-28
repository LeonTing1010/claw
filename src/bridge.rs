//! Bridge server for Chrome extension communication.
//!
//! Runs a WebSocket server on 127.0.0.1:9333. The Claw Chrome extension
//! connects as a client and proxies CDP commands to the user's real browser
//! via chrome.debugger API.

use tokio::net::TcpListener;

use crate::cdp::CdpClient;

const BRIDGE_PORT: u16 = 9333;

/// Try to connect via the Chrome extension bridge.
/// Starts a WebSocket server, waits briefly for the extension to connect,
/// then attaches to the active tab.
pub async fn try_extension_bridge() -> Result<CdpClient, Box<dyn std::error::Error>> {
    let addr = format!("127.0.0.1:{}", BRIDGE_PORT);
    let listener = TcpListener::bind(&addr).await.map_err(|e| {
        format!(
            "bridge: cannot bind port {} (another Claw instance?): {}",
            BRIDGE_PORT, e
        )
    })?;

    eprintln!("bridge: waiting for Chrome extension on ws://{}...", addr);

    // Wait up to 10 seconds for the extension to connect (extension may be in reconnect backoff)
    let accept_future = listener.accept();
    let (stream, _peer) = tokio::time::timeout(std::time::Duration::from_secs(10), accept_future)
        .await
        .map_err(|_| "bridge: no extension connected within 3s")?
        .map_err(|e| format!("bridge: accept failed: {}", e))?;

    eprintln!("bridge: extension connected");

    // Build CdpClient from the TCP stream (handles WebSocket upgrade internally, no stealth)
    let client = CdpClient::connect_from_stream(stream, false).await?;

    // Attach to the active tab via Bridge protocol (with timeout)
    let attach_future = client.send("Bridge.attach", Some(serde_json::json!({})));
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), attach_future)
        .await
        .map_err(|_| "bridge: attach timed out (check extension permissions)")?
        .map_err(|e| format!("bridge: attach failed: {}", e))?;

    if let Some(err) = result.get("error") {
        return Err(format!("bridge: attach error: {}", err).into());
    }

    let tab_id = result.get("tabId").and_then(|v| v.as_i64()).unwrap_or(-1);
    eprintln!("bridge: attached to tab {}", tab_id);

    // Stealth is NOT needed — this is the user's real browser
    Ok(client)
}

/// Check if the extension bridge port is available (not already bound by another Claw instance).
pub async fn bridge_port_available() -> bool {
    TcpListener::bind(format!("127.0.0.1:{}", BRIDGE_PORT))
        .await
        .is_ok()
}

/// Get the bridge port number.
pub fn bridge_port() -> u16 {
    BRIDGE_PORT
}
