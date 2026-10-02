/**
 * Packages the desktop app — the Rust UI, the engine, the CLI and the Node they run on — as one install.
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
 * bifurc-tray.exe      the engine's tray icon (Windows)
 * bin/bifurc[.exe]     the `bifurc` command — the only folder that goes on PATH
 * runtime/node[.exe]   the Node the engine and the CLI run on
 * resources/node_modules/@bifurc/{engine,cli,protocol} and what they depend on
 * ```
 *
 * `ui/crates/engine-client/src/sidecar.rs` and `ui/crates/cli/src/main.rs` look for exactly these paths
 * beside their own executable, and the engine finds the tray helper above the Node it runs on, so the folder
 * works from wherever it is put and no installer has to write a configuration.
 *
 * The engine is copied as built, with its real dependency closure, rather than bundled into one file: its
 * modules hold state at module level (`packages/engine/tsup.config.ts` says why it is not bundled), and the
 * CLI resolves the engine through `node_modules`.
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packaging = path.join(repo, "packaging");
const outDir = path.join(repo, "release", "desktop");
const cacheDir = path.join(outDir, "cache");
const isWindows = process.platform === "win32";
const exe = isWindows ? ".exe" : "";
const version = JSON.parse(fs.readFileSync(path.join(repo, "package.json"), "utf8")).version;

const argv = process.argv.slice(2);
const option = (name) => {
  const at = argv.indexOf(`--${name}`);
  return at === -1 ? undefined : argv[at + 1];
};
const targets = (option("target") ?? (isWindows ? "nsis,msix" : "appimage")).split(",").filter(Boolean);
const skipBuild = argv.includes("--skip-build");
/** The Node that is shipped. By default the version running this build — the one the engine was just built and tested on. */
const nodeVersion = (option("node-version") ?? process.versions.node).replace(/^v/, "");
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
  // `npm` is a .cmd on Windows, which only a shell can start.
  run("npm", ["run", "build:packages"], { shell: isWindows });
  // Building the app builds the tray helper beside it (`ui/crates/app/build.rs`).
  run("cargo", ["build", "--release", "-p", "bifurc-ui", "-p", "bifurc-cli"], { cwd: path.join(repo, "ui") });
}

function cargoOutput(name) {
  const targetDir = process.env.CARGO_TARGET_DIR
    ? path.resolve(repo, "ui", process.env.CARGO_TARGET_DIR)
    : path.join(repo, "ui", "target");
  const file = path.join(targetDir, "release", name);
  if (!fs.existsSync(file)) fail(`${file} is missing — build first (drop --skip-build)`);
  return file;
}

// ---------------------------------------------------------------------------------------------------------
// The bundled Node
// ---------------------------------------------------------------------------------------------------------

async function download(url) {
  console.log(`> download ${url}`);
  const response = await fetch(url);
  if (!response.ok) fail(`${url} answered ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

/** The official `node` binary for this platform, checked against the release's published checksums. */
async function fetchNode() {
  const explicit = option("node");
  if (explicit !== undefined) return path.resolve(explicit);

  const platform = isWindows ? "win" : "linux";
  const dir = path.join(cacheDir, `node-v${nodeVersion}-${platform}-${nodeArch}`);
  const binary = path.join(dir, `node${exe}`);
  if (fs.existsSync(binary)) return binary;

  const base = `https://nodejs.org/dist/v${nodeVersion}`;
  const asset = isWindows ? `win-${nodeArch}/node.exe` : `node-v${nodeVersion}-linux-${nodeArch}.tar.xz`;
  const sums = (await download(`${base}/SHASUMS256.txt`)).toString("utf8");
  const expected = sums.split("\n").map((line) => line.trim().split(/\s+/)).find(([, name]) => name === asset)?.[0];
  if (expected === undefined) fail(`Node ${nodeVersion} publishes no ${asset}`);
  const bytes = await download(`${base}/${asset}`);
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (actual !== expected) fail(`${asset} has sha256 ${actual}, the release says ${expected}`);

  fs.mkdirSync(dir, { recursive: true });
  if (isWindows) {
    fs.writeFileSync(binary, bytes);
  } else {
    const archive = path.join(dir, "node.tar.xz");
    fs.writeFileSync(archive, bytes);
    const inside = `node-v${nodeVersion}-linux-${nodeArch}/bin/node`;
    run("tar", ["-xJf", archive, "-C", dir, "--strip-components=2", inside]);
    fs.rmSync(archive);
    fs.chmodSync(binary, 0o755);
  }
  return binary;
}

// ---------------------------------------------------------------------------------------------------------
// The engine, the CLI and what they depend on
// ---------------------------------------------------------------------------------------------------------

/** Workspace packages by the real directory they live in. */
function workspacePackages() {
  const found = new Map();
  for (const entry of fs.readdirSync(path.join(repo, "packages"), { withFileTypes: true })) {
    const manifest = path.join(repo, "packages", entry.name, "package.json");
    if (entry.isDirectory() && fs.existsSync(manifest)) {
      found.set(fs.realpathSync(path.dirname(manifest)), JSON.parse(fs.readFileSync(manifest, "utf8")).name);
    }
  }
  return found;
}

/** Node's own lookup: the nearest `node_modules/<name>` at or above `from`. */
function resolvePackage(name, from) {
  for (let dir = from; ; dir = path.dirname(dir)) {
    const candidate = path.join(dir, "node_modules", name);
    if (fs.existsSync(path.join(candidate, "package.json"))) return fs.realpathSync(candidate);
    if (path.dirname(dir) === dir) return null;
  }
}

const DECLARATIONS = /\.d\.[cm]?ts$/;

/**
 * Copies the production dependency closure of `@bifurc/cli` (which includes the engine) into `nodeModules`,
 * keeping every package where Node will look for it: hoisted ones at the top, nested ones nested.
 */
function stageJavaScript(nodeModules) {
  const workspaces = workspacePackages();
  const rootModules = fs.realpathSync(path.join(repo, "node_modules")) + path.sep;

  const destinationOf = (dir) => {
    if (dir.startsWith(rootModules)) return path.join(nodeModules, dir.slice(rootModules.length));
    for (const [workspaceDir, name] of workspaces) {
      if (dir === workspaceDir) return path.join(nodeModules, name);
      if (dir.startsWith(workspaceDir + path.sep)) return path.join(nodeModules, name, dir.slice(workspaceDir.length + 1));
    }
    return fail(`${dir} is outside the repository's node_modules`);
  };

  const closure = new Map();
  const visit = (dir) => {
    if (closure.has(dir)) return;
    closure.set(dir, destinationOf(dir));
    const manifest = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8"));
    for (const name of Object.keys(manifest.dependencies ?? {})) {
      visit(resolvePackage(name, dir) ?? fail(`${manifest.name} depends on ${name}, which is not installed`));
    }
    for (const name of Object.keys(manifest.optionalDependencies ?? {})) {
      const optional = resolvePackage(name, dir);
      if (optional !== null) visit(optional);
    }
  };
  const cli = [...workspaces].find(([, name]) => name === "@bifurc/cli")?.[0] ?? fail("packages/cli is missing");
  visit(cli);

  for (const [from, to] of closure) {
    const workspace = workspaces.has(from);
    if (workspace && !fs.existsSync(path.join(from, "dist"))) fail(`${from} is not built — run npm run build:packages`);
    fs.cpSync(from, to, {
      recursive: true,
      filter: (source) => {
        const relative = path.relative(from, source);
        if (relative === "") return true;
        const top = relative.split(path.sep)[0];
        // A nested dependency is copied by its own visit, if production code reaches it at all.
        if (top === "node_modules") return false;
        if (workspace && top !== "dist" && relative !== "package.json") return false;
        if (source.endsWith(".map") || DECLARATIONS.test(source)) return false;
        // The engine is emitted as both cjs and esm, and everything here loads it as cjs.
        if (workspace && source.endsWith(".mjs") && fs.existsSync(`${source.slice(0, -4)}.js`)) return false;
        return true;
      },
    });
  }
  console.log(`staged ${closure.size} packages into ${path.relative(repo, nodeModules)}`);
}

// ---------------------------------------------------------------------------------------------------------
// The staged folder
// ---------------------------------------------------------------------------------------------------------

async function stage() {
  const dir = path.join(outDir, `${isWindows ? "win" : "linux"}-unpacked`);
  emptyDir(dir);
  copyFile(cargoOutput(`bifurc-ui${exe}`), path.join(dir, `bifurc-ui${exe}`));
  if (isWindows) copyFile(cargoOutput("bifurc-tray.exe"), path.join(dir, "bifurc-tray.exe"));
  copyFile(cargoOutput(`bifurc${exe}`), path.join(dir, "bin", `bifurc${exe}`));
  copyFile(await fetchNode(), path.join(dir, "runtime", `node${exe}`));
  copyFile(path.join(repo, "build", "icon.png"), path.join(dir, "resources", "icon.png"));
  copyFile(path.join(repo, "LICENSE"), path.join(dir, "LICENSE"));
  stageJavaScript(path.join(dir, "resources", "node_modules"));
  if (!isWindows) {
    for (const file of ["bifurc-ui", "bin/bifurc", "runtime/node"]) fs.chmodSync(path.join(dir, file), 0o755);
  }
  smokeTest(dir);
  return dir;
}

/** The staged CLI must start on the staged Node and reach the staged engine's modules, before anything wraps it. */
function smokeTest(dir) {
  const result = spawnSync(path.join(dir, "bin", `bifurc${exe}`), ["commands", "--json"], { encoding: "utf8" });
  if (result.status !== 0) fail(`the staged CLI does not run:\n${result.stderr || result.error?.message}`);
  const entry = path.join(dir, "resources", "node_modules", "@bifurc", "engine", "dist", "runtime", "hostCli.js");
  const engine = spawnSync(path.join(dir, "runtime", `node${exe}`), [entry, "--help"], { encoding: "utf8" });
  if (engine.status !== 0) fail(`the staged engine does not load:\n${engine.stderr || engine.error?.message}`);
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
