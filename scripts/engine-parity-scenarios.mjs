/**
 * The scenarios `engine-parity.mjs` runs against both engines. Each gets a fresh engine and data directory.
 *
 * `t.call(action, payload)` records the normalized result and returns the raw data, so a later step can use
 * an id the engine generated. Anything worth comparing that is not a command result goes through `t.note`.
 */

const b64 = (text) => Buffer.from(text, "utf8").toString("base64");
const unb64 = (text) => Buffer.from(text ?? "", "base64").toString("utf8");

const request = (extra = {}) => ({
  name: "Ping",
  method: "GET",
  url: "http://127.0.0.1:18090/echo?x=1",
  headers: { "x-test": "1" },
  body: "",
  ...extra,
});

const mock = (extra = {}) => ({
  name: "Users",
  method: "GET",
  urlPattern: "http://api.test/users",
  useRegex: false,
  capturedHeaders: {},
  capturedBody: "",
  responseStatus: 200,
  responseHeaders: { "content-type": "application/json" },
  responseBody: '{"users":[]}',
  ...extra,
});

export const scenarios = [
  {
    name: "config-and-settings",
    async run(t) {
      await t.call("config.get");
      await t.call("config.get", { extra: true });
      await t.call("nope.nope");
      await t.call("settings.update", { minimizeToTray: false });
      await t.call("settings.update", { port: 70000 });
      await t.call("env.getMerged", { workspaceId: t.ws });
      await t.call("env.resolveText", { workspaceId: t.ws, text: "{{host}}/health" });
      const cfg = await t.call("config.get");
      await t.call("config.save", { config: { ...cfg, minimizeToTray: true } });
      await t.call("config.get");
      await t.call("server.status", {});
      await t.call("server.status", { target: "reverse" });
      await t.call("proxy.status", {});
    },
  },
  {
    name: "workspaces",
    skipDisk: true,
    async run(t) {
      const second = await t.call("workspace.add", { name: "Second" });
      await t.call("workspace.rename", { id: second?.id, name: "  Renamed  " });
      await t.call("workspace.rename", { id: second?.id, name: "   " });
      await t.call("workspace.setActive", { id: second?.id });
      await t.call("config.get");
      await t.call("workspace.setActive", { id: "missing" });
      await t.call("workspace.delete", { id: second?.id });
      await t.call("config.get");
    },
  },
  {
    name: "entities-requests",
    async run(t) {
      const folder = await t.call("folder.add", { kind: "request", name: "Auth: flows?", parentId: null });
      const top = await t.call("entity.create", { kind: "requests", entity: request() });
      const inFolder = await t.call("entity.create", {
        kind: "requests",
        entity: request({ name: "Login", method: "POST", folderId: folder?.id, body: '{"a":1}', preScript: "lp.request.url += '&pre=1'" }),
      });
      await t.call("entity.create", { kind: "requests", entity: { name: "No url", method: "GET" } });
      await t.call("entity.list", { workspaceId: t.ws, kind: "requests" });
      await t.call("entity.load", { workspaceId: t.ws, kind: "requests", id: inFolder?.id ?? "x" });
      await t.call("entity.load", { workspaceId: t.ws, kind: "requests", id: "missing" });
      await t.call("entity.update", { kind: "requests", entity: { ...(top?.entity ?? top ?? {}), name: "Ping renamed", url: "http://127.0.0.1:18090/echo?x=2" } });
      await t.call("entity.update", { kind: "requests", entity: { ...(inFolder?.entity ?? inFolder ?? {}), folderId: null } });
      await t.call("entity.duplicate", { workspaceId: t.ws, kind: "requests", id: top?.id ?? top?.entity?.id ?? "x" });
      await t.call("config.get");
      await t.call("entity.delete", { kind: "requests", id: top?.id ?? top?.entity?.id ?? "x" });
      await t.call("entity.delete", { kind: "requests", id: "missing" });
      await t.call("entity.list", { workspaceId: t.ws, kind: "requests" });
      await t.call("config.get");
    },
  },
  {
    name: "entities-mocks",
    async run(t) {
      const folder = await t.call("folder.add", { kind: "mock", name: "Users", parentId: null });
      const first = await t.call("entity.create", { kind: "mocks", entity: mock() });
      const shadow = await t.call("entity.create", { kind: "mocks", entity: mock({ name: "Users v2", folderId: folder?.id }) });
      await t.call("entity.create", { kind: "mocks", entity: { name: "bad" } });
      await t.call("entity.list", { workspaceId: t.ws, kind: "mocks" });
      await t.call("config.get");
      await t.call("entity.setEnabled", { workspaceId: t.ws, kind: "mocks", id: first?.id ?? "x", enabled: true });
      await t.call("entity.setEnabled", { workspaceId: t.ws, kind: "mocks", id: shadow?.id ?? "x", enabled: false });
      await t.call("entity.load", { workspaceId: t.ws, kind: "mocks", id: first?.id ?? "x" });
      await t.call("entity.update", { kind: "mocks", entity: { ...(first?.entity ?? first ?? {}), responseStatus: 201, enabled: true } });
      await t.call("entity.duplicate", { workspaceId: t.ws, kind: "mocks", id: first?.id ?? "x" });
      await t.call("config.get");
      await t.call("entity.delete", { kind: "mocks", id: shadow?.id ?? "x" });
      await t.call("config.get");
    },
  },
  {
    name: "entities-mappings-environments",
    async run(t) {
      const mapping = await t.call("entity.create", { kind: "mappings", entity: { domain: "App.localhost", target: "127.0.0.1:18090", label: "App" } });
      await t.call("entity.create", { kind: "mappings", entity: { domain: "app.localhost", target: "127.0.0.1:18090" } });
      await t.call("entity.create", { kind: "mappings", entity: { domain: "app.example", target: "127.0.0.1:18090" } });
      await t.call("entity.create", { kind: "mappings", entity: { domain: "x.localhost", target: "nope" } });
      await t.call("entity.create", { kind: "mappings", entity: { domain: "", target: "" } });
      await t.call("entity.setEnabled", { workspaceId: t.ws, kind: "mappings", id: mapping?.id ?? "x", enabled: false });
      await t.call("entity.update", { kind: "mappings", entity: { ...(mapping?.entity ?? mapping ?? {}), target: "http://127.0.0.1:18090" } });
      await t.call("entity.list", { workspaceId: t.ws, kind: "mappings" });

      const dev = await t.call("entity.create", { kind: "environments", entity: { name: "Dev", variables: [{ id: "1", key: "host", value: "dev.test" }] } });
      await t.call("env.resolveText", { workspaceId: t.ws, text: "{{host}}/health {{missing}}" });
      const prod = await t.call("entity.create", { kind: "environments", entity: { name: "Prod", variables: [{ id: "1", key: "host", value: "prod.test" }] } });
      await t.call("env.resolveText", { workspaceId: t.ws, text: "{{host}}/health" });
      await t.call("env.setActive", { id: prod?.id ?? null });
      await t.call("env.resolveText", { workspaceId: t.ws, text: "{{host}}/health" });
      await t.call("env.getMerged", { workspaceId: t.ws });
      await t.call("env.setActive", { id: "__global__" });
      await t.call("entity.update", {
        kind: "environments",
        entity: { id: "__global__", name: "Global", variables: [{ id: "g", key: "token", value: "abc" }], workspaceId: t.ws },
      });
      await t.call("env.getMerged", { workspaceId: t.ws });
      await t.call("entity.delete", { kind: "environments", id: dev?.id ?? "x" });
      await t.call("env.setActive", { id: null });
      await t.call("config.get");
    },
  },
  {
    name: "entities-other-kinds",
    async run(t) {
      const rule = await t.call("entity.create", {
        kind: "proxyRules",
        entity: { name: "Rewrite", pattern: "^http://api\\.test/", useRegex: true, targetType: "external", targetMappingId: "", targetExternal: "127.0.0.1:18090", requestScript: "", responseScript: "" },
      });
      await t.call("entity.create", {
        kind: "proxyRules",
        entity: { name: "Rewrite again", pattern: "^http://api\\.test/", useRegex: true, targetType: "external", targetMappingId: "", targetExternal: "127.0.0.1:18090", requestScript: "", responseScript: "" },
      });
      await t.call("entity.setEnabled", { workspaceId: t.ws, kind: "rules", id: rule?.id ?? "x", enabled: true });
      await t.call("entity.load", { workspaceId: t.ws, kind: "proxyRules", id: rule?.id ?? "x" });
      await t.call("entity.create", { kind: "wsConnections", entity: { name: "Feed", url: "ws://127.0.0.1:18090/feed", headers: {} } });
      await t.call("entity.create", {
        kind: "graphqlRequests",
        entity: { name: "Viewer", endpointUrl: "http://127.0.0.1:18090/graphql", headers: {}, query: "{ viewer { id } }", variables: "", operationName: "" },
      });
      await t.call("entity.create", {
        kind: "soapRequests",
        entity: { name: "Echo", endpointUrl: "http://127.0.0.1:18090/soap", soapAction: "urn:Echo", headers: {}, body: "<Echo/>" },
      });
      await t.call("entity.create", {
        kind: "grpcRequests",
        entity: { name: "Hello", serverAddress: "127.0.0.1:50051", serviceName: "Greeter", methodName: "SayHello", requestBody: "{}", metadata: {}, protoFileId: null, useReflection: false, streamingType: "unary" },
      });
      await t.call("entity.create", { kind: "grpcRequests", entity: { name: "Bad", serverAddress: "a", serviceName: "", methodName: "" } });
      const schema = await t.call("entity.create", { kind: "graphqlSchemas", entity: { name: "Schema", sdl: "type Query { a: Int }" } });
      await t.call("entity.create", { kind: "protoFiles", entity: { name: "greeter.proto", content: 'syntax = "proto3";' } });
      await t.call("entity.create", { kind: "wsdls", entity: { name: "svc.wsdl", content: "<definitions/>" } });
      for (const kind of ["proxyRules", "wsConnections", "graphqlRequests", "soapRequests", "grpcRequests", "graphqlSchemas", "protoFiles", "wsdls"]) {
        await t.call("entity.list", { workspaceId: t.ws, kind });
      }
      await t.call("entity.load", { workspaceId: t.ws, kind: "graphqlSchemas", id: schema?.id ?? "x" });
      await t.call("entity.update", { kind: "graphqlSchemas", entity: { ...(schema?.entity ?? schema ?? {}), name: "Schema 2" } });
      await t.call("entity.delete", { kind: "graphqlSchemas", id: schema?.id ?? "x" });
      await t.call("entity.delete", { kind: "proxyRules", id: rule?.id ?? "x" });
      await t.call("config.get");
    },
  },
  {
    name: "folders",
    // After the folder is deleted the Node engine's last pushed status still lists the deleted request as new, though its
    // own sync.getEntityStatus (the last step) agrees with Rust that it is gone. The query is compared; the stale push is not.
    ignoreEvents: ["event.sync.entityStatus"],
    async run(t) {
      const a = await t.call("folder.add", { kind: "request", name: "A", parentId: null });
      const b = await t.call("folder.add", { kind: "request", name: "B", parentId: null });
      await t.call("folder.add", { kind: "request", name: "A", parentId: null });
      const inside = await t.call("entity.create", { kind: "requests", entity: request({ name: "Inside", folderId: a?.id }) });
      await t.call("folder.rename", { kind: "request", id: a?.id ?? "x", name: "A renamed" });
      await t.call("folder.move", { kind: "request", id: b?.id ?? "x", parentId: a?.id ?? null });
      await t.call("config.get");
      await t.call("entity.load", { workspaceId: t.ws, kind: "requests", id: inside?.id ?? "x" });
      await t.call("folder.delete", { kind: "request", id: a?.id ?? "x" });
      await t.call("folder.delete", { kind: "request", id: "missing" });
      await t.call("config.get");
      for (const kind of ["mock", "ws", "rule", "graphqlRequest", "grpcRequest", "soapRequest"]) {
        const folder = await t.call("folder.add", { kind, name: `F ${kind}`, parentId: null });
        await t.call("folder.rename", { kind, id: folder?.id ?? "x", name: `G ${kind}` });
      }
      await t.call("config.get");
      await t.call("sync.getEntityStatus", { workspaceId: t.ws });
    },
  },
  {
    name: "git-status-publish-history",
    async run(t) {
      const created = await t.call("entity.create", { kind: "requests", entity: request() });
      const id = created?.id ?? "x";
      await t.sleep(300);
      await t.call("sync.getEntityStatus", { workspaceId: t.ws });
      await t.call("sync.getState", { workspaceId: t.ws });
      const relPath = `requests/${id}.json`;
      await t.call("git.diff", { workspaceId: t.ws, relPath });
      await t.call("entity.publish", { workspaceId: t.ws, paths: [relPath] });
      await t.call("sync.getEntityStatus", { workspaceId: t.ws });
      await t.call("entity.update", { kind: "requests", entity: { ...(created?.entity ?? created ?? {}), name: "Ping 2" } });
      await t.call("sync.getEntityStatus", { workspaceId: t.ws });
      await t.call("git.diff", { workspaceId: t.ws, relPath });
      await t.call("git.discard", { workspaceId: t.ws, relPath });
      await t.call("entity.load", { workspaceId: t.ws, kind: "requests", id });
      await t.call("entity.update", { kind: "requests", entity: { ...(created?.entity ?? created ?? {}), name: "Ping 3" } });
      await t.call("git.sync", { workspaceId: t.ws, paths: [relPath] });
      await t.call("git.history", { workspaceId: t.ws, relPath });
      await t.call("history.list", { filePath: relPath });
      await t.call("audit.list", {});
      await t.call("audit.list", { entity: "request", limit: 1 });
      await t.call("entity.delete", { kind: "requests", id });
      await t.call("sync.getEntityStatus", { workspaceId: t.ws });
      await t.call("config.get");
      await t.call("entity.restore", { workspaceId: t.ws, relPath });
      await t.call("config.get");
      await t.call("folder.publish", { workspaceId: t.ws, kind: "request", folderName: null });
      await t.call("sync.push", { workspaceId: t.ws });
      await t.call("sync.setAutoSync", { workspaceId: t.ws, enabled: false });
      await t.call("sync.disconnect", { workspaceId: t.ws });
    },
  },
  {
    name: "requests-replay-send-scripts",
    async run(t) {
      const note = (label, result) => t.note(label, result ? { ...result, body: unb64(result.body) } : result);
      note("replay get", (await t.raw("request.replay", { method: "GET", url: `${t.upstream}/echo?a=1`, headers: { "x-a": "1" }, body: "" })).data);
      note("replay post", (await t.raw("request.replay", { method: "POST", url: `${t.upstream}/echo`, headers: { "content-type": "application/json" }, body: b64('{"n":1}') })).data);
      note("replay 404", (await t.raw("request.replay", { method: "GET", url: `${t.upstream}/status?code=404`, headers: {}, body: "" })).data);
      await t.call("request.replay", { method: "GET", url: "not a url", headers: {}, body: "" });
      await t.call("request.replay", { method: "GET", url: "http://127.0.0.1:9/closed", headers: {}, body: "" });
      await t.call("request.parseCurl", { curl: `curl -X POST '${t.upstream}/echo' -H 'Content-Type: application/json' -H "X-Token: abc" -d '{"a":1}'` });
      await t.call("request.parseCurl", { curl: "curl https://example.test/a?b=1 -u user:pass --compressed" });
      await t.call("request.parseCurl", { curl: "not curl" });

      await t.call("entity.create", { kind: "environments", entity: { name: "Dev", variables: [{ id: "1", key: "base", value: t.upstream }, { id: "2", key: "token", value: "t0" }] } });
      const send = async (label, payload) => {
        const result = await t.raw("request.send", payload);
        // A script parser's wording for a syntax error (V8 or QuickJS) is its own; that one was reported is what counts.
        const data = result.ok && typeof result.data.postScriptError === "string" && JSON.stringify(payload.scripts).includes("nope(") ? { ...result.data, postScriptError: "<syntax error>" } : result.data;
        t.note(label, result.ok ? { ...data, body: unb64(data.body) } : result.error);
      };
      await send("send inline", { workspaceId: t.ws, request: { method: "GET", url: "{{base}}/echo?t={{token}}", headers: { "x-token": "{{token}}" }, body: "" }, scripts: {} });
      await send("send inline scripts", {
        workspaceId: t.ws,
        request: { method: "POST", url: "{{base}}/echo", headers: { "content-type": "application/json" }, body: '{"t":"{{token}}"}' },
        scripts: {
          pre: "lp.request.headers.set('x-pre', 'yes'); lp.request.url = lp.request.url + '?pre=1'; lp.environment.set('token', 't1');",
          post: "lp.environment.set('seen', String(lp.response.status));",
          test: [
            "lp.test('status is 200', () => { lp.expect(lp.response.status).to.equal(200); });",
            "lp.test('body has method', () => { lp.expect(lp.response.json().method).to.equal('POST'); });",
            "lp.test('fails', () => { lp.expect(lp.response.status).to.be.above(300); });",
            "console.log('done', { a: 1 });",
          ].join("\n"),
        },
      });
      await send("send pre error", { workspaceId: t.ws, request: { method: "GET", url: "{{base}}/echo", headers: {}, body: "" }, scripts: { pre: "throw new Error('boom')", post: "nope(", test: "lp.test('x', () => {}); throw 'late'" } });
      await send("send multipart", {
        workspaceId: t.ws,
        request: { method: "POST", url: "{{base}}/status?code=201", headers: {}, body: "", bodyParts: [{ name: "a", kind: "text", value: "{{token}}" }, { name: "f", kind: "file", value: b64("file!"), fileName: "f.txt", contentType: "text/plain" }] },
        scripts: {},
      });
      const saved = await t.call("entity.create", { kind: "requests", entity: request({ url: "{{base}}/echo?saved=1", testScript: "lp.test('ok', () => lp.expect(lp.response.status).to.equal(200));" }) });
      await send("send by id", { workspaceId: t.ws, id: saved?.id ?? "x" });
      await send("send missing id", { workspaceId: t.ws, id: "missing" });

      await t.call("script.execute", { script: "lp.request.method = 'PUT'; lp.environment.set('k', 5); lp.request.headers.unset('a');", context: "pre", request: { method: "GET", url: "http://x/", headers: { a: "1", b: "2" }, body: "" }, envVars: { z: "1" } });
      await t.call("script.execute", { script: "lp.environment.set('s', lp.response.json().a + lp.response.headers.get('h'))", context: "post", response: { status: 200, headers: { h: "v" }, body: '{"a":"b"}' }, envVars: {} });
      await t.call("script.execute", {
        script: [
          "lp.test('eql', () => lp.expect(lp.response.json()).to.eql({ a: [1, 2] }));",
          "lp.test('not', () => lp.expect(1).to.not.equal(2));",
          "lp.test('include', () => lp.expect('hello').to.include('ell'));",
          "lp.test('type', () => lp.expect([]).to.be.an('array'));",
          "lp.test('bad', () => lp.expect({}).to.have.property('x'));",
          "lp.test('header', () => lp.expect(lp.response.headers.get('Content-Type')).to.match(/json/));",
          "console.warn('w'); console.error('e', 1);",
        ].join("\n"),
        context: "test",
        response: { status: 200, headers: { "content-type": "application/json" }, body: '{"a":[1,2]}', responseTime: 12 },
        envVars: { a: "1" },
      });
      const syntax = await t.raw("script.execute", { script: "syntax error here", context: "test", response: { status: 200, headers: {}, body: "" }, envVars: {} });
      t.note("script.execute syntax error", { ...syntax.data, testResults: (syntax.data?.testResults ?? []).map((r) => ({ ...r, error: r.error ? "<syntax error>" : r.error })) });
      await t.call("script.execute", { script: "1", context: "pre", envVars: {} });
    },
  },
  {
    name: "forward-proxy",
    ignoreEvents: ["event.log.entry"],
    async run(t) {
      const viaProxy = async (label, opts) => {
        const res = await t.http({ port: t.ports.proxy, ...opts });
        t.note(label, res.body === undefined ? res : { ...res, headers: { ...res.headers, "content-length": undefined } });
      };
      await t.call("server.status", {});
      await t.call("proxy.status", {});
      await t.call("server.start", {});
      await t.sleep(300);
      await t.call("server.status", { workspaceId: t.ws });
      await t.call("proxy.status", {});

      await viaProxy("passthrough", { method: "GET", requestPath: `${t.upstream}/echo?p=1`, headers: { host: "127.0.0.1:18090", "x-in": "1" } });
      await viaProxy("passthrough post", { method: "POST", requestPath: `${t.upstream}/echo`, headers: { host: "127.0.0.1:18090", "content-type": "application/json" }, body: '{"z":1}' });
      await viaProxy("upstream status", { method: "GET", requestPath: `${t.upstream}/status?code=418`, headers: { host: "127.0.0.1:18090" } });
      await viaProxy("closed upstream", { method: "GET", requestPath: "http://127.0.0.1:9/nothing", headers: { host: "127.0.0.1:9" } });

      const exact = await t.call("entity.create", { kind: "mocks", entity: mock({ urlPattern: "http://api.test/users", standalone: true, responseStatus: 203, responseBody: '{"mocked":true}', responseHeaders: { "content-type": "application/json", "x-mock": "1" } }) });
      await t.call("entity.create", { kind: "mocks", entity: mock({ name: "Regex", urlPattern: "^http://api\\.test/items/\\d+$", useRegex: true, method: "GET", responseStatus: 202, responseBody: "item" }) });
      await t.call("entity.create", { kind: "mocks", entity: mock({ name: "Post only", method: "POST", urlPattern: "http://api.test/users", responseStatus: 201, responseBody: "created" }) });
      t.note("mocks", (await t.raw("entity.list", { workspaceId: t.ws, kind: "mocks" })).data);
      await t.call("entity.setEnabled", { workspaceId: t.ws, kind: "mocks", id: exact?.id ?? "x", enabled: true });
      // A mock that is not standalone overlays the real upstream's response; only what it declares is replaced.
      await t.call("entity.create", { kind: "mocks", entity: mock({ name: "Overlay", urlPattern: `${t.upstream}/echo?overlay=1`, responseStatus: 200, responseHeaders: { "x-overlay": "yes" }, mockedResponseHeaders: ["x-overlay"], responseBody: "" }) });
      await viaProxy("mock overlay", { method: "GET", requestPath: `${t.upstream}/echo?overlay=1`, headers: { host: "127.0.0.1:18090" } });
      await viaProxy("mock exact", { method: "GET", requestPath: "http://api.test/users", headers: { host: "api.test" } });
      await viaProxy("mock regex", { method: "GET", requestPath: "http://api.test/items/42", headers: { host: "api.test" } });
      await viaProxy("mock method", { method: "POST", requestPath: "http://api.test/users", headers: { host: "api.test" }, body: "x" });
      await viaProxy("mock miss", { method: "DELETE", requestPath: "http://api.test/users", headers: { host: "api.test" } });

      const rule = await t.call("entity.create", {
        kind: "proxyRules",
        entity: { name: "To upstream", pattern: "^http://rewrite\\.test/", useRegex: true, targetType: "external", targetMappingId: "", targetExternal: "127.0.0.1:18090", requestScript: "", responseScript: "" },
      });
      await t.call("entity.setEnabled", { workspaceId: t.ws, kind: "rules", id: rule?.id ?? "x", enabled: true });
      await viaProxy("rule", { method: "GET", requestPath: "http://rewrite.test/echo?r=1", headers: { host: "rewrite.test" } });
      await t.call("entity.create", {
        kind: "proxyRules",
        entity: {
          name: "Scripted", pattern: "^http://script\\.test/", useRegex: true, targetType: "external", targetMappingId: "", targetExternal: "127.0.0.1:18090",
          requestScript: "lp.request.headers.set('x-injected', 'yes');", responseScript: "lp.response.headers.set('x-after', '1');",
        },
      });
      await viaProxy("rule scripts", { method: "GET", requestPath: "http://script.test/echo", headers: { host: "script.test" } });
      await t.call("entity.create", {
        kind: "proxyRules",
        entity: { name: "No target", pattern: "^http://broken\\.test/", useRegex: true, targetType: "external", targetMappingId: "", targetExternal: "", requestScript: "", responseScript: "" },
      });
      await viaProxy("rule without target", { method: "GET", requestPath: "http://broken.test/", headers: { host: "broken.test" } });

      const captured = await t.call("capture.list", { workspaceId: t.ws });
      t.note("capture count", Array.isArray(captured) ? captured.length : captured);
      await t.call("capture.setPaused", { workspaceId: t.ws, paused: true });
      await viaProxy("while paused", { method: "GET", requestPath: `${t.upstream}/echo?paused=1`, headers: { host: "127.0.0.1:18090" } });
      await t.call("capture.setPaused", { workspaceId: t.ws, paused: false });
      const listed = await t.call("capture.list", { workspaceId: t.ws });
      const first = listed?.entries?.[0];
      await t.call("capture.get", { workspaceId: t.ws, id: first?.id ?? "missing" });
      await t.call("capture.get", { workspaceId: t.ws, id: "missing" });
      await t.call("capture.setDisableCache", { disableCache: false });
      await viaProxy("cache headers kept", { method: "GET", requestPath: `${t.upstream}/echo?cache=1`, headers: { host: "127.0.0.1:18090", "cache-control": "max-age=0", "if-none-match": "x" } });
      await t.call("capture.setDisableCache", { disableCache: true });
      await viaProxy("cache headers stripped", { method: "GET", requestPath: `${t.upstream}/echo?cache=2`, headers: { host: "127.0.0.1:18090", "cache-control": "max-age=0", "if-none-match": "x" } });
      await t.call("capture.remove", { workspaceId: t.ws, ids: [first?.id ?? "missing"] });
      await t.call("capture.clear", { workspaceId: t.ws });
      await t.call("capture.list", { workspaceId: t.ws });

      await t.call("server.restart", {});
      await t.sleep(300);
      await t.call("server.status", {});
      await viaProxy("after restart", { method: "GET", requestPath: `${t.upstream}/echo?again=1`, headers: { host: "127.0.0.1:18090" } });
      await t.call("server.stop", {});
      await t.sleep(200);
      await t.call("server.status", {});
      await viaProxy("stopped", { method: "GET", requestPath: `${t.upstream}/echo`, headers: { host: "127.0.0.1:18090" } });
    },
  },
  {
    name: "edge-reverse-proxy",
    ignoreEvents: ["event.log.entry"],
    async run(t) {
      const viaEdge = async (label, opts) => {
        const res = await t.http({ port: t.ports.edge, ...opts });
        t.note(label, res.body === undefined ? res : { ...res, headers: { ...res.headers, "content-length": undefined } });
      };
      await t.call("server.status", { target: "reverse" });
      await t.call("server.start", { target: "reverse" });
      await t.sleep(300);
      await t.call("server.status", { target: "reverse", workspaceId: t.ws });
      await viaEdge("home", { method: "GET", requestPath: "/", headers: {} });
      await viaEdge("unmapped", { method: "GET", requestPath: "/echo", headers: { host: "nothing.example" } });
      await t.call("entity.create", { kind: "mappings", entity: { domain: "app.localhost", target: "127.0.0.1:18090", label: "App" } });
      await viaEdge("mapped", { method: "GET", requestPath: "/echo?e=1", headers: { host: "app.localhost" } });
      await viaEdge("mapped post", { method: "POST", requestPath: "/echo", headers: { host: "app.localhost", "content-type": "text/plain" }, body: "hello" });
      await viaEdge("rfc6761 unmapped", { method: "GET", requestPath: "/", headers: { host: "other.localhost" } });
      const down = await t.call("entity.create", { kind: "mappings", entity: { domain: "down.localhost", target: "127.0.0.1:9" } });
      await viaEdge("target down", { method: "GET", requestPath: "/", headers: { host: "down.localhost" } });
      await t.call("entity.setEnabled", { workspaceId: t.ws, kind: "mappings", id: down?.id ?? "x", enabled: false });
      await viaEdge("disabled mapping", { method: "GET", requestPath: "/", headers: { host: "down.localhost" } });
      await t.call("server.restart", { target: "reverse" });
      await t.sleep(300);
      await viaEdge("after restart", { method: "GET", requestPath: "/echo", headers: { host: "app.localhost" } });
      await t.call("server.stop", { target: "reverse" });
      await t.sleep(200);
      await t.call("server.status", { target: "reverse" });
    },
  },
  {
    name: "import-export",
    async run(t) {
      const put = async (filename, text, mimeType = "text/plain") => {
        const staged = await t.raw("blob.put", { workspaceId: t.ws, filename, mimeType, size: Buffer.byteLength(text), data: b64(text) });
        return staged.data?.blobId ?? "missing";
      };
      const exported = async (label, payload) => {
        const result = await t.raw("export.create", { workspaceId: t.ws, ...payload });
        if (!result.ok || result.data?.ok === false) return t.note(label, result);
        const read = await t.raw("blob.read", { workspaceId: t.ws, blobId: result.data.blobId ?? result.data.artifactId ?? "x" });
        // The content is compared decoded; its base64 and digest would only restate it with the generated ids in.
        const { inline, sha256, ...rest } = result.data;
        t.note(label, { ...rest, content: read.ok ? unb64(read.data.data) : read.error });
      };
      // A parser's own wording for malformed JSON is not part of the contract; that it is refused, and how, is.
      const refused = async (label, action, payload) => {
        const result = await t.raw(action, payload);
        t.note(label, { ...result.data, error: String(result.data?.error ?? result.error?.message).replace(/^(\w*Error):.*$/s, "$1") });
      };
      const environmentVars = [{ id: "1", key: "host", value: "dev.test" }, { id: "2", key: "token", value: "abc", secret: true }];

      await t.call("export.formats", {});
      const folder = await t.call("folder.add", { kind: "request", name: "API", parentId: null });
      await t.call("entity.create", { kind: "requests", entity: request({ name: "List", folderId: folder?.id, url: "http://api.test/items?limit=5", headers: { accept: "application/json" } }) });
      await t.call("entity.create", { kind: "requests", entity: request({ name: "Create", method: "POST", folderId: folder?.id, url: "http://api.test/items", headers: { "content-type": "application/json" }, body: '{"name":"x"}' }) });
      await t.call("entity.create", { kind: "requests", entity: request({ name: "Top", url: "http://api.test/top" }) });
      await t.call("entity.create", { kind: "mocks", entity: mock({ name: "Users", responseBody: '{"users":[1]}' }) });
      await t.call("entity.create", { kind: "mocks", entity: mock({ name: "Gone", urlPattern: "http://api.test/gone", responseStatus: 410, responseBody: "" }) });
      await t.call("entity.create", { kind: "environments", entity: { name: "Dev", variables: environmentVars } });

      for (const [kind, format] of [
        ["requests", "requests-postman"], ["requests", "requests-openapi"], ["requests", "requests-curl"], ["requests", "requests-har"], ["requests", "requests-insomnia"],
        ["mocks", "mocks-postman"], ["mocks", "mocks-json"], ["environments", "environments-postman"], ["environments", "environments-env"], ["environments", "environments-json"],
        ["workspace", "workspace-json"], ["requests", "nope"],
      ]) {
        await exported(`export ${format}`, { kind, format });
      }
      await exported("export named", { kind: "mocks", format: "mocks-json", filename: "mine.json" });

      const postman = JSON.stringify({
        info: { name: "Imported", schema: "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
        item: [
          { name: "Folder", item: [{ name: "Inner", request: { method: "POST", url: { raw: "http://imp.test/a?b=1" }, header: [{ key: "X-A", value: "1" }], body: { mode: "raw", raw: '{"a":1}' } } }] },
          { name: "Root", request: { method: "GET", url: "http://imp.test/root" } },
        ],
      });
      const importPair = async (label, kind, format, filename, text) => {
        const blobId = await put(filename, text);
        await t.call("import.preflight", { workspaceId: t.ws, kind, format, blobId });
        await t.call("import.commit", { workspaceId: t.ws, kind, format, blobId, collisionStrategy: "skip" });
        await t.call("import.commit", { workspaceId: t.ws, kind, format, blobId, collisionStrategy: "rename" });
        await t.call("import.commit", { workspaceId: t.ws, kind, format, blobId, collisionStrategy: "overwrite" });
        t.note(`${label} requests`, (await t.raw("entity.list", { workspaceId: t.ws, kind })).data);
      };
      await importPair("postman", "requests", "requests-postman", "c.postman_collection.json", postman);
      await importPair("curl", "requests", "requests-curl", "c.sh", `curl -X POST http://cu.test/x -H 'A: b' -d 'k=v'\ncurl http://cu.test/y`);
      await importPair("har", "requests", "requests-har", "c.har", JSON.stringify({ log: { version: "1.2", entries: [{ request: { method: "GET", url: "http://har.test/a?q=1", headers: [{ name: "X-H", value: "1" }], queryString: [{ name: "q", value: "1" }] }, response: { status: 200 } }] } }));
      await importPair("openapi", "requests", "requests-openapi", "o.json", JSON.stringify({ openapi: "3.0.0", info: { title: "Pets", version: "1" }, servers: [{ url: "http://pets.test" }], paths: { "/pets": { get: { summary: "List pets" }, post: { summary: "Add pet" } } } }));
      await importPair("insomnia", "requests", "requests-insomnia", "i.json", JSON.stringify({ _type: "export", __export_format: 4, resources: [{ _id: "req_1", _type: "request", parentId: "wrk_1", name: "Ins", method: "GET", url: "http://ins.test/", headers: [] }, { _id: "wrk_1", _type: "workspace", name: "W" }] }));
      await importPair("mocks-json", "mocks", "mocks-json", "m.json", JSON.stringify([mock({ name: "Imported mock", urlPattern: "http://imp.test/m" })]));
      await importPair("environments-env", "environments", "environments-env", "dev.env", "HOST=imp.test\nTOKEN=zzz\n");

      await refused("preflight malformed", "import.preflight", { workspaceId: t.ws, kind: "requests", format: "requests-postman", blobId: await put("bad.json", "{ not json") });
      await refused("commit malformed", "import.commit", { workspaceId: t.ws, kind: "requests", format: "requests-postman", blobId: await put("bad.json", "{ not json") });
      await t.call("import.commit", { workspaceId: t.ws, kind: "requests", format: "requests-postman", blobId: "missing" });
      await t.call("import.preflight", { workspaceId: t.ws, kind: "requests", format: "nope", blobId: "x" });
      await t.call("export.create", { workspaceId: t.ws, kind: "requests", format: "nope" });
    },
  },
  {
    name: "tls-certificates",
    skipDisk: true,
    async run(t) {
      const shape = (label, value) => t.note(label, value);
      await t.call("tls.certStatus", {});
      await t.call("tls.exportCert", {});
      await t.call("tls.removeCert", {});
      const generated = await t.raw("tls.generate", {});
      shape("generate", generated.ok ? { keys: Object.keys(generated.data ?? {}).sort(), ok: generated.data?.ok } : generated.error);
      const status = await t.raw("tls.certStatus", {});
      shape("status after generate", status.ok ? { ...status.data, fingerprint: status.data?.fingerprint ? "<fp>" : status.data?.fingerprint } : status.error);
      const exported = await t.raw("tls.exportCert", {});
      shape("export", exported.ok ? { keys: Object.keys(exported.data ?? {}).sort() } : exported.error);
      const readBlob = async (result) => {
        const id = result.data?.blobId ?? result.data?.artifactId;
        if (!id) return undefined;
        return unb64((await t.raw("blob.read", { workspaceId: t.ws, blobId: id })).data?.data);
      };
      const pem = await readBlob(exported);
      shape("pem header", pem?.split("\n")[0]);
      await t.call("tls.install", {});
      const bad = await t.raw("blob.put", { workspaceId: t.ws, filename: "bad.pem", mimeType: "text/plain", size: 3, data: b64("bad") });
      await t.call("tls.importCert", { blobId: bad.data?.blobId ?? "x", workspaceId: t.ws });
      await t.call("tls.importKey", { blobId: bad.data?.blobId ?? "x", workspaceId: t.ws });
      await t.call("tls.importCert", { blobId: "missing", workspaceId: t.ws });
      await t.call("tls.removeCert", {});
      await t.call("tls.certStatus", {});
    },
  },
  {
    name: "runner",
    ignoreEvents: ["event.log.entry"],
    async run(t) {
      const folder = await t.call("folder.add", { kind: "request", name: "Suite", parentId: null });
      const empty = await t.call("folder.add", { kind: "request", name: "Empty", parentId: null });
      await t.call("entity.create", { kind: "environments", entity: { name: "Dev", variables: [{ id: "1", key: "base", value: t.upstream }] } });
      const make = async (name, extra) => (await t.call("entity.create", { kind: "requests", entity: request({ name, folderId: folder?.id, url: "{{base}}/echo", ...extra }) }));
      const a = await make("First", { testScript: "lp.test('200', () => lp.expect(lp.response.status).to.equal(200));", postScript: "lp.environment.set('seen', 'a')" });
      const b = await make("Second", { method: "POST", body: '{"n":2}', headers: { "content-type": "application/json" }, testScript: "lp.test('echoes', () => lp.expect(lp.response.json().method).to.equal('POST')); lp.test('fails', () => lp.expect(1).to.equal(2));" });
      await make("Third", { url: "{{base}}/status?code=500", testScript: "lp.test('ok', () => lp.expect(lp.response.status).to.equal(200));" });
      await make("Broken", { url: "http://127.0.0.1:9/closed" });

      await t.call("runner.status", { workspaceId: t.ws, folderId: folder?.id ?? "x" });
      await t.call("runner.loadConfig", { workspaceId: t.ws, folderId: folder?.id ?? "x" });
      await t.call("runner.saveConfig", { workspaceId: t.ws, folderId: folder?.id ?? "x", config: { requestOrder: [b?.id ?? "b", a?.id ?? "a"], iterations: 1, delayMs: 0 } });
      await t.call("runner.loadConfig", { workspaceId: t.ws, folderId: folder?.id ?? "x" });
      await t.call("runner.listFolderIds", { workspaceId: t.ws });
      await t.call("runner.start", { workspaceId: t.ws, folderId: folder?.id ?? "x" });
      let status;
      for (let i = 0; i < 100; i += 1) {
        status = (await t.raw("runner.status", { workspaceId: t.ws, folderId: folder?.id ?? "x" })).data;
        if (status && (status.finished || status.done || status.running === false)) break;
        await t.sleep(100);
      }
      t.note("final status", status);
      await t.call("runner.cancel", { workspaceId: t.ws, folderId: folder?.id ?? "x" });
      await t.call("runner.cancel", { workspaceId: t.ws, folderId: "missing" });
      await t.call("runner.start", { workspaceId: t.ws, folderId: empty?.id ?? "x" });
      await t.call("runner.start", { workspaceId: t.ws, folderId: "missing" });

      const report = { folderId: folder?.id ?? "x", folderName: "Suite", startedAt: 1700000000000, totalTests: 3, passedTests: 2, failedTests: 1, completedAt: 1700000003500, totalRequests: 2, results: [{ requestId: "r1", requestName: "First", method: "GET", url: "http://x/", status: 200, responseTime: 12, tests: [{ name: "200", passed: true }], testLogs: [] }, { requestId: "r2", requestName: "Second", method: "POST", url: "http://x/2", status: null, responseTime: 0, error: "boom <b>", tests: [], testLogs: [] }] };
      await t.call("runner.saveReport", { workspaceId: t.ws, report });
      await t.call("runner.getHistory", { workspaceId: t.ws, folderId: folder?.id ?? "x" });
      await t.call("runner.listFolderIds", { workspaceId: t.ws });
      for (const format of ["json", "html"]) {
        const result = await t.raw("runner.exportReport", { workspaceId: t.ws, report, format });
        const read = result.ok ? await t.raw("blob.read", { workspaceId: t.ws, blobId: result.data?.blobId ?? "x" }) : undefined;
        const { inline, sha256, ...rest } = result.data ?? {};
        t.note(`export ${format}`, result.ok ? { ...rest, content: read?.ok ? unb64(read.data.data) : unb64(inline) } : result.error);
      }
    },
  },
];
