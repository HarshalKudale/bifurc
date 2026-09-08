# Bifurc

**Bifurc is a desktop API development workspace for localhost routing, capture, mocking, protocol testing, and git-backed configuration.**

It brings together `*.localhost` mappings, proxy rules, capture tooling, REST/GraphQL/SOAP/gRPC request authoring, browser-assisted workflows, TLS interception, and workspace history in one developer-focused app.

## Main capabilities

- map friendly `*.localhost` domains to local services
- intercept, inspect, and replay HTTP traffic
- create mocks for REST, GraphQL, SOAP, and gRPC workflows
- save and organize authored requests across protocols
- use a browser companion extension for DevTools-driven capture and mock creation
- version workspace data with git and sync it between collaborators

## Repository structure

```text
Bifurc/
├── bifurc/            # Electron desktop application
├── bifurc-extension/  # Browser companion extension
```

The published site itself is maintained in the separate `bifurc-website` repository.

## Documentation

The documentation source now lives in the **website repository only** and is rendered from markdown files in `bifurc-website\content\docs`.

- Website repo: `bifurc-website\`
- Docs source: `bifurc-website\content\docs\`
- Website routes: `/docs/*`

## Quick start

```bash
git clone https://github.com/HarshalKudale/bifurc.git
cd bifurc
npm install
cd bifurc
npm run dev
```

## Browser extension

Load the `bifurc-extension/` folder unpacked in Chrome or Edge to:

- toggle browser proxy routing
- review traffic inside a Bifurc DevTools tab
- create saved requests and mocks from live browser traffic

## Licensing

Bifurc is **free for individual developers and teams of up to 10 users**.

You may use it, fork it publicly, and contribute pull requests under the repository license. Commercial products, enterprise/private-fork usage, and organizations beyond the free community grant require written permission and a paid license from the author.

See [LICENSE](LICENSE) for the full terms.
