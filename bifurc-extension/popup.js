/**
 * Bifurc Companion — Popup Script
 */

const proxyToggle = document.getElementById("proxyToggle");
const proxyPortInput = document.getElementById("proxyPort");
const companionPortInput = document.getElementById("companionPort");
const statusDot = document.getElementById("statusDot");
const statusText = document.getElementById("statusText");

// ── Load state ────────────────────────────────────────────────────────────────

async function loadState() {
    const config = await chrome.storage.local.get({
        proxyEnabled: false,
        proxyPort: 80,
        companionPort: 9271,
    });

    proxyToggle.checked = config.proxyEnabled;
    proxyPortInput.value = config.proxyPort;
    companionPortInput.value = config.companionPort;

    // Get connection status
    chrome.runtime.sendMessage({ type: "get:status" }, (response) => {
        if (response && response.connected) {
            statusDot.className = "dot dot-connected";
            statusText.textContent = "Connected to Bifurc";
        } else {
            statusDot.className = "dot dot-disconnected";
            statusText.textContent = "Disconnected";
        }
    });
}

// ── Event handlers ────────────────────────────────────────────────────────────

proxyToggle.addEventListener("change", () => {
    chrome.runtime.sendMessage({
        type: "proxy:toggle",
        enabled: proxyToggle.checked,
    });
});

proxyPortInput.addEventListener("change", () => {
    const port = parseInt(proxyPortInput.value, 10);
    if (port >= 1 && port <= 65535) {
        chrome.runtime.sendMessage({ type: "proxy:setPort", port });
    }
});

companionPortInput.addEventListener("change", () => {
    const port = parseInt(companionPortInput.value, 10);
    if (port >= 1 && port <= 65535) {
        chrome.runtime.sendMessage({ type: "companion:setPort", port });
    }
});

// ── Listen for status updates ─────────────────────────────────────────────────

chrome.runtime.onMessage.addListener((message) => {
    if (message.type === "companion:status") {
        if (message.status === "connected") {
            statusDot.className = "dot dot-connected";
            statusText.textContent = "Connected to Bifurc";
        } else {
            statusDot.className = "dot dot-disconnected";
            statusText.textContent = "Disconnected";
        }
    }
});

// ── Init ──────────────────────────────────────────────────────────────────────

loadState();
