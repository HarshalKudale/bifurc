# Building

Rust 1.95 (pinned in `rust-toolchain.toml`; `rustup` installs it on the first build).

## Dependencies per OS

**Linux** (Ubuntu 22.04 package names):

```bash
sudo apt-get install -y build-essential clang cmake pkg-config libasound2-dev libfontconfig-dev \
  libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev libssl-dev libzstd-dev libvulkan1
# packaging only: dpkg-dev zip
```

**Windows:** Visual Studio Build Tools with the C++ workload and a Windows SDK. The Store package needs the SDK's
`makeappx` and `makepri`.

**macOS:** Xcode command line tools (`xcode-select --install`), and CMake.

## Run from source

```bash
cargo dev:engine     # the engine, in this terminal
cargo dev:ui         # the UI; attaches to a running engine or starts one from target/
cargo dev:cli <cmd>  # the `bifurc` CLI, e.g. `cargo dev:cli status`
cargo dev:mcp        # `bifurc --mcp` on stdin and stdout
```

On Windows a running engine locks `target/debug/bifurc-engine.exe`; stop it (`cargo dev:cli engine stop`) before
rebuilding.

## `.env`

Copy `.env.example` to `.env` (it is git-ignored). `crates/engine/build.rs` reads it and compiles the values in:

| Variable | Effect when empty |
| --- | --- |
| `BIFURC_GITHUB_CLIENT_ID` | GitHub sign-in is off. For a dev build use your own GitHub OAuth App with device flow enabled. |
| `BIFURC_POSTHOG_KEY` | The build sends no analytics (see [analytics.md](analytics.md)). |
| `BIFURC_POSTHOG_HOST` | The default PostHog host. |

## Check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo xtask licenses --check     # third-party notices are current
cargo deny check                 # advisories, licenses, bans, sources
```

## Package

`cargo xtask package` builds what this platform has into `release/desktop`:

| Target | Platform | Output |
| --- | --- | --- |
| `msix` | Windows | `Bifurc-<version>.msix` (unsigned; the Store signs it) |
| `zip` | Windows | `Bifurc-<version>-win-<arch>.zip` |
| `appimage` | Linux | `Bifurc-<version>-<arch>.AppImage` |
| `deb` | Linux | `bifurc_<version>_<arch>.deb` |
| `tarball` | Linux | `Bifurc-<version>-linux-<arch>.tar.gz` |
| `aur` | Linux | `manifests/aur/` (`PKGBUILD`, `.SRCINFO`); not part of the release workflow |
| `dmg` | macOS | `Bifurc-<version>-mac-<arch>.dmg` |
| `dir` | any | the staged folder only |

Choose targets with `--target msix,zip`, skip the build with `--skip-build`, and name the release the manifests
download from with `--release-tag v1.2.3`. `cargo xtask` with no arguments prints the usage.
