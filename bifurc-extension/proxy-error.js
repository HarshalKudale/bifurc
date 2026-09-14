/**
 * Bifurc Companion — proxy failure help page.
 *
 * Opened by the background worker when a proxied main-frame request fails
 * (see handleRequestError in background.js). Explains the likely cause and
 * offers two ways out: retry, or switch proxy mode off.
 *
 * `chrome.*` is absent when this page is opened outside the extension, so every
 * lookup degrades to a static render instead of throwing.
 */

const els = {
    lead: document.getElementById("lead"),
    proxyAddr: document.getElementById("proxyAddr"),
    proxyAddrRow: document.getElementById("proxyAddrRow"),
    failedUrl: document.getElementById("failedUrl"),
    conn: document.getElementById("conn"),
    connText: document.getElementById("connText"),
    retryBtn: document.getElementById("retryBtn"),
    disableBtn: document.getElementById("disableBtn"),
    status: document.getElementById("status"),
};

const hasChrome = typeof chrome !== "undefined" && !!chrome.runtime && !!chrome.runtime.id;

const failedUrl = (() => {
    try {
        return new URLSearchParams(location.search).get("url") || "";
    } catch {
        return "";
    }
})();

let proxyPort = 80;

function send(message, callback) {
    if (!hasChrome) return;
    try {
        chrome.runtime.sendMessage(message, (response) => {
            if (chrome.runtime.lastError) return;
            if (callback) callback(response);
        });
    } catch {
        /* extension context invalidated */
    }
}

// ── Render ────────────────────────────────────────────────────────────────────

function setProxyAddress(port) {
    if (Number.isFinite(port)) proxyPort = port;
    const addr = `127.0.0.1:${proxyPort}`;
    els.proxyAddr.textContent = addr;
    els.proxyAddrRow.textContent = addr;
}

function renderState(state) {
    const connected = !!(state && state.connected);

    els.conn.classList.toggle("on", connected);
    els.connText.textContent = connected ? "Connected" : "Not connected";

    if (state && Number.isFinite(state.proxyPort)) setProxyAddress(state.proxyPort);

    // The advice depends on whether the companion server is reachable at all.
    els.lead.innerHTML = connected
        ? 'The companion app is connected, so the <strong>proxy server inside it is ' +
        'probably turned off</strong>. Turn the proxy server on in the companion app, then ' +
        'try again &mdash; or switch off proxy mode below to browse directly.'
        : 'The companion server is not reachable either, so Bifurc cannot route traffic right ' +
        'now. Start the companion app, then try again &mdash; or switch off proxy mode below ' +
        'to browse directly.';
}

function setBusy(busy) {
    els.retryBtn.disabled = busy;
    els.disableBtn.disabled = busy;
}

// ── Actions ───────────────────────────────────────────────────────────────────

function goBackToTarget() {
    if (failedUrl) location.replace(failedUrl);
    else history.back();
}

els.retryBtn.addEventListener("click", () => {
    setBusy(true);
    els.status.textContent = "Retrying…";
    goBackToTarget();
});

els.disableBtn.addEventListener("click", () => {
    setBusy(true);
    els.status.textContent = "Turning off proxy mode…";

    send({ type: "proxy:toggle", enabled: false }, (state) => {
        if (!state) {
            setBusy(false);
            els.status.textContent = "Could not reach the extension. Reload the page to retry.";
            return;
        }
        if (state.proxyActive) {
            setBusy(false);
            els.status.textContent = "Proxy mode is still on. Try again, or switch it off from the Bifurc icon.";
            return;
        }
        els.status.textContent = "Proxy mode is off — loading directly…";
        goBackToTarget();
    });
});

// ── Init ──────────────────────────────────────────────────────────────────────

els.failedUrl.textContent = failedUrl || "Unknown";
setProxyAddress(80);
setBusy(false);

send({ type: "get:status" }, renderState);
