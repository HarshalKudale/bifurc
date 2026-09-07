/**
 * Bifurc Companion — DevTools Panel Script
 *
 * Captures all network requests via chrome.devtools.network and displays them
 * in a Chrome-style table with search, type filters, and Mock / Save actions.
 */

console.log("[DevTools] Bifurc DevTools panel script loaded");

// Extension context validation
let contextInvalidated = false;
function safeRuntimeMessage(message, callback) {
    if (contextInvalidated) return;
    try {
        chrome.runtime.sendMessage(message, (response) => {
            if (chrome.runtime.lastError) {
                if (chrome.runtime.lastError.message.includes("Extension context invalidated")) {
                    contextInvalidated = true;
                    showToast("⚠ Extension reloaded. Please close and reopen DevTools.", true);
                    console.warn("[DevTools] Extension context invalidated. Close and reopen DevTools.");
                    return;
                }
            }
            if (callback) callback(response);
        });
    } catch (err) {
        if (err.message.includes("Extension context invalidated")) {
            contextInvalidated = true;
            showToast("⚠ Extension reloaded. Please close and reopen DevTools.", true);
            console.warn("[DevTools] Extension context invalidated. Close and reopen DevTools.");
        }
    }
}

// Theme
if (chrome.devtools.panels.themeName !== "dark") {
    document.body.classList.add("light");
}

// State
const allRequests = [];
const MAX = 2000;
let activeType = "all";
let search = "";
let preserve = false;
const pendingMessages = new Map();

// Type detection
function detectType(mimeType, url) {
    const m = (mimeType || "").toLowerCase().split(";")[0].trim();
    if (!m) {
        const ext = (url || "").split("?")[0].split(".").pop().toLowerCase();
        if (["js", "mjs"].includes(ext)) return "js";
        if (ext === "css") return "css";
        if (["png", "jpg", "jpeg", "gif", "svg", "webp", "ico", "avif"].includes(ext)) return "img";
        if (["woff", "woff2", "ttf", "otf", "eot"].includes(ext)) return "font";
        return "other";
    }
    if (m.includes("html")) return "doc";
    if (m.includes("javascript") || m.includes("ecmascript")) return "js";
    if (m === "text/css") return "css";
    if (m.startsWith("image/")) return "img";
    if (m.startsWith("font/") || m.includes("font-") || m === "application/x-font-ttf") return "font";
    if (m.startsWith("video/") || m.startsWith("audio/")) return "media";
    if (m.includes("json") || m.includes("xml") || m === "text/plain"
        || m === "application/x-www-form-urlencoded" || m === "multipart/form-data") return "xhr";
    return "other";
}

// Capture
chrome.devtools.network.onRequestFinished.addListener((entry) => {
    const req = entry.request;
    const res = entry.response;
    const reqH = {};
    for (const h of (req.headers || [])) if (!h.name.startsWith(":")) reqH[h.name] = h.value;
    const resH = {};
    for (const h of (res.headers || [])) if (!h.name.startsWith(":")) resH[h.name] = h.value;
    const reqBody = (req.postData && req.postData.text) ? req.postData.text : "";
    const mime = (res.content && res.content.mimeType)
        ? res.content.mimeType
        : (resH["content-type"] || resH["Content-Type"] || "");

    entry.getContent((body) => {
        const item = {
            id: Date.now().toString(36) + Math.random().toString(36).slice(2),
            method: req.method || "GET",
            url: req.url || "",
            requestHeaders: reqH,
            requestBody: reqBody,
            status: res.status || 0,
            mimeType: mime,
            responseHeaders: resH,
            responseBody: body || "",
            size: (res.content && res.content.size) ? res.content.size : 0,
            time: entry.time || 0,
            type: detectType(mime, req.url),
        };
        allRequests.unshift(item);
        if (allRequests.length > MAX) allRequests.pop();
        render();
    });
});

chrome.devtools.network.onNavigated.addListener(() => {
    if (!preserve) { allRequests.length = 0; render(); }
});

// Helpers
function esc(s) {
    return String(s)
        .replace(/&/g, "&amp;").replace(/</g, "&lt;")
        .replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
function fmtSize(b) {
    if (!b || b <= 0) return "—";
    if (b < 1024) return b + " B";
    if (b < 1048576) return (b / 1024).toFixed(1) + " kB";
    return (b / 1048576).toFixed(1) + " MB";
}
function fmtTime(ms) {
    if (!ms || ms < 0) return "—";
    return ms < 1000 ? Math.round(ms) + " ms" : (ms / 1000).toFixed(2) + " s";
}
function sCls(s) {
    if (!s || s === 0) return "se";
    if (s < 300) return "s2";
    if (s < 400) return "s3";
    if (s < 500) return "s4";
    return "s5";
}
function mCls(m) {
    const map = { GET: "mGET", POST: "mPOST", PUT: "mPUT", DELETE: "mDELETE", PATCH: "mPATCH", HEAD: "mHEAD", OPTIONS: "mOPT" };
    return map[(m || "").toUpperCase()] || "";
}
function dispName(url) {
    try {
        const u = new URL(url);
        const p = u.pathname.split("/").filter(Boolean);
        return p.length ? p[p.length - 1] : u.hostname;
    } catch { return url; }
}
function genId() { return Date.now().toString(36) + Math.random().toString(36).slice(2, 7); }

// Filter
function getFiltered() {
    return allRequests.filter((r) => {
        if (activeType !== "all" && r.type !== activeType) return false;
        if (search && !r.url.toLowerCase().includes(search)) return false;
        return true;
    });
}

// Render
const tbody = document.getElementById("tbody");
const empty = document.getElementById("empty");
const stotal = document.getElementById("stotal");
const sfilt = document.getElementById("sfiltered");
const TYPE_LABEL = { xhr: "fetch", doc: "document", js: "script", img: "image", font: "font", css: "stylesheet", media: "media", ws: "websocket", other: "other" };

function render() {
    const list = getFiltered();
    console.log("[DevTools] Rendering table with", list.length, "requests");
    stotal.textContent = allRequests.length + " request" + (allRequests.length !== 1 ? "s" : "");
    if (list.length !== allRequests.length) {
        sfilt.style.display = ""; sfilt.textContent = list.length + " shown";
    } else { sfilt.style.display = "none"; }

    if (allRequests.length === 0) {
        empty.classList.add("show"); tbody.innerHTML = ""; return;
    }
    empty.classList.remove("show");

    if (list.length === 0) {
        tbody.innerHTML = '<tr><td colspan="7" style="text-align:center;padding:24px;color:var(--dim)">No requests match the current filter.</td></tr>';
        return;
    }

    tbody.innerHTML = list.map((r) => {
        const sTxt = r.status === 0 ? "(failed)" : String(r.status);
        return '<tr data-id="' + r.id + '">' + '<td><div class="ac">'
            + '<button class="ab abm" data-action="mock" data-id="' + r.id + '">Mock</button>'
            + '<button class="ab abs" data-action="save" data-id="' + r.id + '">Save</button>'
            + '</div></td>'
            + '<td class="tn" title="' + esc(r.url) + '">' + esc(dispName(r.url)) + '</td>'
            + '<td><span class="bm ' + mCls(r.method) + '">' + esc(r.method) + '</span></td>'
            + '<td><span class="bs ' + sCls(r.status) + '">' + sTxt + '</span></td>'
            + '<td class="td">' + (TYPE_LABEL[r.type] || r.type) + '</td>'
            + '<td class="tmo">' + fmtSize(r.size) + '</td>'
            + '<td class="tmo">' + fmtTime(r.time) + '</td></tr>';
    }).join("");
}

// Button clicks
tbody.addEventListener("click", (e) => {
    console.log("[DevTools] Click event:", e.target);
    const btn = e.target.closest("button[data-action]");
    if (!btn) {
        console.log("[DevTools] Not a button click");
        return;
    }
    const { action, id } = btn.dataset;
    console.log("[DevTools] Button clicked:", action, id);
    const r = allRequests.find((x) => x.id === id);
    if (!r) {
        console.log("[DevTools] Request not found:", id);
        return;
    }
    console.log("[DevTools] Found request:", r.method, r.url);
    const urlObj = (() => { try { return new URL(r.url); } catch { return null; } })();
    const name = urlObj ? r.method + " " + urlObj.pathname : r.method + " " + r.url;
    if (action === "mock") {
        const msgId = genId();
        console.log("[DevTools] Creating mock message:", msgId);
        pendingMessages.set(msgId, { type: "mock", name });
        const message = {
            type: "companion:send", payload: {
                id: msgId, action: "mock:add", payload: {
                    name, method: r.method,
                    urlPattern: r.url,
                    useRegex: false, enabled: true,
                    capturedHeaders: r.requestHeaders, capturedBody: r.requestBody,
                    responseStatus: r.status || 200,
                    responseHeaders: r.responseHeaders, responseBody: r.responseBody,
                    folderId: null,
                },
            }
        };
        console.log("[DevTools] Sending message to background:", message);
        safeRuntimeMessage(message, (response) => {
            console.log("[DevTools] Background response:", response);
        });
        showToast("Sending to Bifurc as mock...");
    } else if (action === "save") {
        const msgId = genId();
        console.log("[DevTools] Creating request message:", msgId);
        pendingMessages.set(msgId, { type: "request", name });
        const message = {
            type: "companion:send", payload: {
                id: msgId, action: "request:add", payload: {
                    name, method: r.method, url: r.url,
                    headers: r.requestHeaders, body: r.requestBody,
                    folderId: null,
                },
            }
        };
        console.log("[DevTools] Sending message to background:", message);
        safeRuntimeMessage(message, (response) => {
            console.log("[DevTools] Background response:", response);
        });
        showToast("Sending to Bifurc as request...");
    }
});

// Toolbar
document.getElementById("clearBtn").addEventListener("click", () => { allRequests.length = 0; render(); });
document.getElementById("si").addEventListener("input", (e) => { search = e.target.value.toLowerCase(); render(); });
document.getElementById("preserveLog").addEventListener("change", (e) => { preserve = e.target.checked; });

// Pending one-shot callbacks keyed by message id (for folder:add responses)
const pendingCallbacks = new Map();

function sendBulkMocks(visible, folderId) {
    visible.forEach((r) => {
        const urlObj = (() => { try { return new URL(r.url); } catch { return null; } })();
        const name = urlObj ? r.method + " " + urlObj.pathname : r.method + " " + r.url;
        const msgId = genId();
        pendingMessages.set(msgId, { type: "mock", name });
        safeRuntimeMessage({
            type: "companion:send",
            payload: {
                id: msgId, action: "mock:add", payload: {
                    name, method: r.method, urlPattern: r.url,
                    useRegex: false, enabled: true,
                    capturedHeaders: r.requestHeaders, capturedBody: r.requestBody,
                    responseStatus: r.status || 200,
                    responseHeaders: r.responseHeaders, responseBody: r.responseBody,
                    folderId,
                },
            }
        });
    });
}

function sendBulkRequests(visible, folderId) {
    visible.forEach((r) => {
        const urlObj = (() => { try { return new URL(r.url); } catch { return null; } })();
        const name = urlObj ? r.method + " " + urlObj.pathname : r.method + " " + r.url;
        const msgId = genId();
        pendingMessages.set(msgId, { type: "request", name });
        safeRuntimeMessage({
            type: "companion:send",
            payload: {
                id: msgId, action: "request:add", payload: {
                    name, method: r.method, url: r.url,
                    headers: r.requestHeaders, body: r.requestBody,
                    folderId,
                },
            }
        });
    });
}

document.getElementById("mockAllBtn").addEventListener("click", () => {
    const visible = getFiltered();
    if (visible.length === 0) {
        showToast("No requests in current view", true);
        return;
    }
    const folderName = "Captured " + new Date().toLocaleString();
    const folderMsgId = genId();
    pendingCallbacks.set(folderMsgId, (resp) => {
        const folderId = (resp && resp.ok && resp.data) ? resp.data.id : null;
        sendBulkMocks(visible, folderId);
        showToast(`Mocking ${visible.length} request${visible.length !== 1 ? "s" : ""} → "${folderName}"`);
    });
    safeRuntimeMessage({
        type: "companion:send",
        payload: { id: folderMsgId, action: "folder:add", payload: { kind: "mock", name: folderName } }
    });
});

document.getElementById("saveAllBtn").addEventListener("click", () => {
    const visible = getFiltered();
    if (visible.length === 0) {
        showToast("No requests in current view", true);
        return;
    }
    const folderName = "Captured " + new Date().toLocaleString();
    const folderMsgId = genId();
    pendingCallbacks.set(folderMsgId, (resp) => {
        const folderId = (resp && resp.ok && resp.data) ? resp.data.id : null;
        sendBulkRequests(visible, folderId);
        showToast(`Saving ${visible.length} request${visible.length !== 1 ? "s" : ""} → "${folderName}"`);
    });
    safeRuntimeMessage({
        type: "companion:send",
        payload: { id: folderMsgId, action: "folder:add", payload: { kind: "request", name: folderName } }
    });
});

document.getElementById("fbar").addEventListener("click", (e) => {
    const tab = e.target.closest(".ftab");
    if (!tab) return;
    document.querySelectorAll(".ftab").forEach((t) => t.classList.remove("active"));
    tab.classList.add("active");
    activeType = tab.dataset.type;
    render();
});

// Connection status
const cdot = document.getElementById("cdot");
const ctext = document.getElementById("ctext");
function setConn(ok) {
    cdot.className = ok ? "dot ok" : "dot";
    ctext.textContent = ok ? "Bifurc connected" : "Disconnected";
}
function pollConn() {
    if (contextInvalidated) return;
    safeRuntimeMessage({ type: "get:status" }, (r) => {
        if (r) setConn(r.connected);
    });
}
chrome.runtime.onMessage.addListener((msg) => {
    console.log("[DevTools] Received message from background:", msg);
    if (msg.type === "companion:status") setConn(msg.status === "connected");
    if (msg.type === "ws:response") {
        const resp = msg.payload;
        console.log("[DevTools] WebSocket response:", resp);
        // One-shot callbacks (e.g. folder:add responses for bulk operations)
        if (pendingCallbacks.has(resp.id)) {
            const cb = pendingCallbacks.get(resp.id);
            pendingCallbacks.delete(resp.id);
            cb(resp);
            return;
        }
        const pending = pendingMessages.get(resp.id);
        if (pending) {
            console.log("[DevTools] Found pending message:", pending);
            pendingMessages.delete(resp.id);
            if (resp.ok) {
                showToast(`✓ ${pending.type === "mock" ? "Mock" : "Request"} added successfully`);
            } else {
                showToast(`✗ Error: ${resp.error || "Unknown error"}`, true);
            }
        } else {
            console.log("[DevTools] No pending message found for id:", resp.id);
        }
    }
});
pollConn();
setInterval(pollConn, 5000);

// Toast
const toastEl = document.getElementById("toast");
let toastTmr;
function showToast(msg, isError = false) {
    toastEl.textContent = msg;
    toastEl.classList.add("show");
    if (isError) {
        toastEl.style.backgroundColor = "#d32f2f";
    } else {
        toastEl.style.backgroundColor = "";
    }
    clearTimeout(toastTmr);
    toastTmr = setTimeout(() => toastEl.classList.remove("show"), isError ? 4000 : 2000);
}

// Init
render();