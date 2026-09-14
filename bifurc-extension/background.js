/**
 * Bifurc Companion — Background Service Worker
 *
 * Manages:
 * 1. Proxy toggle (route traffic through Bifurc's proxy)
 * 2. WebSocket connection to companion server
 * 3. Context menus for DevTools network panel
 *
 * IMPORTANT — proxy activation rule:
 *   The proxy is applied ONLY when the user toggle is ON **and** the companion
 *   server is connected. If either condition is false we clear the proxy
 *   settings so the browser keeps its normal direct passthrough. Routing
 *   through a proxy whose companion server is down would black-hole traffic,
 *   so the safe default is always "direct".
 */

// ── Default config ────────────────────────────────────────────────────────────

const DEFAULTS = {
    proxyEnabled: false,
    proxyPort: 80,
    companionPort: 9271,
};

// ── State ─────────────────────────────────────────────────────────────────────

let ws = null;
let wsReconnectTimer = null;
let wsReconnectDelay = 1000;
const MAX_RECONNECT_DELAY = 30000;

// Monotonic guard so a slow/racing applyProxy() can never clobber a newer one.
let proxyApplySeq = 0;
// Identity of the proxy config currently pushed to Chrome ("direct" | "pac:<port>").
// null until the first successful apply in this service-worker lifetime.
let appliedProxyKey = null;
// Synchronous mirror of "Chrome is routing regular traffic through our proxy".
// webRequest listeners are sync, so they cannot await getProxyState().
let proxyRouted = false;


// ── Storage helpers ───────────────────────────────────────────────────────────

async function getConfig() {
    const result = await chrome.storage.local.get(DEFAULTS);
    return result;
}

async function setConfig(patch) {
    await chrome.storage.local.set(patch);
}

// ── Connection state ──────────────────────────────────────────────────────────

function isConnected() {
    return !!(ws && ws.readyState === WebSocket.OPEN);
}

/**
 * Resolved, UI-facing state. `proxyActive` is the single source of truth for
 * whether traffic is actually being routed through the proxy right now.
 */
async function getProxyState() {
    const { proxyEnabled, proxyPort, companionPort } = await getConfig();
    const connected = isConnected();
    return {
        connected,
        proxyEnabled: !!proxyEnabled,
        proxyActive: !!proxyEnabled && connected,
        proxyPort,
        companionPort,
    };
}

// ── Proxy management ──────────────────────────────────────────────────────────

function buildPacScript(port) {
    return `function FindProxyForURL(url, host) { return "PROXY 127.0.0.1:${port}"; }`;
}

/**
 * Reconcile Chrome's proxy settings with the current desired state.
 *
 *   proxyEnabled && connected  ->  install PAC script pointing at the proxy
 *   otherwise                  ->  clear proxy settings (direct passthrough)
 */
async function applyProxy() {
    const seq = ++proxyApplySeq;
    const state = await getProxyState();

    // A newer call has already started — let it win.
    if (seq !== proxyApplySeq) return state;

    // Identity of the desired proxy config. Reset to null whenever the service
    // worker restarts, which is what we want: Chrome may still hold settings
    // from a previous lifetime, so the first apply must always run.
    const desiredKey = state.proxyActive ? `pac:${state.proxyPort}` : "direct";

    // Keep the synchronous mirror in step before any early return, so the
    // webRequest listener sees the truth even on the skip path.
    proxyRouted = state.proxyActive;

    if (desiredKey === appliedProxyKey) {
        broadcastState(state);
        return state;
    }

    try {
        if (state.proxyActive) {
            await chrome.proxy.settings.set({
                value: {
                    mode: "pac_script",
                    pacScript: { data: buildPacScript(state.proxyPort) },
                },
                scope: "regular",
            });
        } else {
            await chrome.proxy.settings.clear({ scope: "regular" });
        }
        appliedProxyKey = desiredKey;
        console.log(
            `[Bifurc] Proxy ${state.proxyActive ? "ACTIVE" : "BYPASSED (direct)"}` +
            ` — toggle=${state.proxyEnabled} connected=${state.connected}` +
            (state.proxyActive ? ` port=${state.proxyPort}` : "")
        );
    } catch (err) {
        console.error("[Bifurc] Failed to apply proxy settings:", err);
    }

    broadcastState(state);
    return state;
}

// ── WebSocket connection to companion server ──────────────────────────────────

let connectInFlight = false;

function connectWebSocket() {
    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
        return;
    }
    if (connectInFlight) return;
    connectInFlight = true;

    getConfig().then(({ companionPort }) => {
        connectInFlight = false;

        const url = `ws://127.0.0.1:${companionPort}`;
        let socket;

        try {
            socket = new WebSocket(url);
        } catch (e) {
            scheduleReconnect();
            return;
        }

        ws = socket;

        socket.onopen = () => {
            if (ws !== socket) return; // superseded by a newer socket
            console.log("[Bifurc] Connected to companion server");
            wsReconnectDelay = 1000; // reset backoff
            // Connection is up — the proxy may now be allowed to activate.
            applyProxy();
        };

        socket.onclose = () => {
            // Ignore events from a socket we already replaced, otherwise we
            // would null out the live connection and orphan it.
            if (ws !== socket) return;
            ws = null;
            // Connection is down — immediately fall back to direct passthrough
            // even if the user's toggle is still on.
            applyProxy();
            scheduleReconnect();
        };

        socket.onerror = () => {
            // onerror always fires before onclose
        };

        socket.onmessage = (event) => {
            if (ws !== socket) return;
            console.log("[Bifurc] WebSocket message received:", event.data);
            try {
                const msg = JSON.parse(event.data);
                console.log("[Background] Parsed message:", msg);
                // Forward response to any waiting popup/devtools
                chrome.runtime.sendMessage({ type: "ws:response", payload: msg }).catch((err) => {
                    console.error("[Background] Error forwarding response:", err);
                });
            } catch (err) {
                console.error("[Background] Error parsing WebSocket message:", err);
            }
        };
    }).catch(() => {
        connectInFlight = false;
        scheduleReconnect();
    });
}

function scheduleReconnect() {
    if (wsReconnectTimer) clearTimeout(wsReconnectTimer);
    wsReconnectTimer = setTimeout(() => {
        wsReconnectTimer = null;
        connectWebSocket();
    }, wsReconnectDelay);
    wsReconnectDelay = Math.min(wsReconnectDelay * 2, MAX_RECONNECT_DELAY);
}

/**
 * Push the current connection + proxy state to popup / devtools listeners.
 * Kept on the "companion:status" message type for backward compatibility.
 */
function broadcastState(state) {
    const emit = (s) => {
        chrome.runtime
            .sendMessage({
                type: "companion:status",
                status: s.connected ? "connected" : "disconnected",
                ...s,
            })
            .catch(() => { /* no listeners open — fine */ });
    };

    if (state) emit(state);
    else getProxyState().then(emit).catch(() => { });
}

function sendToCompanion(message) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        console.log("[Bifurc] Sending to companion:", message.action);
        ws.send(JSON.stringify(message));
        return true;
    }
    console.warn("[Bifurc] Cannot send to companion - WebSocket not connected");
    return false;
}

// ── Context menus ─────────────────────────────────────────────────────────────

chrome.runtime.onInstalled.addListener(() => {
    // Remove existing menus to avoid duplicates on update
    chrome.contextMenus.removeAll(() => {
        chrome.contextMenus.create({
            id: "bifurc-mock",
            title: "Mock this request [Bifurc]",
            contexts: ["link"],
        });

        chrome.contextMenus.create({
            id: "bifurc-request",
            title: "Add to Requests [Bifurc]",
            contexts: ["link"],
        });
    });
});

// ── Message handler (from popup and devtools) ─────────────────────────────────

async function handleMessage(message) {
    console.log("[Background] Received message:", message.type, message);
    switch (message.type) {
        case "get:status":
            return await getProxyState();

        case "proxy:toggle":
            await setConfig({ proxyEnabled: !!message.enabled });
            return await applyProxy();

        case "proxy:setPort":
            await setConfig({ proxyPort: message.port });
            return await applyProxy();

        case "companion:setPort":
            await setConfig({ companionPort: message.port });
            // Reconnect with new port. Closing triggers onclose -> applyProxy
            // (direct), then onopen -> applyProxy (active) if it reconnects.
            if (ws) ws.close();
            wsReconnectDelay = 1000;
            connectWebSocket();
            return await getProxyState();

        case "companion:send": {
            // Forward a command to companion server
            console.log("[Background] Forwarding to companion:", message.payload);
            const sent = sendToCompanion(message.payload);
            console.log("[Background] Send result:", sent);
            return { sent };
        }

        case "companion:reconnect":
            if (ws) ws.close();
            wsReconnectDelay = 1000;
            connectWebSocket();
            return await getProxyState();
    }
    return undefined;
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    handleMessage(message, sender)
        .then((response) => sendResponse(response ?? { ok: false }))
        .catch((err) => {
            console.error("[Background] Message handler error:", err);
            sendResponse({ ok: false, error: String(err) });
        });
    return true; // keep the channel open for the async response
});

// ── Network panel context menu click handler ──────────────────────────────────

chrome.contextMenus.onClicked.addListener((info) => {
    // Chrome provides info.linkUrl = the URL of the right-clicked network request.
    // That is the only data available without capturing requests.
    const requestUrl = info.linkUrl;
    if (!requestUrl) return;

    if (info.menuItemId === "bifurc-mock") {
        handleMockRequest(requestUrl);
    } else if (info.menuItemId === "bifurc-request") {
        handleAddRequest(requestUrl);
    }
});

// ── DevTools request handlers ─────────────────────────────────────────────────

function generateId() {
    return Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
}

function handleMockRequest(url) {
    const urlObj = (() => { try { return new URL(url); } catch { return null; } })();

    const payload = {
        id: generateId(),
        action: "mock:add",
        payload: {
            name: urlObj ? `GET ${urlObj.pathname}` : `GET ${url}`,
            method: "GET",
            urlPattern: url,
            useRegex: false,
            enabled: true,
            capturedHeaders: {},
            capturedBody: "",
            responseStatus: 200,
            responseHeaders: {},
            responseBody: "",
            folderId: null,
        },
    };

    sendToCompanion(payload);
}

function handleAddRequest(url) {
    const urlObj = (() => { try { return new URL(url); } catch { return null; } })();

    const payload = {
        id: generateId(),
        action: "request:add",
        payload: {
            name: urlObj ? `GET ${urlObj.pathname}` : `GET ${url}`,
            method: "GET",
            url,
            headers: {},
            body: "",
            folderId: null,
        },
    };

    sendToCompanion(payload);
}

// ── Proxy failure handling ────────────────────────────────────────────────────
//
// The companion server can be connected over WebSocket while the proxy server
// inside the app is switched off. In that case every proxied request dies with
// a proxy-level network error and Chrome shows a bare error page. We catch that
// here and send the tab to a help page instead, explaining the likely cause.
//
// Only main_frame navigations can be replaced with HTML — a failed sub-resource
// (XHR, image, script) has nowhere to render a document.

// Chrome documents the `error` string as not stable across releases, so match on
// the distinctive fragments rather than the full "net::" prefix.
const PROXY_FAILURE_RE =
    /PROXY_CONNECTION_FAILED|TUNNEL_CONNECTION_FAILED|MANDATORY_PROXY_CONFIGURATION_FAILED/i;

// Per-tab cooldown so a burst of failures cannot spam redirects.
const REDIRECT_COOLDOWN_MS = 1500;
const recentRedirects = new Map(); // tabId -> timestamp

const NON_HTTP_SCHEME_RE = /^(chrome|chrome-extension|edge|about|devtools|view-source|data|blob):/i;

function handleRequestError(details) {
    // A failed sub-resource cannot display a document.
    if (details.type !== "main_frame") return;
    // A failure with the proxy switched off is just a normal network error —
    // we must not claim the proxy caused it.
    if (!proxyRouted) return;
    if (!PROXY_FAILURE_RE.test(details.error || "")) return;
    if (details.tabId == null || details.tabId < 0) return;
    if (NON_HTTP_SCHEME_RE.test(details.url || "")) return;

    const now = Date.now();
    // Opportunistic cleanup so the map cannot grow without bound.
    for (const [tabId, at] of recentRedirects) {
        if (now - at > 60000) recentRedirects.delete(tabId);
    }
    if (now - (recentRedirects.get(details.tabId) || 0) < REDIRECT_COOLDOWN_MS) return;
    recentRedirects.set(details.tabId, now);

    console.log(
        `[Bifurc] Proxied request failed (${details.error}) — showing proxy help page`
    );

    const target = chrome.runtime.getURL(
        "proxy-error.html?url=" + encodeURIComponent(details.url || "")
    );
    chrome.tabs.update(details.tabId, { url: target }).catch((err) => {
        console.error("[Bifurc] Could not open the proxy help page:", err);
    });
}

chrome.webRequest.onErrorOccurred.addListener(handleRequestError, {
    urls: ["<all_urls>"],
});

// ── Lifecycle ─────────────────────────────────────────────────────────────────

// Apply proxy settings on startup. Because the WebSocket is not open yet this
// resolves to "direct" — the proxy is enabled automatically once we connect.
applyProxy();

// Connect to companion server
connectWebSocket();

// Handle alarm for keep-alive (service workers can be killed)
chrome.alarms?.create("keepalive", { periodInMinutes: 0.5 });
chrome.alarms?.onAlarm.addListener((alarm) => {
    if (alarm.name === "keepalive") {
        if (!isConnected()) {
            connectWebSocket();
        }
    }
});
