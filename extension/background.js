/**
 * Claw Bridge — Chrome Extension Service Worker (v2: Stealth Mode)
 *
 * TWO modes:
 * - Scripting mode (default): chrome.scripting + chrome.tabs — UNDETECTABLE by websites
 * - Debugger mode (fallback): chrome.debugger — for advanced CDP ops (screenshot, Input events)
 *
 * Scripting mode handles Page.navigate and Runtime.evaluate WITHOUT attaching
 * the debugger, so anti-bot systems cannot detect automation.
 */

const CLAW_PORT = 9333;
const RECONNECT_BASE = 1000;
const RECONNECT_MAX = 30000;
const KEEPALIVE_MS = 25000;

let ws = null;
let reconnectDelay = RECONNECT_BASE;
let activeTabId = null;
let debuggerAttached = false;
let keepaliveTimer = null;

// --- Connection ---

function connect() {
  try {
    ws = new WebSocket(`ws://127.0.0.1:${CLAW_PORT}`);
  } catch (e) {
    scheduleReconnect();
    return;
  }

  ws.onopen = () => {
    console.log("[claw-bridge] connected to Claw");
    reconnectDelay = RECONNECT_BASE;
    updateStatus(true);
    startKeepalive();
  };

  ws.onmessage = (event) => {
    handleMessage(event.data);
  };

  ws.onclose = () => {
    console.log("[claw-bridge] disconnected");
    cleanup();
    scheduleReconnect();
  };

  ws.onerror = () => {
    cleanup();
    scheduleReconnect();
  };
}

function scheduleReconnect() {
  updateStatus(false);
  setTimeout(() => {
    reconnectDelay = Math.min(reconnectDelay * 2, RECONNECT_MAX);
    connect();
  }, reconnectDelay);
}

function cleanup() {
  stopKeepalive();
  if (debuggerAttached && activeTabId) {
    chrome.debugger.detach({ tabId: activeTabId }).catch(() => {});
    debuggerAttached = false;
  }
}

function startKeepalive() {
  stopKeepalive();
  keepaliveTimer = setInterval(() => {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ method: "Bridge.ping" }));
    }
  }, KEEPALIVE_MS);
  chrome.alarms.create("keepalive", { periodInMinutes: 0.4 });
}

function stopKeepalive() {
  if (keepaliveTimer) {
    clearInterval(keepaliveTimer);
    keepaliveTimer = null;
  }
  chrome.alarms.clear("keepalive");
}

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === "keepalive" && (!ws || ws.readyState !== WebSocket.OPEN)) {
    connect();
  }
});

// --- Status ---

function updateStatus(connected) {
  chrome.storage.local.set({
    bridgeStatus: connected ? "connected" : "disconnected",
    activeTabId: activeTabId,
    mode: debuggerAttached ? "debugger" : "scripting",
  });
}

// --- Message Handling ---

async function handleMessage(raw) {
  let msg;
  try {
    msg = JSON.parse(raw);
  } catch (e) {
    return;
  }

  const { id, method, params } = msg;

  // Bridge meta-commands
  if (method && method.startsWith("Bridge.")) {
    try {
      const result = await handleBridgeCommand(method, params || {});
      if (id !== undefined) send({ id, result });
    } catch (e) {
      if (id !== undefined) send({ id, error: { code: -1, message: e.message || String(e) } });
    }
    return;
  }

  // Route CDP commands through scripting mode or debugger mode
  try {
    const result = await routeCommand(method, params || {});
    if (id !== undefined) send({ id, result: result || {} });
  } catch (e) {
    if (id !== undefined) send({ id, error: { code: -32000, message: e.message || String(e) } });
  }
}

// --- Stealth Routing ---
// Route CDP commands to chrome.scripting/chrome.tabs when possible,
// fall back to chrome.debugger only when needed.

async function routeCommand(method, params) {
  if (!activeTabId) {
    throw new Error("No tab selected. Call Bridge.attach first.");
  }

  // Scripting mode: handle common commands WITHOUT debugger
  switch (method) {
    case "Page.navigate": {
      // Use chrome.tabs.update — real navigation, undetectable
      const url = params.url;
      await chrome.tabs.update(activeTabId, { url });
      // Wait for load
      await waitForTabLoad(activeTabId);
      return { frameId: "main" };
    }

    case "Runtime.evaluate": {
      // Use chrome.scripting.executeScript — no debugger needed
      const expression = params.expression;
      const results = await chrome.scripting.executeScript({
        target: { tabId: activeTabId },
        func: (expr) => {
          try {
            return eval(expr);
          } catch (e) {
            return { __error: e.message };
          }
        },
        args: [expression],
        world: "MAIN", // Run in page's JS context (access to page's window.__INITIAL_STATE__ etc)
      });
      const value = results?.[0]?.result;
      if (value && value.__error) {
        return { result: { type: "object", value: null }, exceptionDetails: { text: value.__error } };
      }
      // Format like CDP Runtime.evaluate response
      return { result: { type: typeof value, value: value } };
    }

    case "Page.enable":
    case "Network.enable":
    case "Network.disable":
      // No-op in scripting mode — these are CDP-specific
      return {};

    case "Page.addScriptToEvaluateOnNewDocument":
      // Skip stealth injection — not needed in user's real browser
      return { identifier: "skipped" };

    case "Network.setUserAgentOverride":
      // Skip — user's real browser has the right UA
      return {};

    default:
      // For anything else (Input.*, Page.captureScreenshot, DOM.*, etc.)
      // Fall back to debugger mode
      return await debuggerCommand(method, params);
  }
}

// --- Debugger Fallback ---

async function debuggerCommand(method, params) {
  if (!debuggerAttached) {
    console.log("[claw-bridge] attaching debugger for", method);
    await chrome.debugger.attach({ tabId: activeTabId }, "1.3");
    debuggerAttached = true;
    await chrome.debugger.sendCommand({ tabId: activeTabId }, "Page.enable", {});
  }
  return await chrome.debugger.sendCommand({ tabId: activeTabId }, method, params);
}

// --- Tab Helpers ---

function waitForTabLoad(tabId) {
  return new Promise((resolve) => {
    const check = (tId, changeInfo) => {
      if (tId === tabId && changeInfo.status === "complete") {
        chrome.tabs.onUpdated.removeListener(check);
        resolve();
      }
    };
    chrome.tabs.onUpdated.addListener(check);
    // Timeout fallback
    setTimeout(() => {
      chrome.tabs.onUpdated.removeListener(check);
      resolve();
    }, 30000);
  });
}

// --- Bridge Commands ---

async function handleBridgeCommand(method, params) {
  switch (method) {
    case "Bridge.ping":
      return { pong: true };

    case "Bridge.getTargets": {
      const tabs = await chrome.tabs.query({});
      return tabs.map((t) => ({
        id: String(t.id),
        type: "page",
        title: t.title || "",
        url: t.url || "",
        webSocketDebuggerUrl: `bridge://tab/${t.id}`,
      }));
    }

    case "Bridge.attach": {
      // In scripting mode, we just select the tab — no debugger attachment
      const tabId = params.tabId
        ? Number(params.tabId)
        : (await chrome.tabs.query({ active: true, currentWindow: true }))[0]?.id;

      if (!tabId) {
        return { error: "No tab to attach" };
      }

      console.log("[claw-bridge] selecting tab", tabId, "(scripting mode, no debugger)");
      activeTabId = tabId;
      updateStatus(true);
      return { tabId, attached: true, mode: "scripting" };
    }

    case "Bridge.detach": {
      if (debuggerAttached && activeTabId) {
        await chrome.debugger.detach({ tabId: activeTabId }).catch(() => {});
        debuggerAttached = false;
      }
      activeTabId = null;
      updateStatus(true);
      return { detached: true };
    }

    case "Bridge.newTab": {
      const tab = await chrome.tabs.create({ url: params.url || "about:blank" });
      return { tabId: tab.id, url: tab.url };
    }

    default:
      return { error: `Unknown bridge command: ${method}` };
  }
}

// --- CDP Event Forwarding (debugger mode only) ---

chrome.debugger.onEvent.addListener((source, method, params) => {
  if (source.tabId === activeTabId && ws && ws.readyState === WebSocket.OPEN) {
    send({ method, params });
  }
});

chrome.debugger.onDetach.addListener((source, reason) => {
  if (source.tabId === activeTabId) {
    debuggerAttached = false;
    updateStatus(true);
  }
});

// --- Send ---

function send(msg) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(msg));
  }
}

// --- Start ---

connect();
