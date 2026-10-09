//! Architecture regression test: the runtime side never links the GUI, and
//! the GUI never links the GPU renderer or script VM. See
//! `docs/architecture.md` (rule 5). Only normal and build dependencies count;
//! dev-dependencies don't ship.

use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

struct Graph {
    names: HashMap<String, String>,
    deps: HashMap<String, Vec<String>>,
}

impl Graph {
    fn load() -> Self {
        let out = std::process::Command::new(env!("CARGO"))
            .args(["metadata", "--format-version", "1", "--locked"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("running cargo metadata");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let meta: Value = serde_json::from_slice(&out.stdout).unwrap();

        let names = meta["packages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p["id"].as_str().unwrap().to_owned(), p["name"].as_str().unwrap().to_owned()))
            .collect();
        let mut deps = HashMap::new();
        for node in meta["resolve"]["nodes"].as_array().unwrap() {
            let shipped = node["deps"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|d| {
                    d["dep_kinds"].as_array().unwrap().iter().any(|k| k["kind"].as_str() != Some("dev"))
                })
                .map(|d| d["pkg"].as_str().unwrap().to_owned())
                .collect();
            deps.insert(node["id"].as_str().unwrap().to_owned(), shipped);
        }
        Self { names, deps }
    }

    /// Names of all packages `root` depends on, transitively.
    fn closure(&self, root: &str) -> BTreeSet<String> {
        let start = self.names.iter().find(|(_, n)| *n == root).map(|(id, _)| id.clone());
        let mut stack = vec![start.unwrap_or_else(|| panic!("no package {root}"))];
        let mut seen = BTreeSet::new();
        while let Some(id) = stack.pop() {
            for dep in &self.deps[&id] {
                if seen.insert(self.names[dep].clone()) {
                    stack.push(dep.clone());
                }
            }
        }
        seen
    }
}

fn assert_excludes(graph: &Graph, root: &str, forbidden: &[&str]) {
    let closure = graph.closure(root);
    let found: Vec<_> = forbidden.iter().filter(|f| closure.contains(**f)).collect();
    assert!(found.is_empty(), "{root} must not depend on {found:?}");
}

#[test]
fn runtime_crates_never_link_the_gui() {
    let graph = Graph::load();
    for root in [
        "backstage_core",
        "backstage_protocol",
        "backstage_render",
        "backstage_script",
        "backstage_stage",
        "backstage_player",
    ] {
        assert_excludes(&graph, root, &["gtk4", "relm4", "backstage_tools"]);
    }
}

#[test]
fn tools_never_links_gpu_or_vm() {
    assert_excludes(
        &Graph::load(),
        "backstage_tools",
        &["wgpu", "lyon", "backstage_render", "backstage_script"],
    );
}

#[test]
fn nothing_ships_the_ui_test_driver() {
    // backstage_uidriver is a test tool: tests start it as a separate binary.
    let graph = Graph::load();
    for root in [
        "backstage_core",
        "backstage_protocol",
        "backstage_render",
        "backstage_script",
        "backstage_stage",
        "backstage_player",
        "backstage_tools",
    ] {
        assert_excludes(&graph, root, &["backstage_uidriver", "wayland-protocols-misc"]);
    }
}

#[test]
fn core_is_pure_data() {
    // The document model is shared by every process and must stay headless.
    assert_excludes(&Graph::load(), "backstage_core", &["wgpu", "gtk4", "relm4", "lyon", "winit"]);
}
