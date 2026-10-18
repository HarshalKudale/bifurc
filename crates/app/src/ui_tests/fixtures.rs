//! Canned engine data the UI tests share.

use serde_json::{Value, json};

use crate::test_support::FakeEngine;

/// A workspace with something of everything: requests in a folder, a mock, a mapping, an environment, a flow
/// and a proxy rule.
pub fn config() -> Value {
    json!({
        "environments": [
            {"id": "__global__", "name": "Global", "variables": []},
            {"id": "e1", "name": "Staging", "variables": [{"id": "v1", "key": "HOST", "value": "stg.test"}]},
            {"id": "e2", "name": "Production", "variables": []},
            {"id": "e3", "name": "Newer", "variables": [], "locked": true, "requiredVersion": "9.9.9"},
            {"id": "e4", "name": "Retired", "variables": [], "deleted": true},
        ],
        "activeEnvironmentId": "e1",
        "mappings": [
            {"id": "m1", "domain": "api.localhost", "target": "localhost:3000", "enabled": true},
            {"id": "m2", "domain": "old.localhost", "target": "localhost:4000", "enabled": false, "deleted": true},
            {"id": "m3", "domain": "new.localhost", "target": "localhost:5000", "enabled": true, "locked": true, "requiredVersion": "9.9.9"},
        ],
        "requests": [
            {"id": "r1", "name": "List users", "method": "GET", "url": "https://api.test/users", "folderId": "f1"},
            {"id": "r2", "name": "Ping", "method": "POST", "url": "https://api.test/ping"},
            {"id": "r3", "name": "Gone", "method": "GET", "url": "https://api.test/gone", "deleted": true},
            {"id": "r4", "name": "Newer", "method": "GET", "url": "https://api.test/newer", "locked": true, "requiredVersion": "9.9.9"},
        ],
        "requestFolders": [{"id": "f1", "name": "Users", "parentId": null}],
        "mocks": [
            {"id": "k1", "name": "Users mock", "method": "GET", "urlPattern": "/users", "enabled": true},
            {"id": "k2", "name": "Orders mock", "method": "POST", "urlPattern": "/orders", "enabled": false},
            {"id": "k3", "name": "Retired mock", "method": "GET", "urlPattern": "/old", "enabled": false, "deleted": true},
            {"id": "k4", "name": "Newer mock", "method": "GET", "urlPattern": "/new", "enabled": true, "locked": true, "requiredVersion": "9.9.9"},
        ],
        "flows": [{"id": "fl1", "name": "Checkout flow"}],
        "proxyRules": [
            {"id": "p1", "name": "Rewrite", "pattern": "api.test"},
            {"id": "p3", "name": "Old rule", "pattern": "old.test", "deleted": true},
            {"id": "p4", "name": "Newer rule", "pattern": "newer.test", "locked": true, "requiredVersion": "9.9.9"},
        ],
    })
}

/// The saved REST request `r1`, as `entity.load` returns it.
pub fn rest_request() -> Value {
    json!({
        "id": "r1", "name": "List users", "method": "GET", "url": "https://api.test/users",
        "folderId": "f1", "headers": [], "params": [], "body": {"type": "none"},
    })
}

/// Opens the fake on [`config`] with the entities behind it loadable and every list screen's data in place.
pub fn populated(fake: &FakeEngine) {
    fake.config(config());
    fake.entity("requests", rest_request());
    fake.entity(
        "requests",
        json!({
            "id": "r2", "name": "Ping", "method": "POST", "url": "https://api.test/ping",
            "headers": [], "params": [], "body": {"type": "none"},
        }),
    );
    fake.entity(
        "mocks",
        json!({
            "id": "k1", "name": "Users mock", "method": "GET", "urlPattern": "/users", "enabled": true,
            "statusCode": 200, "responseBody": "[]", "responseHeaders": [],
        }),
    );
    fake.entity(
        "environments",
        json!({
            "id": "e1", "name": "Staging", "variables": [{"id": "v1", "key": "HOST", "value": "stg.test"}],
        }),
    );
    fake.entity("flows", json!({"id": "fl1", "name": "Checkout flow", "nodes": [], "connections": []}));
    fake.reply("entity.list", json!({"entities": [
        {"id": "p1", "name": "Rewrite", "pattern": "api.test", "enabled": true, "targetType": "external", "targetExternal": "http://localhost:9000"},
        {"id": "p2", "name": "Mapped", "pattern": "web.test", "enabled": false, "targetType": "mapping", "targetMappingId": "m1"},
    ]}));
    fake.reply(
        "services.discover",
        json!([
            {"port": 3000, "address": "127.0.0.1", "pid": 11, "processName": "node"},
            {"port": 5432, "address": "127.0.0.1", "pid": 12, "processName": "postgres"},
        ]),
    );
    fake.reply(
        "devservice.list",
        json!({"services": [
            {"name": "api", "cmd": "npm run api", "port": 3000, "env": {"A": "1"}, "state": "running", "pid": 21, "listening": true},
            {"name": "db", "cmd": "pg start", "vault": "Locked", "state": "stopped"},
            {"name": "web", "cmd": "npm run dev", "port": 5173, "vault": "Prod", "state": "stopped"},
            {"name": "worker", "cmd": "node worker.js", "state": "crashed", "exitCode": 2},
        ]}),
    );
    fake.reply(
        "healthbar.getServices",
        json!([
            {"id": "h1", "name": "API", "url": "http://localhost:3000/health"},
            {"id": "h2", "name": "DB", "url": "http://localhost:5432"},
        ]),
    );
    fake.reply("healthbar.checkUrl", json!({"ok": true, "statusCode": 200, "body": "ok", "durationMs": 5}));
    fake.reply("capture.list", json!({"paused": false, "disableCache": true, "entries": [
        {"id": "c1", "timestamp": 1, "method": "GET", "url": "https://api.test/users", "status": 200, "via": "proxy"},
        {"id": "c2", "timestamp": 2, "method": "POST", "url": "https://api.test/orders", "status": 500, "via": "mock"},
    ]}));
    fake.reply("audit.list", json!({"total": 2, "entries": [
        {"commitHash": "aaa111", "ts": 1, "action": "create", "entity": "requests", "entityName": "Ping", "message": "Add Ping", "actor": "me", "changedFields": ["name"]},
        {"commitHash": "bbb222", "ts": 2, "action": "update", "entity": "mocks", "entityName": "Users mock", "message": "Edit mock", "actor": "me", "changedFields": []},
    ]}));
    fake.reply("vault.list", json!([
        {"id": "vt1", "name": "Prod", "state": "unsealed", "secrets": ["API_KEY", "DB_PASS"], "path": "vaults/prod.json"},
        {"id": "vt2", "name": "Locked", "state": "sealed", "secrets": [], "path": "vaults/locked.json"},
    ]));
}
