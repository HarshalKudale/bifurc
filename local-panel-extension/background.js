/**
 * Bifurc Companion — Background Service Worker
 *
 * Manages:
 * 1. Proxy toggle (route traffic through Bifurc's proxy)
 * 2. WebSocket connection to companion server
 * 3. Context menus for DevTools network panel
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


// ── Storage helpers ───────────────────────────────────────────────────────────

async function getConfig() {
    const result = await chrome.storage.local.get(DEFAULTS);
    return result;
}

async function setConfig(patch) {
    await chrome.storage.local.set(patch);
}

// ── Proxy management ──────────────────────────────────────────────────────────

function buildPacScript(port) {
    return `function FindProxyForURL(url, host) { return "PROXY 127.0.0.1:${port}"; }`;
}

async function applyProxy() {
    const { proxyEnabled, proxyPort } = await getConfig();

    if (proxyEnabled) {
        const config = {
            mode: "pac_script",
            pacScript: {
                data: buildPacScript(proxyPort),
            },
        };
        chrome.proxy.settings.set({ value: config, scope: "regular" });
    } else {
        chrome.proxy.settings.clear({ scope: "regular" });
    }
}

// ── WebSocket connection to companion server ──────────────────────────────────

function connectWebSocket() {
    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
        return;
    }

    getConfig().then(({ companionPort }) => {
        const url = `ws://127.0.0.1:${companionPort}`;

        try {
            ws = new WebSocket(url);
        } catch (e) {
            scheduleReconnect();
            return;
        }

        ws.onopen = () => {
            console.log("[Bifurc] Connected to companion server");
            wsReconnectDelay = 1000; // reset backoff
            broadcastStatus("connected");
        };

        ws.onclose = () => {
            ws = null;
            broadcastStatus("disconnected");
            scheduleReconnect();
        };

        ws.onerror = () => {
            // onerror always fires before onclose
        };

        ws.onmessage = (event) => {
            console.log("[Background] WebSocket message received:", event.data);
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

function broadcastStatus(status) {
    chrome.runtime.sendMessage({ type: "companion:status", status }).catch(() => { });
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

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    console.log("[Background] Received message:", message.type, message);
    switch (message.type) {
        case "get:status":
            sendResponse({
                connected: ws && ws.readyState === WebSocket.OPEN,
            });
            return false;

        case "proxy:toggle":
            setConfig({ proxyEnabled: message.enabled }).then(() => {
                applyProxy();
                sendResponse({ ok: true });
            });
            return true; // async response

        case "proxy:setPort":
            setConfig({ proxyPort: message.port }).then(() => {
                applyProxy();
                sendResponse({ ok: true });
            });
            return true;

        case "companion:setPort":
            setConfig({ companionPort: message.port }).then(() => {
                // Reconnect with new port
                if (ws) ws.close();
                connectWebSocket();
                sendResponse({ ok: true });
            });
            return true;

        case "companion:send":
            // Forward a command to companion server
            console.log("[Background] Forwarding to companion:", message.payload);
            const sent = sendToCompanion(message.payload);
            console.log("[Background] Send result:", sent);
            sendResponse({ sent });
            return false;

        case "companion:reconnect":
            if (ws) ws.close();
            wsReconnectDelay = 1000;
            connectWebSocket();
            sendResponse({ ok: true });
            return false;
    }
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

// ── Lifecycle ─────────────────────────────────────────────────────────────────

// Apply proxy settings on startup
applyProxy();

// Connect to companion server
connectWebSocket();

// Handle alarm for keep-alive (service workers can be killed)
chrome.alarms?.create("keepalive", { periodInMinutes: 0.5 });
chrome.alarms?.onAlarm.addListener((alarm) => {
    if (alarm.name === "keepalive") {
        if (!ws || ws.readyState !== WebSocket.OPEN) {
            connectWebSocket();
        }
    }
});
