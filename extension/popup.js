chrome.storage.local.get(["bridgeStatus", "attachedTabId"], (data) => {
  const connected = data.bridgeStatus === "connected";
  document.getElementById("dot").className = "dot " + (connected ? "on" : "off");
  document.getElementById("status-text").textContent = connected
    ? "Connected to Claw"
    : "Waiting for Claw...";

  if (data.attachedTabId) {
    chrome.tabs.get(data.attachedTabId, (tab) => {
      if (tab) {
        const info = document.getElementById("tab-info");
        info.style.display = "block";
        info.textContent = `Debugging: ${tab.title || tab.url}`;
      }
    });
  }
});
