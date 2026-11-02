# The update manifest

A running Bifurc reads `https://bifurc.app/updates/v1/stable.json` to learn that a newer version exists. The
website serves it as a static file, written by its **Publish release** workflow when a release is published here.

## Schema v1

```json
{
  "schema": 1,
  "channels": { "store": "0.6.5", "winget": "0.6.5" },
  "releases": [
    { "version": "0.6.5", "date": "2026-11-02", "critical": false,
      "summary": "...", "notesUrl": "https://bifurc.app/changelog",
      "packages": { "zip-x64": { "url": "https://bifurc.app/dl/v0.6.5/Bifurc-0.6.5-win-x64.zip", "sha256": "...", "size": 1 } } }
  ]
}
```

`v1` in the address is a contract with every install already out there: a field is only ever **added**, and a
field an engine does not know is ignored. The reader is `crates/engine/src/updates/manifest.rs`.

- `channels` names the version a store or package manager has. It lags the newest release until that store has
  accepted it, and an install from one of them is offered nothing newer. `store` and `winget` follow the Store; `aur`
  is the AUR package.
- Any version is replaced by the newest in one step; the new version migrates the data it finds.
- `packages` keys name the file an install is replaced with (`zip-x64`, `appimage-x86_64`, `deb-amd64`,
  `tarball-x86_64`, `dmg-arm64`), as `Channel::package_key` in `updates/channel.rs` says.

## Where a download may come from

An engine only follows a package URL under a trusted prefix (`TRUSTED_PREFIXES` in `manifest.rs`). Engines before
0.6.5 trust only `github.com/HarshalKudale/bifurc-releases/` and `bifurc.app/`, so the manifest writes package URLs
as

```text
https://bifurc.app/dl/<tag>/<file>
```

The website answers that with a `302` to
`https://github.com/HarshalKudale/bifurc/releases/download/<tag>/<file>`. The route accepts only a tag of the form
`vX.Y.Z` and a file name of letters, digits, dots, dashes and underscores, so it cannot be used as an open
redirect. Engines from 0.6.5 also trust `github.com/HarshalKudale/bifurc/` directly (with the trailing slash, so
`bifurc-evil` does not match).

## Testing against another manifest

Set `BIFURC_UPDATE_URL` to point an engine at a manifest of your own.
