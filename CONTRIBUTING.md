# Contributing to Bifurc

Bifurc is a free project maintained by one person. PRs are welcome but may take a while to review, and some will
be declined to keep the scope small. Open an issue before starting anything bigger than a bug fix. Vaults are in beta; please don't open feature PRs for them yet.

## Before you open a pull request

1. Read the [LICENSE](LICENSE). Bifurc is free for personal and non-commercial use; you may change your own copy
   and send changes upstream, but not distribute changed versions.
2. Sign the [CLA](CLA.md). A bot asks you on your first PR.
3. Keep the change focused: one logical change per PR, explain the user impact, no unrelated cleanup.
4. Add no new telemetry or network calls without discussing it in an issue first.

## Development

Bifurc is built with Rust and GPUI. Per-OS build dependencies, the `.env` file and the packaging targets are in
[docs/building.md](docs/building.md).

```
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

GitHub sign-in in a dev build needs your own GitHub OAuth App with device flow enabled. Put its client id in
`.env` (see `.env.example`). Builds you make yourself send no analytics.

User documentation lives on the website, [bifurc.app/docs](https://bifurc.app/docs). Developer documentation is
in [docs/](docs/). Update the docs when behavior changes.

## Commits and PRs

Write a short, plain subject line that says what changed. Fork the repo, branch from `master`, and open the PR
against `master`. CI must pass.

## Security

Report vulnerabilities privately; see [SECURITY.md](SECURITY.md).

## Need help?

See [SUPPORT.md](SUPPORT.md), or write to **contact@bifurc.app**.
