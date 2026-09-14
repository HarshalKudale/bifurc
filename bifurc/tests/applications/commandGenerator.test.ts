import { describe, it, expect } from "vitest";
import { generateResolvedCommand } from "@/applications/commandGenerator";
import { DEFAULT_DEBUG_PORTS, type ApplicationConfig, type RunConfigType } from "@/applications/types";

/**
 * `generateResolvedCommand` runs at config-save time and its output is persisted
 * and later spawned verbatim. A wrong flag here means the user's app simply will
 * not start — and because the command is baked in at save time, a regression is
 * invisible until someone presses Run. Every run-config type is therefore covered
 * explicitly, including the debug variants.
 */

function app(type: RunConfigType, over: Partial<ApplicationConfig> = {}): ApplicationConfig {
    return {
        id: "app-1",
        name: "Test App",
        type,
        workingDirectory: "/srv/app",
        args: [],
        createdAt: 1,
        workspaceId: "ws-1",
        resolvedCommand: "",
        resolvedCwd: "",
        resolvedEnv: {},
        ...over,
    } as ApplicationConfig;
}

describe("generateResolvedCommand — common behaviour", () => {
    it("uses the configured working directory", () => {
        expect(generateResolvedCommand(app("shell", { command: "ls" })).resolvedCwd).toBe("/srv/app");
    });

    it("falls back to the current directory when none is configured", () => {
        expect(generateResolvedCommand(app("shell", { command: "ls", workingDirectory: "" })).resolvedCwd).toBe(".");
    });

    it("splits a whitespace-separated args string", () => {
        expect(generateResolvedCommand(app("shell", { command: "run", args: "--a 1  --b" as unknown as string[] })).resolvedCommand)
            .toBe("run --a 1 --b");
    });

    it("accepts an args array and drops empty entries", () => {
        expect(generateResolvedCommand(app("shell", { command: "run", args: ["--a", "", "--b"] })).resolvedCommand)
            .toBe("run --a --b");
    });

    it("prepends the pre-run command to the main command", () => {
        expect(generateResolvedCommand(app("shell", { command: "serve", preRunCommand: "npm i" })).resolvedCommand)
            .toBe("npm i && serve");
    });

    it("ignores a whitespace-only pre-run command", () => {
        expect(generateResolvedCommand(app("shell", { command: "serve", preRunCommand: "   " })).resolvedCommand)
            .toBe("serve");
    });

    it("prepends the pre-run command to the debug command too", () => {
        const out = generateResolvedCommand(
            app("node", { preRunCommand: "npm i", nodeConfig: { scriptPath: "index.js", nodeArgs: [] } }),
        );
        expect(out.resolvedDebugCommand!.startsWith("npm i && node --inspect")).toBe(true);
    });

    it("falls back to config.command for an unknown type", () => {
        expect(generateResolvedCommand(app("shell", { command: "raw-cmd", type: "unknown" as RunConfigType })).resolvedCommand)
            .toBe("raw-cmd");
    });

    it("returns an empty command when nothing is configured", () => {
        expect(generateResolvedCommand(app("shell", { command: "" })).resolvedCommand).toBe("");
    });

    it("does not set a debug command for types without debug support", () => {
        expect(generateResolvedCommand(app("shell", { command: "x" })).resolvedDebugCommand).toBeUndefined();
        expect(generateResolvedCommand(app("maven")).resolvedDebugCommand).toBeUndefined();
        expect(generateResolvedCommand(app("docker")).resolvedDebugCommand).toBeUndefined();
    });
});

describe("generateResolvedCommand — shell", () => {
    it("appends args to the command", () => {
        expect(generateResolvedCommand(app("shell", { command: "bash run.sh", args: ["--port", "80"] })).resolvedCommand)
            .toBe("bash run.sh --port 80");
    });
});

describe("generateResolvedCommand — node", () => {
    it("defaults to index.js with no flags", () => {
        const out = generateResolvedCommand(app("node", { nodeConfig: { scriptPath: "", nodeArgs: [] } }));
        expect(out.resolvedCommand).toBe("node index.js");
    });

    it("puts node flags before the script and app args after it", () => {
        const out = generateResolvedCommand(
            app("node", { nodeConfig: { scriptPath: "server.js", nodeArgs: ["--enable-source-maps"] }, args: ["--port", "3000"] }),
        );
        expect(out.resolvedCommand).toBe("node --enable-source-maps server.js --port 3000");
    });

    it("accepts nodeArgs as a string", () => {
        const out = generateResolvedCommand(
            app("node", { nodeConfig: { scriptPath: "s.js", nodeArgs: "--max-old-space-size=512" as unknown as string[] } }),
        );
        expect(out.resolvedCommand).toBe("node --max-old-space-size=512 s.js");
    });

    it("builds an --inspect debug command on the default port", () => {
        const out = generateResolvedCommand(app("node", { nodeConfig: { scriptPath: "s.js", nodeArgs: [] } }));
        expect(out.resolvedDebugPort).toBe(DEFAULT_DEBUG_PORTS.node);
        expect(out.resolvedDebugCommand).toBe(`node --inspect=0.0.0.0:${DEFAULT_DEBUG_PORTS.node} s.js`);
    });

    it("honours a custom debug port", () => {
        const out = generateResolvedCommand(
            app("node", { debugPort: 9333, nodeConfig: { scriptPath: "s.js", nodeArgs: [] } }),
        );
        expect(out.resolvedDebugPort).toBe(9333);
        expect(out.resolvedDebugCommand).toContain("--inspect=0.0.0.0:9333");
    });
});

describe("generateResolvedCommand — npm", () => {
    it("uses npm run with a -- separator for args", () => {
        expect(generateResolvedCommand(app("npm", { npmConfig: { scriptName: "dev", packageManager: "npm" }, args: ["--port", "1"] })).resolvedCommand)
            .toBe("npm run dev -- --port 1");
    });

    it("defaults to the start script when there is no npm config", () => {
        expect(generateResolvedCommand(app("npm", {})).resolvedCommand).toBe("npm run start");
    });

    it("does NOT fall back to `start` for an explicitly empty script name", () => {
        // `cfg?.scriptName ?? "start"` only falls back on null/undefined, so an empty
        // string produces a command that is missing its script. Documented rather
        // than asserted as desirable — see TESTING.md §6.
        expect(generateResolvedCommand(app("npm", { npmConfig: { scriptName: "", packageManager: "npm" } })).resolvedCommand)
            .toBe("npm run ");
    });

    it("uses yarn without a run keyword", () => {
        expect(generateResolvedCommand(app("npm", { npmConfig: { scriptName: "dev", packageManager: "yarn" }, args: ["--x"] })).resolvedCommand)
            .toBe("yarn dev --x");
    });

    it("uses pnpm run", () => {
        expect(generateResolvedCommand(app("npm", { npmConfig: { scriptName: "dev", packageManager: "pnpm" } })).resolvedCommand)
            .toBe("pnpm run dev");
    });
});

describe("generateResolvedCommand — python", () => {
    it("runs a script", () => {
        expect(generateResolvedCommand(app("python", { pythonConfig: { scriptPath: "main.py", pythonArgs: [] } })).resolvedCommand)
            .toBe("python main.py");
    });

    it("defaults to main.py", () => {
        expect(generateResolvedCommand(app("python", { pythonConfig: { scriptPath: "", pythonArgs: [] } })).resolvedCommand)
            .toBe("python main.py");
    });

    it("supports module mode via -m", () => {
        expect(generateResolvedCommand(app("python", { pythonConfig: { scriptPath: "ignored.py", pythonArgs: [], module: "pkg.mod" } })).resolvedCommand)
            .toBe("python -m pkg.mod");
    });

    it("places python flags before the script", () => {
        expect(generateResolvedCommand(app("python", { pythonConfig: { scriptPath: "a.py", pythonArgs: ["-u"] } })).resolvedCommand)
            .toBe("python -u a.py");
    });

    it("builds a debugpy debug command that waits for the client", () => {
        const out = generateResolvedCommand(app("python", { pythonConfig: { scriptPath: "a.py", pythonArgs: [] } }));
        expect(out.resolvedDebugPort).toBe(DEFAULT_DEBUG_PORTS.python);
        expect(out.resolvedDebugCommand).toBe(
            `python -m debugpy --listen 0.0.0.0:${DEFAULT_DEBUG_PORTS.python} --wait-for-client a.py`,
        );
    });
});

describe("generateResolvedCommand — java", () => {
    it("runs a jar", () => {
        const out = generateResolvedCommand(
            app("java", { javaConfig: { mainClass: "", classpath: "", vmOptions: [], jarPath: "app.jar" } }),
        );
        expect(out.resolvedCommand).toBe("java -jar app.jar");
        expect(out.resolvedDebugCommand).toContain("-agentlib:jdwp=");
    });

    it("runs a main class on a classpath", () => {
        const out = generateResolvedCommand(
            app("java", { javaConfig: { mainClass: "com.x.Main", classpath: "out", vmOptions: ["-Xmx1g"] } }),
        );
        expect(out.resolvedCommand).toBe("java -Xmx1g -cp out com.x.Main");
    });

    it("defaults the main class and classpath", () => {
        expect(generateResolvedCommand(app("java", { javaConfig: { mainClass: "", classpath: "", vmOptions: [] } })).resolvedCommand)
            .toBe("java -cp . Main");
    });

    it("bakes the debug port into the JDWP address", () => {
        const out = generateResolvedCommand(
            app("java", { debugPort: 6006, javaConfig: { mainClass: "M", classpath: ".", vmOptions: [] } }),
        );
        expect(out.resolvedDebugPort).toBe(6006);
        expect(out.resolvedDebugCommand).toContain("address=*:6006");
    });
});

describe("generateResolvedCommand — spring-boot", () => {
    it("uses mvn on posix", () => {
        const out = generateResolvedCommand(
            app("spring-boot", { springBootConfig: { activeProfiles: [], buildTool: "maven" } }),
            "linux",
        );
        expect(out.resolvedCommand).toBe("mvn spring-boot:run");
        expect(out.resolvedEnv["MAVEN_OPTS"]).toContain("agentlib:jdwp");
    });

    it("uses mvn.cmd on win32", () => {
        const out = generateResolvedCommand(
            app("spring-boot", { springBootConfig: { activeProfiles: [], buildTool: "maven" } }),
            "win32",
        );
        expect(out.resolvedCommand).toBe("mvn.cmd spring-boot:run");
    });

    it("uses ./gradlew on posix and gradlew.bat on win32, debugging via JAVA_OPTS", () => {
        const posix = generateResolvedCommand(
            app("spring-boot", { springBootConfig: { activeProfiles: [], buildTool: "gradle" } }),
            "linux",
        );
        expect(posix.resolvedCommand).toBe("./gradlew bootRun");
        expect(posix.resolvedEnv["JAVA_OPTS"]).toContain("agentlib:jdwp");

        const win = generateResolvedCommand(
            app("spring-boot", { springBootConfig: { activeProfiles: [], buildTool: "gradle" } }),
            "win32",
        );
        expect(win.resolvedCommand).toBe("gradlew.bat bootRun");
    });

    it("maps active profiles to SPRING_PROFILES_ACTIVE", () => {
        const out = generateResolvedCommand(
            app("spring-boot", { springBootConfig: { activeProfiles: ["dev", "local"], buildTool: "maven" } }),
        );
        expect(out.resolvedEnv["SPRING_PROFILES_ACTIVE"]).toBe("dev,local");
    });

    it("omits SPRING_PROFILES_ACTIVE when no profile is set", () => {
        const out = generateResolvedCommand(
            app("spring-boot", { springBootConfig: { activeProfiles: [], buildTool: "maven" } }),
        );
        expect(out.resolvedEnv["SPRING_PROFILES_ACTIVE"]).toBeUndefined();
    });

    it("passes the main class through the build tool's own flag", () => {
        expect(
            generateResolvedCommand(app("spring-boot", { springBootConfig: { activeProfiles: [], buildTool: "maven", mainClass: "com.x.App" } }), "linux").resolvedCommand,
        ).toBe("mvn spring-boot:run -Dspring-boot.run.main-class=com.x.App");
        expect(
            generateResolvedCommand(app("spring-boot", { springBootConfig: { activeProfiles: [], buildTool: "gradle", mainClass: "com.x.App" } }), "linux").resolvedCommand,
        ).toBe("./gradlew bootRun -PmainClass=com.x.App");
    });
});

describe("generateResolvedCommand — maven and gradle", () => {
    it("defaults maven goals to clean install when there is no maven config", () => {
        expect(generateResolvedCommand(app("maven", {}), "linux").resolvedCommand).toBe("mvn clean install");
    });

    it("defaults gradle tasks to build when there is no gradle config", () => {
        expect(generateResolvedCommand(app("gradle", {}), "linux").resolvedCommand).toBe("./gradlew build");
    });

    it("does NOT fall back for an explicitly empty goals/tasks array", () => {
        // `cfg?.goals ?? ["clean","install"]` only falls back on null/undefined, so an
        // empty array produces a bare command. Documented, not endorsed.
        expect(generateResolvedCommand(app("maven", { mavenConfig: { goals: [], profiles: [] } }), "linux").resolvedCommand)
            .toBe("mvn ");
        expect(generateResolvedCommand(app("gradle", { gradleConfig: { tasks: [], extraArgs: [] } }), "linux").resolvedCommand)
            .toBe("./gradlew ");
    });

    it("joins goals, profiles and pom path", () => {
        expect(
            generateResolvedCommand(app("maven", { mavenConfig: { goals: ["clean", "verify"], profiles: ["ci", "fast"], pomPath: "pom.xml" } }), "linux").resolvedCommand,
        ).toBe("mvn clean verify -Pci,fast -f pom.xml");
    });

    it("joins gradle tasks, project dir and extra args", () => {
        expect(
            generateResolvedCommand(app("gradle", { gradleConfig: { tasks: ["test", "jar"], projectDir: "sub", extraArgs: ["--info"] } }), "linux").resolvedCommand,
        ).toBe("./gradlew test jar -p sub --info");
    });
});

describe("generateResolvedCommand — dotnet", () => {
    it("runs with just dotnet run", () => {
        expect(generateResolvedCommand(app("dotnet", { dotnetConfig: { projectPath: "" } })).resolvedCommand)
            .toBe("dotnet run");
    });

    it("adds project, framework and launch profile, then args after --", () => {
        expect(
            generateResolvedCommand(
                app("dotnet", {
                    dotnetConfig: { projectPath: "src/App.csproj", framework: "net8.0", launchProfile: "Dev" },
                    args: ["--seed"],
                }),
            ).resolvedCommand,
        ).toBe("dotnet run --project src/App.csproj --framework net8.0 --launch-profile Dev -- --seed");
    });
});

describe("generateResolvedCommand — go", () => {
    it("defaults the package path to .", () => {
        expect(generateResolvedCommand(app("go", { goConfig: { packagePath: "", buildFlags: [] } })).resolvedCommand)
            .toBe("go run .");
    });

    it("places build flags before the package", () => {
        expect(
            generateResolvedCommand(app("go", { goConfig: { packagePath: "./cmd/api", buildFlags: ["-race"] }, args: ["-v"] })).resolvedCommand,
        ).toBe("go run -race ./cmd/api -v");
    });

    it("builds a dlv debug command and forwards args after --", () => {
        const out = generateResolvedCommand(
            app("go", { goConfig: { packagePath: "./cmd/api", buildFlags: [] }, args: ["-v"] }),
        );
        expect(out.resolvedDebugPort).toBe(DEFAULT_DEBUG_PORTS.go);
        expect(out.resolvedDebugCommand).toBe(
            `dlv debug ./cmd/api --headless --listen=:${DEFAULT_DEBUG_PORTS.go} --api-version=2 --accept-multiclient -- -v`,
        );
    });

    it("omits the args separator when there are no args", () => {
        const out = generateResolvedCommand(app("go", { goConfig: { packagePath: ".", buildFlags: [] } }));
        expect(out.resolvedDebugCommand).not.toContain(" -- ");
    });
});

describe("generateResolvedCommand — docker", () => {
    it("builds a run command with ports, volumes and extra args", () => {
        const out = generateResolvedCommand(
            app("docker", {
                dockerConfig: { image: "nginx:latest", ports: ["8080:80"], volumes: ["/a:/b"], extraArgs: ["--name", "web"] },
            }),
        );
        expect(out.resolvedCommand).toBe("docker run --rm -p 8080:80 -v /a:/b --name web nginx:latest");
    });

    it("skips empty ports/volumes/extra args", () => {
        const out = generateResolvedCommand(
            app("docker", { dockerConfig: { image: "alpine", ports: ["", ""], volumes: [""], extraArgs: [""] } }),
        );
        expect(out.resolvedCommand).toBe("docker run --rm alpine");
    });

    it("builds then runs when there is a dockerfile but no image", () => {
        const out = generateResolvedCommand(
            app("docker", { dockerConfig: { dockerfile: "Dockerfile", buildContext: "./svc" } }),
        );
        expect(out.resolvedCommand).toBe("docker build -f Dockerfile -t _bifurc_build ./svc && docker run --rm _bifurc_build");
    });

    it("prefers the image when both image and dockerfile are set", () => {
        const out = generateResolvedCommand(
            app("docker", { dockerConfig: { image: "nginx", dockerfile: "Dockerfile" } }),
        );
        expect(out.resolvedCommand).toBe("docker run --rm nginx");
    });

    it("appends args after the image", () => {
        const out = generateResolvedCommand(
            app("docker", { dockerConfig: { image: "alpine" }, args: ["sh", "-c", "echo hi"] }),
        );
        expect(out.resolvedCommand).toBe("docker run --rm alpine sh -c echo hi");
    });
});

describe("generateResolvedCommand — docker-compose", () => {
    it("defaults the compose file", () => {
        expect(generateResolvedCommand(app("docker-compose", { dockerComposeConfig: { composeFile: "", extraArgs: [] } })).resolvedCommand)
            .toBe("docker compose -f docker-compose.yml up");
    });

    it("adds the service name and extra args", () => {
        expect(
            generateResolvedCommand(
                app("docker-compose", { dockerComposeConfig: { composeFile: "compose.dev.yml", serviceName: "api", extraArgs: ["-d"] } }),
            ).resolvedCommand,
        ).toBe("docker compose -f compose.dev.yml up api -d");
    });
});
