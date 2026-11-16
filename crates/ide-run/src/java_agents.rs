//! Plugin Java agents added to a JVM application's command line.
//!
//! The host decides which jar (and whether the user granted it); this module
//! only inserts the argument. Arguments are separate argv items, so no shell
//! quoting is involved.

use crate::RunConfig;
use std::path::PathBuf;

/// The configuration kinds an agent may be added to.
pub const KINDS: [&str; 3] = ["application", "spring-boot", "jar-application"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaAgent {
    pub path: PathBuf,
    pub args: Option<String>,
}

impl JavaAgent {
    fn option(&self) -> String {
        format!(
            "-javaagent:{}{}",
            self.path.display(),
            self.args.as_deref().map(|a| format!("={a}")).unwrap_or_default()
        )
    }
}

/// `config` with `agent` after its VM options and before `-cp` / the main
/// class. Returns `None` when the configuration kind is not supported or the
/// same agent is already present.
pub fn with_agent(config: &RunConfig, agent: &JavaAgent) -> Option<RunConfig> {
    if !KINDS.contains(&config.kind.as_str()) {
        return None;
    }
    let marker = format!("-javaagent:{}", agent.path.display());
    if config.args.iter().any(|arg| arg.contains(&marker)) {
        return None;
    }
    let option = agent.option();
    let at = insertion_index(&config.args);
    let mut args = config.args.clone();
    args.insert(at, option);
    Some(RunConfig { args, ..config.clone() })
}

/// Where a JVM option goes: before `-cp`, `-jar`, `-m`, or the main class, so
/// existing user VM options stay first and the agent never becomes a program
/// argument.
fn insertion_index(args: &[String]) -> usize {
    const WITH_VALUE: &[&str] = &[
        "--add-opens",
        "--add-exports",
        "--add-reads",
        "--add-modules",
        "--patch-module",
        "--upgrade-module-path",
        "--limit-modules",
        "--enable-native-access",
    ];
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if matches!(arg, "-cp" | "-classpath" | "--class-path" | "-p" | "--module-path" | "-jar" | "-m" | "--module")
            || !arg.starts_with('-')
        {
            return i;
        }
        i += if WITH_VALUE.contains(&arg) { 2 } else { 1 };
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(kind: &str, args: &[&str]) -> RunConfig {
        serde_json::from_value(serde_json::json!({
            "id": "a", "name": "App", "kind": kind, "command": "java",
            "args": args, "cwd": null, "env": {}, "detected": false,
            "isTest": false, "debugAdapter": null
        }))
        .unwrap()
    }

    #[test]
    fn adds_after_vm_options_and_before_classpath() {
        let c = config("application", &["-Xmx512m", "-Dapp=1", "-cp", "${classpath}", "a.App"]);
        let next = with_agent(&c, &JavaAgent { path: "/p/agent.jar".into(), args: None }).unwrap();
        assert_eq!(next.args, ["-Xmx512m", "-Dapp=1", "-javaagent:/p/agent.jar", "-cp", "${classpath}", "a.App"]);
    }

    #[test]
    fn handles_spaces_without_shell_quoting() {
        let c = config("spring-boot", &["-cp", "cp", "a.App"]);
        let next = with_agent(&c, &JavaAgent { path: "/p/Agent Folder/agent.jar".into(), args: Some("--x=1".into()) })
            .unwrap();
        assert_eq!(next.args[0], "-javaagent:/p/Agent Folder/agent.jar=--x=1");
    }

    #[test]
    fn deduplicates_and_limits_kinds() {
        let c = config("application", &["-javaagent:/p/agent.jar", "-cp", "cp", "a.App"]);
        assert!(with_agent(&c, &JavaAgent { path: "/p/agent.jar".into(), args: None }).is_none());
        for kind in ["maven", "gradle", "tomcat", "java-file", "plugin"] {
            assert!(with_agent(&config(kind, &["a"]), &JavaAgent { path: "/p/a.jar".into(), args: None }).is_none());
        }
    }

    #[test]
    fn keeps_separate_value_vm_options_together() {
        let c = config("application", &["--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", "cp", "a.App"]);
        assert_eq!(
            with_agent(&c, &JavaAgent { path: "/p/a.jar".into(), args: None }).unwrap().args,
            ["--add-opens", "java.base/java.lang=ALL-UNNAMED", "-javaagent:/p/a.jar", "-cp", "cp", "a.App"]
        );
    }

    #[test]
    fn inserts_before_jar_and_module_launches() {
        let jar = config("jar-application", &["-Xmx1g", "-jar", "app.jar"]);
        assert_eq!(
            with_agent(&jar, &JavaAgent { path: "/p/a.jar".into(), args: None }).unwrap().args,
            ["-Xmx1g", "-javaagent:/p/a.jar", "-jar", "app.jar"]
        );
        let module = config("application", &["-p", "mods", "-m", "app/main"]);
        assert_eq!(
            with_agent(&module, &JavaAgent { path: "/p/a.jar".into(), args: None }).unwrap().args,
            ["-javaagent:/p/a.jar", "-p", "mods", "-m", "app/main"]
        );
    }
}
