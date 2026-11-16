//! The `javaAgents` extension point: a Java agent a plugin wants the IDE to
//! add to the applications it starts.
//!
//! ```toml
//! [[contributes.javaAgents]]
//! path = "agent/agent.jar"
//! args = "optional-arguments"
//! ```
//!
//! One agent per plugin. The path is resolved with [`Package::resolve`], so a
//! missing file or a symlink leaving the package fails at install time.

use ide_extension::{Capability, Disposable, ExtensionPoint, Package, Placement, Prepared, decode};
use parking_lot::RwLock;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

pub const POINT_ID: &str = "javaAgents";
const MAX_ARGS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaAgent {
    pub plugin: String,
    /// Canonical path inside the plugin package.
    pub path: PathBuf,
    pub args: Option<String>,
}

#[derive(Default)]
pub struct JavaAgents {
    items: RwLock<Vec<JavaAgent>>,
}

impl JavaAgents {
    pub fn for_plugin(&self, plugin: &str) -> Option<JavaAgent> {
        self.items.read().iter().find(|item| item.plugin == plugin).cloned()
    }

    pub fn all(&self) -> Vec<JavaAgent> {
        self.items.read().clone()
    }
}

pub struct JavaAgentsPoint {
    store: Arc<JavaAgents>,
}

impl JavaAgentsPoint {
    pub fn new(store: Arc<JavaAgents>) -> Self {
        Self { store }
    }
}

impl ExtensionPoint for JavaAgentsPoint {
    fn id(&self) -> &'static str {
        POINT_ID
    }

    fn placement(&self) -> Placement {
        Placement::Workspace
    }

    fn prepare(&self, pkg: &Package, decl: toml::Value) -> Result<Prepared, String> {
        pkg.manifest.needs_api("contributes.javaAgents", 9)?;
        if pkg.manifest.declarations(POINT_ID).len() > 1 {
            return Err("a plugin can declare one Java agent".into());
        }
        let Decl { path, args } = decode(POINT_ID, decl)?;
        let path = PathBuf::from(path);
        let real = pkg.resolve(&path)?;
        let metadata = std::fs::metadata(&real).map_err(|e| format!("javaAgents.path: {e}"))?;
        if !metadata.is_file() {
            return Err("javaAgents.path: not a file".into());
        }
        if args.as_ref().is_some_and(|value| value.len() > MAX_ARGS) {
            return Err(format!("javaAgents.args: at most {MAX_ARGS} bytes"));
        }
        let plugin = pkg.id().to_owned();
        let agent = JavaAgent { plugin: plugin.clone(), path: real, args: args.clone() };
        let store = self.store.clone();
        let cleanup_plugin = plugin.clone();
        let label = format!("{plugin} Java agent");
        Ok(Prepared::new(label, move || {
            {
                let mut items = store.items.write();
                if items.iter().any(|item| item.plugin == agent.plugin) {
                    return Err("a plugin can declare one Java agent".into());
                }
                items.push(agent);
            }
            let weak = Arc::downgrade(&store);
            Ok(Disposable::new(move || {
                if let Some(store) = weak.upgrade() {
                    store.items.write().retain(|item| item.plugin != cleanup_plugin);
                }
            }))
        })
        .file(path)
        .capability(Capability::JvmAgent))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decl {
    path: String,
    #[serde(default)]
    args: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ide_extension::Manifest;

    const MANIFEST: &str = "id = \"acme.agent\"\nname = \"Agent\"\nversion = \"1.0.0\"\nengines.openidea = \"^1.9\"\n";

    fn package(dir: &std::path::Path, declarations: &str) -> Package {
        let manifest = Manifest::parse(&format!("{MANIFEST}{declarations}")).unwrap();
        Package::new(manifest, dir.to_path_buf())
    }

    fn decl(pkg: &Package) -> toml::Value {
        pkg.manifest.declarations(POINT_ID).into_iter().next().unwrap()
    }

    #[test]
    fn a_relative_jar_becomes_a_granted_contribution() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("agent")).unwrap();
        std::fs::write(dir.path().join("agent/agent.jar"), b"jar").unwrap();
        let pkg = package(dir.path(), "[[contributes.javaAgents]]\npath = \"agent/agent.jar\"\nargs = \"--x=1\"\n");
        let prepared = JavaAgentsPoint::new(Arc::default()).prepare(&pkg, decl(&pkg)).unwrap();
        assert!(prepared.capabilities.contains(&Capability::JvmAgent));
        assert!(prepared.files.contains(&PathBuf::from("agent/agent.jar")));
        assert_eq!(prepared.label, "acme.agent Java agent");
    }

    #[test]
    fn one_agent_per_plugin_and_only_inside_the_package() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("agent")).unwrap();
        std::fs::write(dir.path().join("agent/agent.jar"), b"jar").unwrap();
        let pkg = package(
            dir.path(),
            "[[contributes.javaAgents]]\npath = \"agent/agent.jar\"\n[[contributes.javaAgents]]\npath = \"agent/agent.jar\"\n",
        );
        assert!(JavaAgentsPoint::new(Arc::default()).prepare(&pkg, decl(&pkg)).is_err());
        let absolute = package(dir.path(), "[[contributes.javaAgents]]\npath = \"/tmp/agent.jar\"\n");
        assert!(JavaAgentsPoint::new(Arc::default()).prepare(&absolute, decl(&absolute)).is_err());
        let missing = package(dir.path(), "[[contributes.javaAgents]]\npath = \"agent/missing.jar\"\n");
        assert!(JavaAgentsPoint::new(Arc::default()).prepare(&missing, decl(&missing)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_leaving_the_package_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        let outside = outside_dir.path().join("outside.jar");
        std::fs::write(&outside, b"jar").unwrap();
        std::fs::create_dir(dir.path().join("agent")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.path().join("agent/agent.jar")).unwrap();
        let pkg = package(dir.path(), "[[contributes.javaAgents]]\npath = \"agent/agent.jar\"\n");
        assert!(JavaAgentsPoint::new(Arc::default()).prepare(&pkg, decl(&pkg)).is_err());
    }
}
