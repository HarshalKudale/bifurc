/**
 * Packages the desktop app — the Rust UI, the Rust engine and the Rust CLI — as one install. No Node is shipped:
 * the TypeScript engine and CLI (`packages/engine`, `packages/cli`) are scheduled for decommissioning.
 *
 * ```
 * node scripts/package-desktop.mjs                       # this platform's packages
 * node scripts/package-desktop.mjs --target nsis,msix    # windows: installer exe, Store package
 * node scripts/package-desktop.mjs --target appimage     # linux
 * node scripts/package-desktop.mjs --target dir          # the staged folder only
 * node scripts/package-desktop.mjs --skip-build          # package what is already built
 * ```
 *
 * Every package is the same folder (`release/desktop/<platform>-unpacked`) in a different wrapper:
 *
 * ```
 * bifurc-ui[.exe]      the interface
 * bifurc-engine[.exe]  the engine (Rust)
 * bifurc-tray.exe      the engine's tray icon (Windows)
 * bin/bifurc[.exe]     the `bifurc` command — the only folder that goes on PATH; runs resources/bifurc-cli
 * resources/bifurc-cli[.exe]   the CLI (Rust, `packages/cli-rust`)
 * ```
 *
 * `ui/crates/engine-client/src/sidecar.rs` and `ui/crates/cli/src/main.rs` look for exactly these paths
 * beside their own executable, and the engine finds the tray helper beside itself, so the folder works from
 * wherever it is put and no installer has to write a configuration.
 */
import { spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packaging = path.join(repo, "packaging");
const outDir = path.join(repo, "release", "desktop");
const cacheDir = path.join(outDir, "cache");
const isWindows = process.platform === "win32";
const exe = isWindows ? ".exe" : "";
const version = /\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/.exec(fs.readFileSync(path.join(repo, "Cargo.toml"), "utf8"))?.[1];
if (!version) throw new Error("package-desktop: no [workspace.package] version in Cargo.toml");

const argv = process.argv.slice(2);
const option = (name) => {
  const at = argv.indexOf(`--${name}`);
  return at === -1 ? undefined : argv[at + 1];
};
const targets = (option("target") ?? (isWindows ? "nsis,msix" : "appimage")).split(",").filter(Boolean);
const skipBuild = argv.includes("--skip-build");
const nodeArch = process.arch === "arm64" ? "arm64" : "x64";

function fail(message) {
  console.error(`package-desktop: ${message}`);
  process.exit(1);
}

/** `quiet` keeps a tool that lists every file it touches off the console unless it fails. */
function run(command, args, { quiet = false, ...opts } = {}) {
  console.log(`> ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, { stdio: quiet ? "pipe" : "inherit", encoding: "utf8", cwd: repo, ...opts });
  if (result.error) fail(`could not run ${command}: ${result.error.message}`);
  if (result.status !== 0) fail(`${command} exited with ${result.status}${quiet ? `\n${result.stdout}${result.stderr}` : ""}`);
}

function emptyDir(dir) {
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
}

function copyFile(from, to) {
  fs.mkdirSync(path.dirname(to), { recursive: true });
  fs.copyFileSync(from, to);
}

/** A template from `packaging/`, with `{{NAME}}` filled in. Always LF: two of them are shell and desktop files. */
function template(file, values) {
  let text = fs.readFileSync(path.join(packaging, file), "utf8").replace(/\r\n/g, "\n");
  for (const [name, value] of Object.entries(values)) text = text.replaceAll(`{{${name}}}`, value);
  return text;
}

// ---------------------------------------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------------------------------------

function build() {
  // The engine and the CLI: the repository's own cargo workspace.
  run("cargo", ["build", "--release", "-p", "bifurc-engine", "-p", "bifurc-cli"]);
  // The UI workspace: the app, which builds the tray helper beside it (`ui/crates/app/build.rs`), and the
  // `bifurc` launcher that goes on PATH.
  run("cargo", ["build", "--release", "-p", "bifurc-ui", "-p", "bifurc-cli"], { cwd: path.join(repo, "ui") });
}

/** A release build from `workspace` (the repository root, or `ui`). */
function cargoOutput(name, workspace = "ui") {
  const base = workspace === "root" ? repo : path.join(repo, "ui");
  const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(base, process.env.CARGO_TARGET_DIR) : path.join(base, "target");
  const file = path.join(targetDir, "release", name);
  if (!fs.existsSync(file)) fail(`${file} is missing — build first (drop --skip-build)`);
  return file;
}

// ---------------------------------------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------------------------------------

async function download(url) {
  console.log(`> download ${url}`);
  const response = await fetch(url);
  if (!response.ok) fail(`${url} answered ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

// ---------------------------------------------------------------------------------------------------------
// The staged folder
// ---------------------------------------------------------------------------------------------------------

async function stage() {
  const dir = path.join(outDir, `${isWindows ? "win" : "linux"}-unpacked`);
  emptyDir(dir);
  copyFile(cargoOutput(`bifurc-ui${exe}`), path.join(dir, `bifurc-ui${exe}`));
  if (isWindows) copyFile(cargoOutput("bifurc-tray.exe"), path.join(dir, "bifurc-tray.exe"));
  // The launcher (ui workspace) is `bin/bifurc`; the CLI it runs (repository workspace) and the engine are the Rust ones.
  copyFile(cargoOutput(`bifurc${exe}`), path.join(dir, "bin", `bifurc${exe}`));
  copyFile(cargoOutput(`bifurc${exe}`, "root"), path.join(dir, "resources", `bifurc-cli${exe}`));
  copyFile(cargoOutput(`bifurc-engine${exe}`, "root"), path.join(dir, `bifurc-engine${exe}`));
  copyFile(path.join(repo, "build", "icon.png"), path.join(dir, "resources", "icon.png"));
  copyFile(path.join(repo, "LICENSE"), path.join(dir, "LICENSE"));
  if (!isWindows) {
    for (const file of ["bifurc-ui", "bifurc-engine", "bin/bifurc", "resources/bifurc-cli"]) fs.chmodSync(path.join(dir, file), 0o755);
  }
  smokeTest(dir);
  return dir;
}

/** The staged `bifurc` must reach the staged CLI, and the staged engine must run, before anything wraps them. */
function smokeTest(dir) {
  const result = spawnSync(path.join(dir, "bin", `bifurc${exe}`), ["commands", "--json"], { encoding: "utf8" });
  if (result.status !== 0) fail(`the staged CLI does not run:\n${result.stderr || result.error?.message}`);
  const engine = spawnSync(path.join(dir, `bifurc-engine${exe}`), ["--help"], { encoding: "utf8" });
  if (engine.status !== 0) fail(`the staged engine does not run:\n${engine.stderr || engine.error?.message}`);
  console.log("staged CLI and engine start");
}

// ---------------------------------------------------------------------------------------------------------
// Windows: installer
// ---------------------------------------------------------------------------------------------------------

function firstExisting(candidates) {
  return candidates.find((candidate) => candidate && fs.existsSync(candidate));
}

function onPath(name) {
  const result = spawnSync(isWindows ? "where" : "which", [name], { encoding: "utf8" });
  return result.status === 0 ? result.stdout.split(/\r?\n/)[0].trim() : undefined;
}

function findMakensis() {
  const builderCache = path.join(process.env.LOCALAPPDATA ?? "", "electron-builder", "Cache", "nsis");
  const cached = fs.existsSync(builderCache)
    ? fs.readdirSync(builderCache).map((name) => path.join(builderCache, name, "makensis.exe"))
    : [];
  return (
    firstExisting([
      process.env.MAKENSIS,
      onPath("makensis"),
      path.join(process.env["ProgramFiles(x86)"] ?? "", "NSIS", "makensis.exe"),
      path.join(process.env.ProgramFiles ?? "", "NSIS", "makensis.exe"),
      ...cached,
    ]) ?? fail("makensis was not found. Install NSIS (https://nsis.sourceforge.io) or set MAKENSIS to makensis.exe.")
  );
}

function packageNsis(staged) {
  const out = path.join(outDir, `Bifurc-${version}-setup.exe`);
  run(findMakensis(), [
    "/V2",
    `/DVERSION=${version}`,
    `/DSTAGE=${staged}`,
    `/DOUT=${out}`,
    `/DICON=${path.join(repo, "build", "icon.ico")}`,
    path.join(packaging, "windows", "bifurc.nsi"),
  ]);
  return out;
}

// ---------------------------------------------------------------------------------------------------------
// Windows: Store package
// ---------------------------------------------------------------------------------------------------------

function findSdkTool(name) {
  const found = onPath(name);
  if (found) return found;
  const kits = path.join(process.env["ProgramFiles(x86)"] ?? "", "Windows Kits", "10", "bin");
  const versions = fs.existsSync(kits)
    ? fs.readdirSync(kits).filter((entry) => /^10\./.test(entry)).sort((a, b) => b.localeCompare(a, undefined, { numeric: true }))
    : [];
  return (
    firstExisting(versions.map((sdk) => path.join(kits, sdk, nodeArch, name))) ??
    fail(`${name} was not found. Install the Windows SDK.`)
  );
}

function packageMsix(staged) {
  const layout = path.join(outDir, "msix-layout");
  emptyDir(layout);
  fs.cpSync(staged, layout, { recursive: true });
  // The icons are indexed from a folder of their own. Indexing the whole layout would read file names
  // under node_modules as resource qualifiers (`is.js` as Icelandic) and emit a language pack for each.
  const icons = path.join(outDir, "msix-icons");
  emptyDir(icons);
  for (const assets of [path.join(repo, "build", "appx"), path.join(packaging, "windows", "assets")]) {
    for (const file of fs.readdirSync(assets).filter((name) => name.endsWith(".png"))) {
      copyFile(path.join(assets, file), path.join(icons, "assets", file));
    }
  }
  fs.cpSync(icons, layout, { recursive: true });
  // A package version is four numbers, and the Store keeps the fourth for itself.
  const manifest = path.join(layout, "AppxManifest.xml");
  fs.writeFileSync(manifest, template("windows/AppxManifest.xml", { VERSION: `${version}.0`, ARCH: nodeArch }));

  // The index that lets Windows pick the unplated, size-matched icons for the taskbar and the Start menu.
  const priConfig = path.join(outDir, "priconfig.xml");
  const makepri = findSdkTool("makepri.exe");
  run(makepri, ["createconfig", "/cf", priConfig, "/dq", "en-US", "/pv", "10.0.0", "/o"], { quiet: true });
  run(makepri, ["new", "/pr", icons, "/cf", priConfig, "/of", path.join(layout, "resources.pri"), "/mn", manifest, "/o"], { quiet: true });

  const out = path.join(outDir, `Bifurc-${version}.msix`);
  run(findSdkTool("makeappx.exe"), ["pack", "/d", layout, "/p", out, "/o"], { quiet: true });
  return out;
}

// ---------------------------------------------------------------------------------------------------------
// Linux: AppImage
// ---------------------------------------------------------------------------------------------------------

async function findAppimagetool() {
  const found = process.env.APPIMAGETOOL ?? onPath("appimagetool");
  if (found) return found;
  const arch = nodeArch === "arm64" ? "aarch64" : "x86_64";
  const tool = path.join(cacheDir, `appimagetool-${arch}.AppImage`);
  if (!fs.existsSync(tool)) {
    fs.mkdirSync(cacheDir, { recursive: true });
    const url = `https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${arch}.AppImage`;
    fs.writeFileSync(tool, await download(url), { mode: 0o755 });
  }
  return tool;
}

async function packageAppImage(staged) {
  const arch = nodeArch === "arm64" ? "aarch64" : "x86_64";
  const appDir = path.join(outDir, "Bifurc.AppDir");
  emptyDir(appDir);
  fs.cpSync(staged, path.join(appDir, "usr", "lib", "bifurc"), { recursive: true });
  // Rebuilding the same version must still replace what an earlier build of it unpacked.
  const buildId = `${version}-${Date.now().toString(36)}`;
  fs.writeFileSync(path.join(appDir, "AppRun"), template("linux/AppRun", { BUILD_ID: buildId }), { mode: 0o755 });
  fs.writeFileSync(path.join(appDir, "bifurc.desktop"), template("linux/bifurc.desktop", { EXEC: "AppRun" }));
  const icon = path.join(repo, "build", "icon.png");
  copyFile(icon, path.join(appDir, "bifurc.png"));
  copyFile(icon, path.join(appDir, ".DirIcon"));
  copyFile(icon, path.join(appDir, "usr", "share", "icons", "hicolor", "512x512", "apps", "bifurc.png"));

  const out = path.join(outDir, `Bifurc-${version}-${arch}.AppImage`);
  // The tool is itself an AppImage; unpacking instead of mounting spares the build machine FUSE.
  run(await findAppimagetool(), [appDir, out], { env: { ...process.env, ARCH: arch, APPIMAGE_EXTRACT_AND_RUN: "1" } });
  return out;
}

// ---------------------------------------------------------------------------------------------------------

const KNOWN = { dir: null, nsis: "win32", msix: "win32", appimage: "linux" };
for (const target of targets) {
  if (!(target in KNOWN)) fail(`unknown target "${target}" (dir, nsis, msix, appimage)`);
  if (KNOWN[target] !== null && KNOWN[target] !== process.platform) {
    fail(`${target} is built on ${KNOWN[target]}; this is ${process.platform}`);
  }
}

if (!skipBuild) build();
const staged = await stage();
const made = [staged];
if (targets.includes("nsis")) made.push(packageNsis(staged));
if (targets.includes("msix")) made.push(packageMsix(staged));
if (targets.includes("appimage")) made.push(await packageAppImage(staged));
console.log(`\nBifurc ${version}:\n${made.map((file) => `  ${path.relative(repo, file)}`).join("\n")}`);
