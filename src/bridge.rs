//! Bridge server for Chrome extension communication.
//!
//! Persistent WebSocket server on 127.0.0.1:9333. The Chrome extension
//! connects as a client and proxies commands to the user's real browser.

use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::cdp::CdpClient;

const BRIDGE_PORT: u16 = 9333;

/// Persistent bridge server state.
pub struct BridgeServer {
    client: Arc<Mutex<Option<CdpClient>>>,
}

impl BridgeServer {
    /// Start the bridge server in the background.
    /// Returns immediately — listens for extension connections in a spawned task.
    pub fn start() -> Arc<Self> {
        let client: Arc<Mutex<Option<CdpClient>>> = Arc::new(Mutex::new(None));

        let client_clone = client.clone();
        tokio::spawn(async move {
            if let Err(e) = listen_loop(client_clone).await {
                eprintln!("bridge: listener error: {}", e);
            }
        });

        Arc::new(Self { client })
    }

    /// Get the bridge client if extension is connected.
    pub async fn get_client(&self) -> Option<CdpClient> {
        self.client.lock().await.clone()
    }
}

/// Background listener — accepts extension connections.
async fn listen_loop(client_slot: Arc<Mutex<Option<CdpClient>>>) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", BRIDGE_PORT);
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("bridge: cannot bind {} ({})", addr, e))?;

    eprintln!("bridge: listening on ws://{}", addr);

    loop {
        let (stream, _peer) = listener
            .accept()
            .await
            .map_err(|e| format!("bridge: accept error: {}", e))?;

        eprintln!("bridge: extension connected");

        // Build CdpClient + attach — errors converted to String to stay Send
        match try_connect_and_attach(stream).await {
            Ok((cdp_client, tab_id)) => {
                eprintln!("bridge: attached to tab {}", tab_id);
                *client_slot.lock().await = Some(cdp_client);
            }
            Err(e) => {
                eprintln!("bridge: {}", e);
            }
        }
    }
}

/// Connect and attach in one step — isolates non-Send errors from the spawned task.
async fn try_connect_and_attach(stream: tokio::net::TcpStream) -> Result<(CdpClient, i64), String> {
    let client = CdpClient::connect_from_stream(stream, false)
        .await
        .map_err(|e| format!("handshake failed: {}", e))?;

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.send("Bridge.attach", Some(serde_json::json!({}))),
    )
    .await
    .map_err(|_| "attach timed out".to_string())?
    .map_err(|e| format!("attach failed: {}", e))?;

    if let Some(err) = result.get("error") {
        return Err(format!("attach error: {}", err));
    }

    let tab_id = result.get("tabId").and_then(|v| v.as_i64()).unwrap_or(-1);
    Ok((client, tab_id))
}

/// Try to connect via Chrome extension bridge (blocking, with timeout).
/// Used by CLI commands that don't have a persistent BridgeServer.
pub async fn try_extension_bridge() -> Result<CdpClient, Box<dyn std::error::Error>> {
    let addr = format!("127.0.0.1:{}", BRIDGE_PORT);
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("bridge: cannot bind port {} ({})", BRIDGE_PORT, e))?;

    eprintln!("bridge: waiting for Chrome extension on ws://{}...", addr);

    let (stream, _peer) =
        tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
            .await
            .map_err(|_| "bridge: no extension connected within 10s")?
            .map_err(|e| format!("bridge: accept failed: {}", e))?;

    eprintln!("bridge: extension connected");
    let client = CdpClient::connect_from_stream(stream, false).await?;

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.send("Bridge.attach", Some(serde_json::json!({}))),
    )
    .await
    .map_err(|_| "bridge: attach timed out")?
    .map_err(|e| format!("bridge: attach failed: {}", e))?;

    if let Some(err) = result.get("error") {
        return Err(format!("bridge: attach error: {}", err).into());
    }

    let tab_id = result.get("tabId").and_then(|v| v.as_i64()).unwrap_or(-1);
    eprintln!("bridge: attached to tab {}", tab_id);
    Ok(client)
}
