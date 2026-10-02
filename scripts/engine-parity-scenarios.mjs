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
        t.note(label, result.ok ? { ...result.data, body: unb64(result.data.body) } : result.error);
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
      await t.call("script.execute", { script: "syntax error here", context: "test", response: { status: 200, headers: {}, body: "" }, envVars: {} });
      await t.call("script.execute", { script: "1", context: "pre", envVars: {} });
    },
  },
];
