/**
 * Application run configuration types.
 * Modeled after IntelliJ IDEA run/debug configurations.
 */

// ── Run Config Types ──────────────────────────────────────────────────────────

export type RunConfigType =
    | "shell"
    | "node"
    | "npm"
    | "python"
    | "java"
    | "dotnet"
    | "go"
    | "docker"
    | "docker-compose"
    | "maven"
    | "gradle"
    | "spring-boot";

// ── Type-Specific Configs ─────────────────────────────────────────────────────

export interface NodeConfig {
    scriptPath: string;
    nodeArgs: string[];
}

export interface NpmConfig {
    scriptName: string;
    packageManager: "npm" | "yarn" | "pnpm";
}

export interface PythonConfig {
    scriptPath: string;
    pythonArgs: string[];
    module?: string; // -m module instead of script
}

export interface JavaConfig {
    mainClass: string;
    classpath: string;
    vmOptions: string[];
    jarPath?: string; // alternative: run via -jar
}

export interface DotnetConfig {
    projectPath: string;
    framework?: string;
    launchProfile?: string;
}

export interface GoConfig {
    packagePath: string;
    buildFlags: string[];
}

export interface DockerConfig {
    image?: string;
    dockerfile?: string;
    buildContext?: string;
    ports: string[];
    volumes: string[];
    extraArgs: string[];
}

export interface DockerComposeConfig {
    composeFile: string;
    serviceName?: string;
    extraArgs: string[];
}

export interface MavenConfig {
    goals: string[];
    profiles: string[];
    pomPath?: string;
}

export interface GradleConfig {
    tasks: string[];
    projectDir?: string;
    extraArgs: string[];
}

export interface SpringBootConfig {
    activeProfiles: string[];
    mainClass?: string;
    buildTool: "maven" | "gradle";
}

// ── Main Application Config ───────────────────────────────────────────────────

export interface ApplicationConfig {
    id: string;
    name: string;
    type: RunConfigType;
    workingDirectory: string;
    command?: string;          // custom command override (for shell type)
    args: string[];
    debugPort?: number;
    preRunCommand?: string;    // shell command to run before main process
    createdAt: number;
    workspaceId: string;

    // ── Pre-computed at save time (used directly at run time) ──
    resolvedCommand: string;                   // full shell command to spawn
    resolvedCwd: string;                       // resolved working directory
    resolvedEnv: Record<string, string>;       // extra env vars for spawn
    resolvedDebugCommand?: string;             // debug variant (with inspect flags etc.)
    resolvedDebugPort?: number;                // debug port baked in

    // Type-specific config (only one populated based on `type`)
    nodeConfig?: NodeConfig;
    npmConfig?: NpmConfig;
    pythonConfig?: PythonConfig;
    javaConfig?: JavaConfig;
    dotnetConfig?: DotnetConfig;
    goConfig?: GoConfig;
    dockerConfig?: DockerConfig;
    dockerComposeConfig?: DockerComposeConfig;
    mavenConfig?: MavenConfig;
    gradleConfig?: GradleConfig;
    springBootConfig?: SpringBootConfig;
}

// ── Process State ─────────────────────────────────────────────────────────────

export type AppProcessStatus = "idle" | "starting" | "running" | "debugging" | "stopping" | "error" | "exited";

export interface AppProcessState {
    appId: string;
    status: AppProcessStatus;
    pid?: number;
    exitCode?: number | null;
    error?: string;
    debugPort?: number;
    startedAt?: number;
    stoppedAt?: number;
}

// ── Log Entry ─────────────────────────────────────────────────────────────────

export interface AppLogChunk {
    appId: string;
    stream: "stdout" | "stderr" | "system";
    data: string;
    ts: number;
}

// ── Debug Info ────────────────────────────────────────────────────────────────

export interface DebugInfo {
    type: RunConfigType;
    protocol: string;       // e.g. "V8 Inspector", "JDWP", "DAP"
    host: string;
    port: number;
    connectUrl?: string;    // e.g. "chrome://inspect" for Node
    instructions: string;   // human-readable connect instructions
}


// ── Default Debug Ports ───────────────────────────────────────────────────────

export const DEFAULT_DEBUG_PORTS: Partial<Record<RunConfigType, number>> = {
    node: 9229,
    java: 5005,
    "spring-boot": 5005,
    python: 5678,
    dotnet: 0, // random port assigned by runtime
    go: 2345,
};

// ── Run Config Type Labels ────────────────────────────────────────────────────

export const RUN_CONFIG_TYPE_LABELS: Record<RunConfigType, string> = {
    shell: "Shell / Script",
    node: "Node.js",
    npm: "NPM Script",
    python: "Python",
    java: "Java Application",
    dotnet: ".NET",
    go: "Go",
    docker: "Docker",
    "docker-compose": "Docker Compose",
    maven: "Maven",
    gradle: "Gradle",
    "spring-boot": "Spring Boot",
};

// ── Run Config Type Icons (lucide icon names) ─────────────────────────────────

export const RUN_CONFIG_TYPE_ICONS: Record<RunConfigType, string> = {
    shell: "Terminal",
    node: "Hexagon",
    npm: "Package",
    python: "Code2",
    java: "Coffee",
    dotnet: "Circle",
    go: "Zap",
    docker: "Container",
    "docker-compose": "Layers",
    maven: "Hammer",
    gradle: "Wrench",
    "spring-boot": "Leaf",
};
