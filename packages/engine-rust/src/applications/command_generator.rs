//! The command line of an application, computed when its definition is saved. Starting an application later
//! only spawns the stored `resolvedCommand` in `resolvedCwd` with `resolvedEnv`.

use serde_json::{Map, Value, json};

use super::types::{
    DEFAULT_DEBUG_PORT_GO, DEFAULT_DEBUG_PORT_JAVA, DEFAULT_DEBUG_PORT_NODE, DEFAULT_DEBUG_PORT_PYTHON,
    DEFAULT_DEBUG_PORT_SPRING_BOOT, join_truthy, js_join, js_str,
};
use crate::utils::json::{is_truthy, opt_str};

/// The fields `application.save` persists beside the definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCommand {
    pub resolved_command: String,
    pub resolved_cwd: String,
    pub resolved_env: Map<String, Value>,
    pub resolved_debug_command: Option<String>,
    pub resolved_debug_port: Option<Value>,
}

static NULL: Value = Value::Null;

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&NULL)
}

/// `value[key] || default`, as a string.
fn or_default(value: &Value, key: &str, default: &str) -> String {
    let v = field(value, key);
    if is_truthy(v) { js_str(v) } else { default.to_string() }
}

/// `value[key]` when it is a truthy value, as a string.
fn truthy_str(value: &Value, key: &str) -> Option<String> {
    let v = field(value, key);
    is_truthy(v).then(|| js_str(v))
}

/// `value[key] ?? default`.
fn nullish<'a>(value: &'a Value, key: &str, default: &'a Value) -> &'a Value {
    match field(value, key) {
        Value::Null => default,
        other => other,
    }
}

/// `Array.isArray(v) ? v : []`.
fn as_list(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

/// `Array.isArray(v) ? v.join(sep) : String(v)`.
fn join_or_str(value: &Value, sep: &str) -> String {
    match value {
        Value::Array(items) => js_join(items, sep),
        other => js_str(other),
    }
}

/// `v.length > 0`.
fn has_length(value: &Value) -> bool {
    match value {
        Value::Array(items) => !items.is_empty(),
        Value::String(s) => !s.is_empty(),
        _ => false,
    }
}

/// The `args` of a definition: an array with its falsy entries removed, or a string split on whitespace.
fn parse_args(raw: &Value) -> Vec<String> {
    match raw {
        Value::Array(items) => items.iter().filter(|v| is_truthy(v)).map(js_str).collect(),
        Value::String(s) if !s.trim().is_empty() => s.split_whitespace().map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

fn with_pre_run(pre_run: Option<&str>, main: &str) -> String {
    match pre_run {
        Some(pre) if !pre.trim().is_empty() => format!("{} && {main}", pre.trim()),
        _ => main.to_string(),
    }
}

fn debug_port(config: &Value, default: i64) -> Value {
    nullish(config, "debugPort", &json!(default)).clone()
}

fn jdwp(port: &Value) -> String {
    format!("-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address=*:{}", js_str(port))
}

/// Compute the command line, working directory, environment and debug variant of `config` for `platform`
/// (a Node `process.platform` name; only `"win32"` is special).
pub fn generate_resolved_command(config: &Value, platform: &str) -> ResolvedCommand {
    let args = parse_args(field(config, "args"));
    let cwd = or_default(config, "workingDirectory", ".");
    let is_win = platform == "win32";

    let mut command: String;
    let mut debug_command: Option<String> = None;
    let mut debug_port_value: Option<Value> = None;
    let mut env: Map<String, Value> = Map::new();

    let kind = opt_str(config, "type").unwrap_or("");
    match kind {
        "shell" => {
            let shell_cmd = or_default(config, "command", "");
            command = if args.is_empty() { shell_cmd } else { format!("{shell_cmd} {}", args.join(" ")) };
        }

        "node" => {
            let cfg = field(config, "nodeConfig");
            let script_path = or_default(cfg, "scriptPath", "index.js");
            let node_flags = match nullish(cfg, "nodeArgs", &Value::Null) {
                Value::Array(items) => join_truthy(items, " "),
                Value::String(s) => s.trim().to_string(),
                _ => String::new(),
            };
            let mut parts = vec!["node".to_string()];
            if !node_flags.is_empty() {
                parts.push(node_flags.clone());
            }
            parts.push(script_path.clone());
            parts.extend(args.iter().cloned());
            command = parts.join(" ");

            let port = debug_port(config, DEFAULT_DEBUG_PORT_NODE);
            let mut debug_parts = vec!["node".to_string(), format!("--inspect=0.0.0.0:{}", js_str(&port))];
            if !node_flags.is_empty() {
                debug_parts.push(node_flags);
            }
            debug_parts.push(script_path);
            debug_parts.extend(args.iter().cloned());
            debug_command = Some(debug_parts.join(" "));
            debug_port_value = Some(port);
        }

        "npm" => {
            let cfg = field(config, "npmConfig");
            let pm = js_str(nullish(cfg, "packageManager", &json!("npm")));
            let script = js_str(nullish(cfg, "scriptName", &json!("start")));
            let tail = if args.is_empty() { String::new() } else { args.join(" ") };
            command = match (pm.as_str(), args.is_empty()) {
                ("yarn", true) => format!("yarn {script}"),
                ("yarn", false) => format!("yarn {script} {tail}"),
                ("pnpm", true) => format!("pnpm run {script}"),
                ("pnpm", false) => format!("pnpm run {script} {tail}"),
                (_, true) => format!("npm run {script}"),
                (_, false) => format!("npm run {script} -- {tail}"),
            };
        }

        "python" => {
            let cfg = field(config, "pythonConfig");
            let flags = match nullish(cfg, "pythonArgs", &Value::Null) {
                Value::Array(items) => join_truthy(items, " "),
                _ => String::new(),
            };
            let module = truthy_str(cfg, "module");
            let script = or_default(cfg, "scriptPath", "main.py");

            let mut parts = vec!["python".to_string()];
            if !flags.is_empty() {
                parts.push(flags.clone());
            }
            match &module {
                Some(module) => parts.extend(["-m".to_string(), module.clone()]),
                None => parts.push(script.clone()),
            }
            parts.extend(args.iter().cloned());
            command = parts.join(" ");

            let port = debug_port(config, DEFAULT_DEBUG_PORT_PYTHON);
            let mut debug_parts = vec!["python".to_string()];
            if !flags.is_empty() {
                debug_parts.push(flags);
            }
            debug_parts.extend(
                ["-m", "debugpy", "--listen"]
                    .iter()
                    .map(|s| s.to_string())
                    .chain([format!("0.0.0.0:{}", js_str(&port)), "--wait-for-client".to_string()]),
            );
            match &module {
                Some(module) => debug_parts.extend(["-m".to_string(), module.clone()]),
                None => debug_parts.push(script),
            }
            debug_parts.extend(args.iter().cloned());
            debug_command = Some(debug_parts.join(" "));
            debug_port_value = Some(port);
        }

        "java" => {
            let cfg = field(config, "javaConfig");
            let vm_flags = match nullish(cfg, "vmOptions", &Value::Null) {
                Value::Array(items) => join_truthy(items, " "),
                _ => String::new(),
            };
            // What follows the VM options: `-jar <jar>` or `-cp <classpath> <main class>`.
            let target: Vec<String> = match truthy_str(cfg, "jarPath") {
                Some(jar) => vec!["-jar".to_string(), jar],
                None => {
                    vec!["-cp".to_string(), or_default(cfg, "classpath", "."), or_default(cfg, "mainClass", "Main")]
                }
            };
            let mut parts = vec!["java".to_string()];
            if !vm_flags.is_empty() {
                parts.push(vm_flags.clone());
            }
            parts.extend(target.iter().cloned());
            parts.extend(args.iter().cloned());
            command = parts.join(" ");

            let port = debug_port(config, DEFAULT_DEBUG_PORT_JAVA);
            let mut debug_parts = vec!["java".to_string(), jdwp(&port)];
            if !vm_flags.is_empty() {
                debug_parts.push(vm_flags);
            }
            debug_parts.extend(target);
            debug_parts.extend(args.iter().cloned());
            debug_command = Some(debug_parts.join(" "));
            debug_port_value = Some(port);
        }

        "spring-boot" => {
            let cfg = field(config, "springBootConfig");
            let build_tool = js_str(nullish(cfg, "buildTool", &json!("maven")));
            let empty = json!([]);
            let profiles = nullish(cfg, "activeProfiles", &empty);

            if has_length(profiles) {
                let joined = match profiles {
                    Value::Array(items) => js_join(items, ","),
                    other => js_str(other),
                };
                env.insert("SPRING_PROFILES_ACTIVE".to_string(), Value::String(joined));
            }

            let port = debug_port(config, DEFAULT_DEBUG_PORT_SPRING_BOOT);
            let jdwp = jdwp(&port);
            let main_class = truthy_str(cfg, "mainClass");

            if build_tool == "gradle" {
                let gradle = if is_win { "gradlew.bat" } else { "./gradlew" };
                let mut parts = vec![gradle.to_string(), "bootRun".to_string()];
                if let Some(main_class) = main_class {
                    parts.push(format!("-PmainClass={main_class}"));
                }
                parts.extend(args.iter().cloned());
                command = parts.join(" ");
                env.insert("JAVA_OPTS".to_string(), Value::String(jdwp));
            } else {
                let mvn = if is_win { "mvn.cmd" } else { "mvn" };
                let mut parts = vec![mvn.to_string(), "spring-boot:run".to_string()];
                if let Some(main_class) = main_class {
                    parts.push(format!("-Dspring-boot.run.main-class={main_class}"));
                }
                parts.extend(args.iter().cloned());
                command = parts.join(" ");
                env.insert("MAVEN_OPTS".to_string(), Value::String(jdwp));
            }
            debug_command = Some(command.clone());
            debug_port_value = Some(port);
        }

        "maven" => {
            let cfg = field(config, "mavenConfig");
            let mvn = if is_win { "mvn.cmd" } else { "mvn" };
            let goals = join_or_str(nullish(cfg, "goals", &json!(["clean", "install"])), " ");
            let empty = json!([]);
            let profiles = nullish(cfg, "profiles", &empty);
            let mut parts = vec![mvn.to_string(), goals];
            if has_length(profiles) {
                parts.push(format!("-P{}", join_or_str(profiles, ",")));
            }
            if let Some(pom) = truthy_str(cfg, "pomPath") {
                parts.extend(["-f".to_string(), pom]);
            }
            parts.extend(args.iter().cloned());
            command = parts.join(" ");
        }

        "gradle" => {
            let cfg = field(config, "gradleConfig");
            let gradle = if is_win { "gradlew.bat" } else { "./gradlew" };
            let tasks = join_or_str(nullish(cfg, "tasks", &json!(["build"])), " ");
            let mut parts = vec![gradle.to_string(), tasks];
            if let Some(dir) = truthy_str(cfg, "projectDir") {
                parts.extend(["-p".to_string(), dir]);
            }
            if let Value::Array(extra) = nullish(cfg, "extraArgs", &json!([])) {
                parts.extend(extra.iter().map(js_str));
            }
            parts.extend(args.iter().cloned());
            command = parts.join(" ");
        }

        "dotnet" => {
            let cfg = field(config, "dotnetConfig");
            let mut parts = vec!["dotnet".to_string(), "run".to_string()];
            for (key, flag) in
                [("projectPath", "--project"), ("framework", "--framework"), ("launchProfile", "--launch-profile")]
            {
                if let Some(value) = truthy_str(cfg, key) {
                    parts.extend([flag.to_string(), value]);
                }
            }
            if !args.is_empty() {
                parts.push("--".to_string());
                parts.extend(args.iter().cloned());
            }
            command = parts.join(" ");
        }

        "go" => {
            let cfg = field(config, "goConfig");
            let flags = match nullish(cfg, "buildFlags", &Value::Null) {
                Value::Array(items) => join_truthy(items, " "),
                _ => String::new(),
            };
            let pkg = or_default(cfg, "packagePath", ".");
            let mut parts = vec!["go".to_string(), "run".to_string()];
            if !flags.is_empty() {
                parts.push(flags);
            }
            parts.push(pkg.clone());
            parts.extend(args.iter().cloned());
            command = parts.join(" ");

            let port = debug_port(config, DEFAULT_DEBUG_PORT_GO);
            let mut debug =
                format!("dlv debug {pkg} --headless --listen=:{} --api-version=2 --accept-multiclient", js_str(&port));
            if !args.is_empty() {
                debug.push_str(&format!(" -- {}", args.join(" ")));
            }
            debug_command = Some(debug);
            debug_port_value = Some(port);
        }

        "docker" => {
            let cfg = field(config, "dockerConfig");
            let mut parts = vec!["docker".to_string(), "run".to_string(), "--rm".to_string()];
            for port in as_list(nullish(cfg, "ports", &json!([]))).iter().filter(|v| is_truthy(v)) {
                parts.extend(["-p".to_string(), js_str(port)]);
            }
            for volume in as_list(nullish(cfg, "volumes", &json!([]))).iter().filter(|v| is_truthy(v)) {
                parts.extend(["-v".to_string(), js_str(volume)]);
            }
            for extra in as_list(nullish(cfg, "extraArgs", &json!([]))).iter().filter(|v| is_truthy(v)) {
                parts.push(js_str(extra));
            }
            let image = or_default(cfg, "image", "");
            if !image.is_empty() {
                parts.push(image.clone());
            }
            parts.extend(args.iter().cloned());
            command = parts.join(" ");

            if image.is_empty() {
                if let Some(dockerfile) = truthy_str(cfg, "dockerfile") {
                    let context = or_default(cfg, "buildContext", ".");
                    command = format!(
                        "docker build -f {dockerfile} -t _bifurc_build {context} && docker run --rm _bifurc_build"
                    );
                }
            }
        }

        "docker-compose" => {
            let cfg = field(config, "dockerComposeConfig");
            let file = or_default(cfg, "composeFile", "docker-compose.yml");
            let mut parts = vec!["docker".to_string(), "compose".to_string(), "-f".to_string(), file, "up".to_string()];
            if let Some(service) = truthy_str(cfg, "serviceName") {
                parts.push(service);
            }
            for extra in as_list(nullish(cfg, "extraArgs", &json!([]))).iter().filter(|v| is_truthy(v)) {
                parts.push(js_str(extra));
            }
            parts.extend(args.iter().cloned());
            command = parts.join(" ");
        }

        _ => {
            command = or_default(config, "command", "");
        }
    }

    let pre_run = opt_str(config, "preRunCommand");
    ResolvedCommand {
        resolved_command: with_pre_run(pre_run, &command),
        resolved_cwd: cwd,
        resolved_env: env,
        resolved_debug_command: debug_command.map(|c| with_pre_run(pre_run, &c)),
        resolved_debug_port: debug_port_value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(config: Value) -> ResolvedCommand {
        generate_resolved_command(&config, "linux")
    }

    #[test]
    fn shell_joins_command_and_args() {
        let r = resolve(
            json!({ "type": "shell", "command": "echo hi", "args": ["a b", "", "c"], "workingDirectory": "/w" }),
        );
        assert_eq!(r.resolved_command, "echo hi a b c");
        assert_eq!(r.resolved_cwd, "/w");
        assert!(r.resolved_debug_command.is_none() && r.resolved_debug_port.is_none());
        assert_eq!(resolve(json!({ "type": "shell" })).resolved_cwd, ".");
    }

    #[test]
    fn string_args_split_on_whitespace() {
        let r = resolve(json!({ "type": "shell", "command": "run", "args": "  --a   --b=\"x y\" " }));
        assert_eq!(r.resolved_command, "run --a --b=\"x y\"");
    }

    #[test]
    fn pre_run_is_chained_with_and() {
        let r =
            resolve(json!({ "type": "node", "preRunCommand": "  npm i  ", "nodeConfig": { "scriptPath": "s.js" } }));
        assert_eq!(r.resolved_command, "npm i && node s.js");
        assert_eq!(r.resolved_debug_command.unwrap(), "npm i && node --inspect=0.0.0.0:9229 s.js");
        let blank = resolve(json!({ "type": "shell", "command": "x", "preRunCommand": "   " }));
        assert_eq!(blank.resolved_command, "x");
    }

    #[test]
    fn node_flags_script_and_debug_port() {
        let r = resolve(json!({
            "type": "node", "args": ["--port", "80"], "debugPort": 9300,
            "nodeConfig": { "scriptPath": "my app/server.js", "nodeArgs": ["--max-old-space-size=4096", "", "-r ts-node/register"] }
        }));
        assert_eq!(r.resolved_command, "node --max-old-space-size=4096 -r ts-node/register my app/server.js --port 80");
        assert_eq!(
            r.resolved_debug_command.unwrap(),
            "node --inspect=0.0.0.0:9300 --max-old-space-size=4096 -r ts-node/register my app/server.js --port 80"
        );
        assert_eq!(r.resolved_debug_port, Some(json!(9300)));
        assert_eq!(resolve(json!({ "type": "node" })).resolved_command, "node index.js");
        let string_flags = resolve(json!({ "type": "node", "nodeConfig": { "nodeArgs": "  --x  " } }));
        assert_eq!(string_flags.resolved_command, "node --x index.js");
    }

    #[test]
    fn npm_variants() {
        let npm = |pm: Value, args: Value| {
            resolve(json!({ "type": "npm", "args": args, "npmConfig": { "scriptName": "dev", "packageManager": pm } }))
                .resolved_command
        };
        assert_eq!(npm(json!("npm"), json!([])), "npm run dev");
        assert_eq!(npm(json!("npm"), json!(["--host"])), "npm run dev -- --host");
        assert_eq!(npm(json!("yarn"), json!(["--host"])), "yarn dev --host");
        assert_eq!(npm(json!("pnpm"), json!([])), "pnpm run dev");
        assert_eq!(resolve(json!({ "type": "npm" })).resolved_command, "npm run start");
    }

    #[test]
    fn python_script_module_and_debugpy() {
        let r = resolve(json!({
            "type": "python", "args": ["--x"],
            "pythonConfig": { "scriptPath": "app.py", "pythonArgs": ["-u"] }
        }));
        assert_eq!(r.resolved_command, "python -u app.py --x");
        assert_eq!(
            r.resolved_debug_command.unwrap(),
            "python -u -m debugpy --listen 0.0.0.0:5678 --wait-for-client app.py --x"
        );
        assert_eq!(r.resolved_debug_port, Some(json!(5678)));
        let m = resolve(json!({ "type": "python", "pythonConfig": { "module": "flask", "scriptPath": "ignored.py" } }));
        assert_eq!(m.resolved_command, "python -m flask");
        assert_eq!(
            m.resolved_debug_command.unwrap(),
            "python -m debugpy --listen 0.0.0.0:5678 --wait-for-client -m flask"
        );
        assert_eq!(resolve(json!({ "type": "python" })).resolved_command, "python main.py");
    }

    #[test]
    fn java_jar_and_main_class() {
        let jar = resolve(
            json!({ "type": "java", "args": ["x"], "javaConfig": { "jarPath": "a.jar", "vmOptions": ["-Xmx1g"] } }),
        );
        assert_eq!(jar.resolved_command, "java -Xmx1g -jar a.jar x");
        assert_eq!(
            jar.resolved_debug_command.unwrap(),
            "java -agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address=*:5005 -Xmx1g -jar a.jar x"
        );
        let class =
            resolve(json!({ "type": "java", "javaConfig": { "mainClass": "com.A", "classpath": "out:lib/*" } }));
        assert_eq!(class.resolved_command, "java -cp out:lib/* com.A");
        assert_eq!(resolve(json!({ "type": "java" })).resolved_command, "java -cp . Main");
    }

    #[test]
    fn spring_boot_maven_and_gradle_per_platform() {
        let cfg = |tool: &str| {
            json!({
                "type": "spring-boot", "args": ["-q"], "debugPort": 6006,
                "springBootConfig": { "buildTool": tool, "activeProfiles": ["dev", "local"], "mainClass": "com.App" }
            })
        };
        let maven = generate_resolved_command(&cfg("maven"), "win32");
        assert_eq!(maven.resolved_command, "mvn.cmd spring-boot:run -Dspring-boot.run.main-class=com.App -q");
        assert_eq!(maven.resolved_debug_command.as_deref(), Some(maven.resolved_command.as_str()));
        assert_eq!(maven.resolved_env["SPRING_PROFILES_ACTIVE"], json!("dev,local"));
        assert_eq!(
            maven.resolved_env["MAVEN_OPTS"],
            json!("-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address=*:6006")
        );
        assert_eq!(maven.resolved_debug_port, Some(json!(6006)));

        let gradle = generate_resolved_command(&cfg("gradle"), "linux");
        assert_eq!(gradle.resolved_command, "./gradlew bootRun -PmainClass=com.App -q");
        assert!(gradle.resolved_env.contains_key("JAVA_OPTS") && !gradle.resolved_env.contains_key("MAVEN_OPTS"));
        let win_gradle = generate_resolved_command(&cfg("gradle"), "win32");
        assert!(win_gradle.resolved_command.starts_with("gradlew.bat bootRun"));

        let plain = resolve(json!({ "type": "spring-boot" }));
        assert_eq!(plain.resolved_command, "mvn spring-boot:run");
        assert!(!plain.resolved_env.contains_key("SPRING_PROFILES_ACTIVE"));
        assert_eq!(plain.resolved_debug_port, Some(json!(5005)));
    }

    #[test]
    fn maven_and_gradle() {
        let m = resolve(json!({
            "type": "maven", "args": ["-DskipTests"],
            "mavenConfig": { "goals": ["clean", "package"], "profiles": ["a", "b"], "pomPath": "sub/pom.xml" }
        }));
        assert_eq!(m.resolved_command, "mvn clean package -Pa,b -f sub/pom.xml -DskipTests");
        assert_eq!(resolve(json!({ "type": "maven" })).resolved_command, "mvn clean install");
        assert_eq!(
            generate_resolved_command(&json!({ "type": "maven" }), "win32").resolved_command,
            "mvn.cmd clean install"
        );

        let g = resolve(json!({
            "type": "gradle", "args": ["--info"],
            "gradleConfig": { "tasks": ["clean", "test"], "projectDir": "app", "extraArgs": ["--stacktrace", ""] }
        }));
        assert_eq!(g.resolved_command, "./gradlew clean test -p app --stacktrace  --info");
        assert_eq!(resolve(json!({ "type": "gradle" })).resolved_command, "./gradlew build");
    }

    #[test]
    fn dotnet_and_go() {
        let d = resolve(json!({
            "type": "dotnet", "args": ["a"],
            "dotnetConfig": { "projectPath": "Api/Api.csproj", "framework": "net8.0", "launchProfile": "http" }
        }));
        assert_eq!(
            d.resolved_command,
            "dotnet run --project Api/Api.csproj --framework net8.0 --launch-profile http -- a"
        );
        assert_eq!(resolve(json!({ "type": "dotnet" })).resolved_command, "dotnet run");

        let g = resolve(
            json!({ "type": "go", "args": ["-p", "1"], "goConfig": { "packagePath": "./cmd/x", "buildFlags": ["-race", ""] } }),
        );
        assert_eq!(g.resolved_command, "go run -race ./cmd/x -p 1");
        assert_eq!(
            g.resolved_debug_command.unwrap(),
            "dlv debug ./cmd/x --headless --listen=:2345 --api-version=2 --accept-multiclient -- -p 1"
        );
        assert_eq!(resolve(json!({ "type": "go" })).resolved_command, "go run .");
    }

    #[test]
    fn docker_run_and_build() {
        let r = resolve(json!({
            "type": "docker", "args": ["serve"],
            "dockerConfig": { "image": "nginx:1", "ports": ["80:80", ""], "volumes": ["/a b:/c"], "extraArgs": ["--name", "web"] }
        }));
        assert_eq!(r.resolved_command, "docker run --rm -p 80:80 -v /a b:/c --name web nginx:1 serve");
        let build = resolve(
            json!({ "type": "docker", "dockerConfig": { "dockerfile": "Dockerfile.dev", "buildContext": "ctx" } }),
        );
        assert_eq!(
            build.resolved_command,
            "docker build -f Dockerfile.dev -t _bifurc_build ctx && docker run --rm _bifurc_build"
        );
        let default_ctx = resolve(json!({ "type": "docker", "dockerConfig": { "dockerfile": "D" } }));
        assert!(default_ctx.resolved_command.contains("_bifurc_build . &&"));
        assert_eq!(resolve(json!({ "type": "docker" })).resolved_command, "docker run --rm");
    }

    #[test]
    fn docker_compose() {
        let r = resolve(json!({
            "type": "docker-compose", "args": ["--build"],
            "dockerComposeConfig": { "composeFile": "dc.yml", "serviceName": "api", "extraArgs": ["-d"] }
        }));
        assert_eq!(r.resolved_command, "docker compose -f dc.yml up api -d --build");
        assert_eq!(
            resolve(json!({ "type": "docker-compose" })).resolved_command,
            "docker compose -f docker-compose.yml up"
        );
    }

    #[test]
    fn unknown_type_uses_command() {
        assert_eq!(resolve(json!({ "type": "weird", "command": "do it" })).resolved_command, "do it");
        assert_eq!(resolve(json!({})).resolved_command, "");
    }

    #[test]
    fn zero_debug_port_is_kept() {
        let r = resolve(json!({ "type": "node", "debugPort": 0 }));
        assert_eq!(r.resolved_debug_port, Some(json!(0)));
        assert!(r.resolved_debug_command.unwrap().contains("--inspect=0.0.0.0:0 "));
    }
}
