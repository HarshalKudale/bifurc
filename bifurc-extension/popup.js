/**
 * Bifurc Companion — Popup Script
 *
 * The popup is a thin view over the background worker's resolved state.
 * `proxyActive` (toggle ON **and** companion connected) is the only value that
 * means traffic is really being routed; everything else is direct passthrough.
 */

const els = {
    statusCard: document.getElementById("statusCard"),
    statusText: document.getElementById("statusText"),
    reconnectBtn: document.getElementById("reconnectBtn"),
    proxyToggle: document.getElementById("proxyToggle"),
    proxyPort: document.getElementById("proxyPort"),
    companionPort: document.getElementById("companionPort"),
    proxyState: document.getElementById("proxyState"),
    proxyStateText: document.getElementById("proxyStateText"),
};

// Mirrors background state so render() can stay synchronous.
const state = {
    connected: false,
    proxyEnabled: false,
    proxyActive: false,
    proxyPort: 80,
    companionPort: 9271,
};

// ── Messaging ─────────────────────────────────────────────────────────────────

function send(message, callback) {
    try {
        chrome.runtime.sendMessage(message, (response) => {
            if (chrome.runtime.lastError) return;
            if (callback) callback(response);
        });
    } catch {
        /* extension context invalidated — nothing useful to do */
    }
}

function applyState(next) {
    if (!next) return;
    if (typeof next.connected === "boolean") state.connected = next.connected;
    if (typeof next.proxyEnabled === "boolean") state.proxyEnabled = next.proxyEnabled;
    if (typeof next.proxyActive === "boolean") state.proxyActive = next.proxyActive;
    if (Number.isFinite(next.proxyPort)) state.proxyPort = next.proxyPort;
    if (Number.isFinite(next.companionPort)) state.companionPort = next.companionPort;
    render();
}

function refresh() {
    send({ type: "get:status" }, applyState);
}

// ── Render ────────────────────────────────────────────────────────────────────

function render() {
    // Connection pill
    els.statusCard.classList.toggle("on", state.connected);
    els.statusText.textContent = state.connected
        ? "Connected to Bifurc"
        : "Companion server offline";

    // Toggle reflects stored intent
    els.proxyToggle.checked = state.proxyEnabled;

    // Resolved routing state — this is what actually matters
    els.proxyState.classList.remove("active", "paused");
    if (state.proxyActive) {
        els.proxyState.classList.add("active");
        els.proxyStateText.innerHTML =
            `Routing via <code>127.0.0.1:${state.proxyPort}</code>`;
    } else if (state.proxyEnabled) {
        els.proxyState.classList.add("paused");
        els.proxyStateText.textContent = "Paused — companion server not connected";
    } else {
        els.proxyStateText.textContent = "Direct passthrough — proxy is off";
    }

    // Ports — never clobber the field the user is currently editing
    if (document.activeElement !== els.proxyPort) els.proxyPort.value = state.proxyPort;
    if (document.activeElement !== els.companionPort) els.companionPort.value = state.companionPort;
}

// ── Event handlers ────────────────────────────────────────────────────────────

els.proxyToggle.addEventListener("change", () => {
    state.proxyEnabled = els.proxyToggle.checked;
    render(); // optimistic — avoids a visible lag on the switch
    send({ type: "proxy:toggle", enabled: state.proxyEnabled }, applyState);
});

function commitPort(input, key, messageType) {
    const value = parseInt(input.value, 10);
    if (!Number.isFinite(value) || value < 1 || value > 65535 || value === state[key]) {
        render(); // revert to the last known-good value
        return;
    }
    send({ type: messageType, port: value }, applyState);
}

els.proxyPort.addEventListener("change", () =>
    commitPort(els.proxyPort, "proxyPort", "proxy:setPort"));
els.companionPort.addEventListener("change", () =>
    commitPort(els.companionPort, "companionPort", "companion:setPort"));

for (const input of [els.proxyPort, els.companionPort]) {
    input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") input.blur();
        if (e.key === "Escape") render();
    });
}

els.reconnectBtn.addEventListener("click", () => {
    els.reconnectBtn.classList.add("spin");
    send({ type: "companion:reconnect" }, (res) => {
        applyState(res);
        setTimeout(() => els.reconnectBtn.classList.remove("spin"), 600);
    });
});

// ── Live updates ──────────────────────────────────────────────────────────────

chrome.runtime.onMessage.addListener((message) => {
    if (message && message.type === "companion:status") applyState(message);
});

// Light poll so the popup stays correct if the service worker restarted while
// the popup was open and missed a status broadcast.
const pollTimer = setInterval(refresh, 3000);
window.addEventListener("unload", () => clearInterval(pollTimer));

// ── Init ──────────────────────────────────────────────────────────────────────

render();
refresh();
