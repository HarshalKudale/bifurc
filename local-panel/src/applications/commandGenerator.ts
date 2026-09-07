/**
 * Command Generator — computes the full shell command string at config-save time.
 * At run time the backend just reads the pre-computed command and spawns it.
 * No runtime type dispatching, no class hierarchy.
 */

import { ApplicationConfig, DEFAULT_DEBUG_PORTS, RunConfigType } from "./types";

export interface ResolvedCommand {
    resolvedCommand: string;
    resolvedCwd: string;
    resolvedEnv: Record<string, string>;
    resolvedDebugCommand?: string;
    resolvedDebugPort?: number;
}

/**
 * Parse args — handles both string (from renderer textarea) and string[] from stored config.
 */
function parseArgs(raw: unknown): string[] {
    if (Array.isArray(raw)) return raw.filter(Boolean);
    if (typeof raw === "string" && raw.trim()) return raw.trim().split(/\s+/);
    return [];
}

/**
 * Prepend a pre-run command with && separator (single shell invocation).
 */
function withPreRun(preRun: string | undefined, main: string): string {
    if (!preRun || !preRun.trim()) return main;
    return `${preRun.trim()} && ${main}`;
}

/**
 * Generate resolved command fields from an application config.
 * Called at save time — results are persisted to disk.
 */
export function generateResolvedCommand(
    config: ApplicationConfig,
    platform: string = process.platform,
): ResolvedCommand {
    const args = parseArgs(config.args);
    const cwd = config.workingDirectory || ".";
    const isWin = platform === "win32";

    let command = "";
    let debugCommand: string | undefined;
    let debugPort: number | undefined;
    const env: Record<string, string> = {};

    switch (config.type) {
        case "shell": {
            const shellCmd = config.command || "";
            command = args.length > 0 ? `${shellCmd} ${args.join(" ")}` : shellCmd;
            break;
        }

        case "node": {
            const cfg = config.nodeConfig;
            const scriptPath = cfg?.scriptPath || "index.js";
            const nodeArgs = cfg?.nodeArgs ?? [];
            const nodeFlags = Array.isArray(nodeArgs)
                ? nodeArgs.filter(Boolean).join(" ")
                : typeof nodeArgs === "string" ? (nodeArgs as string).trim() : "";
            const parts = ["node"];
            if (nodeFlags) parts.push(nodeFlags);
            parts.push(scriptPath);
            if (args.length > 0) parts.push(...args);
            command = parts.join(" ");

            // Debug variant
            debugPort = config.debugPort ?? DEFAULT_DEBUG_PORTS.node ?? 9229;
            const debugParts = ["node", `--inspect=0.0.0.0:${debugPort}`];
            if (nodeFlags) debugParts.push(nodeFlags);
            debugParts.push(scriptPath);
            if (args.length > 0) debugParts.push(...args);
            debugCommand = debugParts.join(" ");
            break;
        }

        case "npm": {
            const cfg = config.npmConfig;
            const pm = cfg?.packageManager ?? "npm";
            const script = cfg?.scriptName ?? "start";

            if (pm === "yarn") {
                command = args.length > 0 ? `yarn ${script} ${args.join(" ")}` : `yarn ${script}`;
            } else if (pm === "pnpm") {
                command = args.length > 0 ? `pnpm run ${script} ${args.join(" ")}` : `pnpm run ${script}`;
            } else {
                command = args.length > 0
                    ? `npm run ${script} -- ${args.join(" ")}`
                    : `npm run ${script}`;
            }
            break;
        }

        case "python": {
            const cfg = config.pythonConfig;
            const pythonArgs = cfg?.pythonArgs ?? [];
            const flags = Array.isArray(pythonArgs) ? pythonArgs.filter(Boolean).join(" ") : "";
            const parts = ["python"];
            if (flags) parts.push(flags);

            if (cfg?.module) {
                parts.push("-m", cfg.module);
            } else {
                parts.push(cfg?.scriptPath || "main.py");
            }
            if (args.length > 0) parts.push(...args);
            command = parts.join(" ");

            // Debug variant using debugpy
            debugPort = config.debugPort ?? DEFAULT_DEBUG_PORTS.python ?? 5678;
            const debugParts = ["python"];
            if (flags) debugParts.push(flags);
            debugParts.push("-m", "debugpy", "--listen", `0.0.0.0:${debugPort}`, "--wait-for-client");
            if (cfg?.module) {
                debugParts.push("-m", cfg.module);
            } else {
                debugParts.push(cfg?.scriptPath || "main.py");
            }
            if (args.length > 0) debugParts.push(...args);
            debugCommand = debugParts.join(" ");
            break;
        }

        case "java": {
            const cfg = config.javaConfig;
            const vmOpts = cfg?.vmOptions ?? [];
            const vmFlags = Array.isArray(vmOpts) ? vmOpts.filter(Boolean).join(" ") : "";

            if (cfg?.jarPath) {
                const parts = ["java"];
                if (vmFlags) parts.push(vmFlags);
                parts.push("-jar", cfg.jarPath);
                if (args.length > 0) parts.push(...args);
                command = parts.join(" ");

                debugPort = config.debugPort ?? DEFAULT_DEBUG_PORTS.java ?? 5005;
                const dp = ["java", `-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address=*:${debugPort}`];
                if (vmFlags) dp.push(vmFlags);
                dp.push("-jar", cfg.jarPath);
                if (args.length > 0) dp.push(...args);
                debugCommand = dp.join(" ");
            } else {
                const mainClass = cfg?.mainClass || "Main";
                const cp = cfg?.classpath || ".";
                const parts = ["java"];
                if (vmFlags) parts.push(vmFlags);
                parts.push("-cp", cp, mainClass);
                if (args.length > 0) parts.push(...args);
                command = parts.join(" ");

                debugPort = config.debugPort ?? DEFAULT_DEBUG_PORTS.java ?? 5005;
                const dp = ["java", `-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address=*:${debugPort}`];
                if (vmFlags) dp.push(vmFlags);
                dp.push("-cp", cp, mainClass);
                if (args.length > 0) dp.push(...args);
                debugCommand = dp.join(" ");
            }
            break;
        }

        case "spring-boot": {
            const cfg = config.springBootConfig;
            const buildTool = cfg?.buildTool ?? "maven";
            const profiles = cfg?.activeProfiles ?? [];

            if (profiles.length > 0) {
                env["SPRING_PROFILES_ACTIVE"] = Array.isArray(profiles) ? profiles.join(",") : String(profiles);
            }

            debugPort = config.debugPort ?? DEFAULT_DEBUG_PORTS["spring-boot"] ?? 5005;
            const jdwp = `-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address=*:${debugPort}`;

            if (buildTool === "gradle") {
                const gradleCmd = isWin ? "gradlew.bat" : "./gradlew";
                const parts = [gradleCmd, "bootRun"];
                if (cfg?.mainClass) parts.push(`-PmainClass=${cfg.mainClass}`);
                if (args.length > 0) parts.push(...args);
                command = parts.join(" ");
                debugCommand = `${command}`;
                env["JAVA_OPTS"] = jdwp; // debug via env
            } else {
                const mvnCmd = isWin ? "mvn.cmd" : "mvn";
                const parts = [mvnCmd, "spring-boot:run"];
                if (cfg?.mainClass) parts.push(`-Dspring-boot.run.main-class=${cfg.mainClass}`);
                if (args.length > 0) parts.push(...args);
                command = parts.join(" ");
                debugCommand = `${command}`;
                env["MAVEN_OPTS"] = jdwp; // debug via env
            }
            break;
        }

        case "maven": {
            const cfg = config.mavenConfig;
            const mvnCmd = isWin ? "mvn.cmd" : "mvn";
            const goals = cfg?.goals ?? ["clean", "install"];
            const goalStr = Array.isArray(goals) ? goals.join(" ") : String(goals);
            const profiles = cfg?.profiles ?? [];
            const parts = [mvnCmd, goalStr];
            if (profiles.length > 0) {
                const profileStr = Array.isArray(profiles) ? profiles.join(",") : String(profiles);
                parts.push(`-P${profileStr}`);
            }
            if (cfg?.pomPath) parts.push(`-f`, cfg.pomPath);
            if (args.length > 0) parts.push(...args);
            command = parts.join(" ");
            break;
        }

        case "gradle": {
            const cfg = config.gradleConfig;
            const gradleCmd = isWin ? "gradlew.bat" : "./gradlew";
            const tasks = cfg?.tasks ?? ["build"];
            const taskStr = Array.isArray(tasks) ? tasks.join(" ") : String(tasks);
            const parts = [gradleCmd, taskStr];
            if (cfg?.projectDir) parts.push(`-p`, cfg.projectDir);
            const extraArgs = cfg?.extraArgs ?? [];
            if (Array.isArray(extraArgs) && extraArgs.length > 0) parts.push(...extraArgs);
            if (args.length > 0) parts.push(...args);
            command = parts.join(" ");
            break;
        }

        case "dotnet": {
            const cfg = config.dotnetConfig;
            const parts = ["dotnet", "run"];
            if (cfg?.projectPath) parts.push("--project", cfg.projectPath);
            if (cfg?.framework) parts.push("--framework", cfg.framework);
            if (cfg?.launchProfile) parts.push("--launch-profile", cfg.launchProfile);
            if (args.length > 0) parts.push("--", ...args);
            command = parts.join(" ");
            break;
        }

        case "go": {
            const cfg = config.goConfig;
            const buildFlags = cfg?.buildFlags ?? [];
            const flagStr = Array.isArray(buildFlags) ? buildFlags.filter(Boolean).join(" ") : "";
            const pkg = cfg?.packagePath || ".";
            const parts = ["go", "run"];
            if (flagStr) parts.push(flagStr);
            parts.push(pkg);
            if (args.length > 0) parts.push(...args);
            command = parts.join(" ");

            // Debug using dlv
            debugPort = config.debugPort ?? DEFAULT_DEBUG_PORTS.go ?? 2345;
            debugCommand = `dlv debug ${pkg} --headless --listen=:${debugPort} --api-version=2 --accept-multiclient`;
            if (args.length > 0) debugCommand += ` -- ${args.join(" ")}`;
            break;
        }

        case "docker": {
            const cfg = config.dockerConfig;
            const parts = ["docker", "run", "--rm"];
            const ports = cfg?.ports ?? [];
            for (const p of (Array.isArray(ports) ? ports : [])) {
                if (p) parts.push("-p", p);
            }
            const volumes = cfg?.volumes ?? [];
            for (const v of (Array.isArray(volumes) ? volumes : [])) {
                if (v) parts.push("-v", v);
            }
            const extraArgs = cfg?.extraArgs ?? [];
            for (const a of (Array.isArray(extraArgs) ? extraArgs : [])) {
                if (a) parts.push(a);
            }
            const image = cfg?.image || "";
            if (image) parts.push(image);
            if (args.length > 0) parts.push(...args);
            command = parts.join(" ");

            // If using build mode, build first then run
            if (!image && cfg?.dockerfile) {
                const buildCtx = cfg.buildContext || ".";
                const buildCmd = `docker build -f ${cfg.dockerfile} -t _bifurc_build ${buildCtx}`;
                command = `${buildCmd} && docker run --rm _bifurc_build`;
            }
            break;
        }

        case "docker-compose": {
            const cfg = config.dockerComposeConfig;
            const file = cfg?.composeFile || "docker-compose.yml";
            const parts = ["docker", "compose", "-f", file, "up"];
            if (cfg?.serviceName) parts.push(cfg.serviceName);
            const extraArgs = cfg?.extraArgs ?? [];
            for (const a of (Array.isArray(extraArgs) ? extraArgs : [])) {
                if (a) parts.push(a);
            }
            if (args.length > 0) parts.push(...args);
            command = parts.join(" ");
            break;
        }

        default:
            command = config.command || "";
            break;
    }

    // Prepend pre-run command if specified
    const finalCommand = withPreRun(config.preRunCommand, command);
    const finalDebugCommand = debugCommand
        ? withPreRun(config.preRunCommand, debugCommand)
        : undefined;

    return {
        resolvedCommand: finalCommand,
        resolvedCwd: cwd,
        resolvedEnv: env,
        resolvedDebugCommand: finalDebugCommand,
        resolvedDebugPort: debugPort,
    };
}
