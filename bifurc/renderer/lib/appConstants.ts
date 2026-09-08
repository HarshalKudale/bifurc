// Non-translatable example placeholders for the Applications panel.
// File paths, port numbers, CLI commands, VM args, image names, env var
// samples, etc. - grouped by run-config type, mirroring the panel structure.

export const appConstants = {
    common: {
        workingDirectory: "/path/to/project",
        programArgs: "--port 3000 --env production",
        preRunCommand: "npm install",
        debugPort: "9229",
    },
    shell: {
        scriptShell: "start.sh",
        scriptBat: "run.bat",
        scriptPowershell: "deploy.ps1",
        scriptVbs: "script.vbs",
        interpreter: "bash",
    },
    node: {
        scriptPath: "src/index.js",
        nodeFlags: "--experimental-vm-modules --max-old-space-size=4096",
        inspectPort: 9229,
        inspectPortPlaceholder: "9229",
    },
    npm: {
        scriptName: "start",
    },
    python: {
        scriptPath: "main.py",
        moduleName: "mypackage.server",
    },
    java: {
        mainClass: "com.example.MainApplication",
        jarPath: "target/app.jar",
        classpath: "target/classes:lib/*",
        vmOptions: "-Xmx2g -Xms512m -XX:+UseG1GC",
        systemProperties: "-Dserver.port=8080\n-Dspring.profiles.active=dev\n-Dlogging.level.root=INFO",
    },
    springBoot: {
        activeProfiles: "dev,local",
        mainClass: "com.example.Application",
        vmArgs: "-Xmx512m -Xms256m -Dspring.devtools.restart.enabled=true",
        programArgs: "--server.port=8080 --debug",
        beforeLaunchGoalMaven: "mvn compile",
        beforeLaunchGoalGradle: "./gradlew classes",
    },
    maven: {
        executable: "./mvnw",
        pomFile: "pom.xml",
        goals: "clean spring-boot:run",
        profiles: "dev,local",
        jvmArgs: "-Xmx1g",
        properties: "-Dserver.port=8080\n-Dspring.profiles.active=dev",
        settingsFile: "settings.xml",
    },
    gradle: {
        executable: "./gradlew",
        projectDir: ".",
        tasks: "bootRun",
        jvmArgs: "-Xmx1g",
        extraArgs: "--info",
        properties: "-Pprofile=dev",
    },
    dotnet: {
        projectFile: "MyApp.csproj",
        framework: "net8.0",
        launchProfile: "https",
    },
    go: {
        packagePath: "./cmd/server",
        packagePathDefault: "./...",
        buildFlags: '-ldflags "-s -w"',
    },
    docker: {
        image: "nginx:latest",
        dockerfile: "Dockerfile",
        buildContext: ".",
        ports: "8080:80\n3000:3000",
        volumes: "./data:/data\n./config:/app/config",
        envVars: "NODE_ENV=production\nPORT=3000",
        network: "my-network",
        entrypoint: "/bin/sh",
        extraArgs: "--rm --name mycontainer",
    },
    dockerCompose: {
        composeFile: "docker-compose.yml",
        services: "web db redis",
        profile: "development",
        extraArgs: "--remove-orphans",
    },
} as const;
