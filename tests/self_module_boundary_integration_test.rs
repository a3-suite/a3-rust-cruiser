use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

use serde::Deserialize;
use serde_json::{Value, json};

static SCRATCH_COUNTER: AtomicUsize = AtomicUsize::new(0);

// integration_id: self-module-boundary-sdd-projection
mod self_module_boundary_sdd_projection {
    use super::*;

    #[test]
    fn should_enforce_current_dependencies_with_source_union_profile() {
        // Arrange
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let design = ModuleBoundaryDesign::load(project_root);
        let scratch = ScratchDirectory::new("current-project");
        let policy_path = scratch.write("boundary-policy.yaml", &design.dependency_policy_yaml());

        // Act
        let output = run_check(
            project_root,
            &policy_path,
            env!("CARGO_PKG_NAME"),
            &format!("bin:{}", env!("CARGO_PKG_NAME")),
        );
        let report = report_from(&output, &[0]);

        // Assert
        assert_eq!(
            report["analysis_context"]["analysis_profile"],
            "source-union"
        );
        assert_eq!(report["decision"], "conformant");
        assert_eq!(report["analysis_issues"], json!([]));
        assert_eq!(report["violations"], json!([]));
        assert_eq!(
            observed_top_level_modules(&report),
            design.top_level_modules
        );
        assert_eq!(
            observed_component_dependencies(&report, &design),
            design.dependencies
        );
    }

    #[test]
    fn should_accept_sdd_declared_component_dependencies() {
        // Arrange
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let design = ModuleBoundaryDesign::load(project_root);
        let fixture = ScratchDirectory::new("declared-dependencies");
        write_component_fixture(&fixture, &design, &design.dependencies);
        let policy_path = fixture.write("boundary-policy.yaml", &design.dependency_policy_yaml());

        // Act
        let output = run_check(
            fixture.path(),
            &policy_path,
            "self-boundary-fixture",
            "bin:self-boundary-fixture",
        );
        let report = report_from(&output, &[0]);

        // Assert
        assert_eq!(report["decision"], "conformant");
        assert_eq!(report["analysis_issues"], json!([]));
        assert_eq!(report["violations"], json!([]));
        assert_eq!(
            observed_component_dependencies(&report, &design),
            design.dependencies
        );
    }

    #[test]
    fn should_reject_an_undeclared_component_dependency() {
        // Arrange
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let design = ModuleBoundaryDesign::load(project_root);
        let (source, target) = design
            .first_forbidden_dependency()
            .expect("the design must contain at least one forbidden dependency");
        let fixture = ScratchDirectory::new("forbidden-dependency");
        let mut dependencies = design.dependencies.clone();
        dependencies.insert((source.clone(), target.clone()));
        write_component_fixture(&fixture, &design, &dependencies);
        let policy_path = fixture.write("boundary-policy.yaml", &design.dependency_policy_yaml());

        // Act
        let output = run_check(
            fixture.path(),
            &policy_path,
            "self-boundary-fixture",
            "bin:self-boundary-fixture",
        );
        let report = report_from(&output, &[1]);

        // Assert
        assert!(report["violations"].as_array().is_some_and(|violations| {
            violations.iter().any(|violation| {
                violation["kind"] == "forbidden"
                    && violation["source"] == source
                    && violation["target"] == target
            })
        }));
    }

    #[test]
    fn should_keep_the_sdd_component_graph_acyclic() {
        // Arrange
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let design = ModuleBoundaryDesign::load(project_root);
        let fixture = ScratchDirectory::new("component-cycle");
        write_component_fixture(&fixture, &design, &design.dependencies);
        let policy_path = fixture.write("boundary-policy.yaml", &design.cycle_policy_yaml());

        // Act
        let output = run_check(
            fixture.path(),
            &policy_path,
            "self-boundary-fixture",
            "bin:self-boundary-fixture",
        );
        let report = report_from(&output, &[0]);

        // Assert
        assert_eq!(report["violations"], json!([]));
        assert_eq!(report["policy_evaluation"]["cycle_outcome"], "acyclic");
    }

    #[test]
    fn should_reject_a_component_cycle() {
        // Arrange
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let design = ModuleBoundaryDesign::load(project_root);
        let (source, target) = design
            .dependencies
            .iter()
            .next()
            .expect("the design must contain at least one dependency");
        let mut dependencies = design.dependencies.clone();
        dependencies.insert((target.clone(), source.clone()));
        let fixture = ScratchDirectory::new("component-cycle-negative");
        write_component_fixture(&fixture, &design, &dependencies);
        let policy_path = fixture.write("boundary-policy.yaml", &design.cycle_policy_yaml());

        // Act
        let output = run_check(
            fixture.path(),
            &policy_path,
            "self-boundary-fixture",
            "bin:self-boundary-fixture",
        );
        let report = report_from(&output, &[1]);

        // Assert
        assert!(report["violations"].as_array().is_some_and(|violations| {
            violations
                .iter()
                .any(|violation| violation["kind"] == "circular")
        }));
    }
}

#[derive(Debug)]
struct ModuleBoundaryDesign {
    component_modules: BTreeMap<String, String>,
    dependencies: BTreeSet<(String, String)>,
    top_level_modules: BTreeSet<String>,
}

impl ModuleBoundaryDesign {
    fn load(project_root: &Path) -> Self {
        let logical: LogicalStructure = read_yaml(&project_root.join(
            "sdd/dsl/designs/scopes/logical-module-boundary-validation/logical-structure.sdd.yml",
        ));
        let projection: Projection =
            read_yaml(&project_root.join(
                "sdd/dsl/designs/scopes/logical-module-boundary-validation/projection.sdd.yml",
            ));
        let directories: DirectoryStructure =
            read_yaml(&project_root.join("sdd/dsl/designs/global/directory-structure.sdd.yml"));

        let nodes: BTreeMap<_, _> = directories
            .nodes
            .into_iter()
            .map(|node| (node.node_id, node.path))
            .collect();
        let component_modules: BTreeMap<_, _> = projection
            .mappings
            .into_iter()
            .map(|mapping| {
                let path = nodes.get(&mapping.node_id).unwrap_or_else(|| {
                    panic!("projection references unknown node {}", mapping.node_id)
                });
                (mapping.component_id, rust_module_id(path))
            })
            .collect();
        let component_ids: BTreeSet<_> = logical
            .components
            .into_iter()
            .map(|component| component.component_id)
            .collect();
        assert_eq!(
            component_modules.keys().cloned().collect::<BTreeSet<_>>(),
            component_ids,
            "every logical component must have exactly one physical projection"
        );

        let dependencies = logical
            .dependencies
            .into_iter()
            .map(|dependency| {
                (
                    component_modules[&dependency.from_component_id].clone(),
                    component_modules[&dependency.to_component_id].clone(),
                )
            })
            .collect();
        let top_level_modules = component_modules
            .values()
            .map(|module| top_level_module(module))
            .collect();
        assert!(
            component_modules
                .values()
                .all(|module| top_level_module(module) == *module),
            "self boundary fixtures require components projected to crate-root modules"
        );

        Self {
            component_modules,
            dependencies,
            top_level_modules,
        }
    }

    fn dependency_policy_yaml(&self) -> String {
        let modules: Vec<_> = self.component_modules.values().cloned().collect();
        let mut rules = Vec::new();
        for source in &modules {
            for target in &modules {
                if source != target
                    && !self
                        .dependencies
                        .contains(&(source.clone(), target.clone()))
                {
                    rules.push(json!({
                        "id": forbidden_rule_id(source, target),
                        "kind": "forbidden",
                        "severity": "error",
                        "from": module_patterns(source),
                        "to": module_patterns(target),
                    }));
                }
            }
        }
        serde_yaml::to_string(&json!({ "rules": rules }))
            .expect("derived boundary policy must be serializable")
    }

    fn cycle_policy_yaml(&self) -> String {
        serde_yaml::to_string(&json!({
            "rules": [{
                "id": "self-no-component-cycles",
                "kind": "circular",
                "severity": "error",
                "modules": self.component_modules.values().collect::<Vec<_>>(),
            }]
        }))
        .expect("derived cycle policy must be serializable")
    }

    fn first_forbidden_dependency(&self) -> Option<(String, String)> {
        for source in self.component_modules.values() {
            for target in self.component_modules.values() {
                let dependency = (source.clone(), target.clone());
                if source != target && !self.dependencies.contains(&dependency) {
                    return Some(dependency);
                }
            }
        }
        None
    }
}

#[derive(Deserialize)]
struct LogicalStructure {
    components: Vec<LogicalComponent>,
    dependencies: Vec<LogicalDependency>,
}

#[derive(Deserialize)]
struct LogicalComponent {
    component_id: String,
}

#[derive(Deserialize)]
struct LogicalDependency {
    from_component_id: String,
    to_component_id: String,
}

#[derive(Deserialize)]
struct Projection {
    mappings: Vec<ProjectionMapping>,
}

#[derive(Deserialize)]
struct ProjectionMapping {
    component_id: String,
    node_id: String,
}

#[derive(Deserialize)]
struct DirectoryStructure {
    nodes: Vec<DirectoryNode>,
}

#[derive(Deserialize)]
struct DirectoryNode {
    node_id: String,
    path: String,
}

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_yaml::from_str(&content)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn rust_module_id(path: &str) -> String {
    let relative = path
        .strip_prefix("src/")
        .unwrap_or_else(|| panic!("component path must be below src/: {path}"));
    assert!(
        !relative.is_empty(),
        "src root cannot represent a component"
    );
    assert!(
        !relative.ends_with(".rs"),
        "component projection must target a module directory: {path}"
    );
    format!("crate::{}", relative.replace('/', "::"))
}

fn top_level_module(module: &str) -> String {
    let first_segment = module
        .strip_prefix("crate::")
        .and_then(|relative| relative.split("::").next())
        .expect("projected component must be a crate module");
    format!("crate::{first_segment}")
}

fn module_patterns(module: &str) -> [String; 2] {
    [module.to_owned(), format!("{module}::**")]
}

fn forbidden_rule_id(source: &str, target: &str) -> String {
    format!(
        "self-forbid-{}-to-{}",
        source
            .trim_start_matches("crate::")
            .replace("::", "-")
            .replace('_', "-"),
        target
            .trim_start_matches("crate::")
            .replace("::", "-")
            .replace('_', "-")
    )
}

fn fixture_entrypoint(design: &ModuleBoundaryDesign) -> String {
    let mut declarations = String::new();
    for module in &design.top_level_modules {
        let name = module
            .strip_prefix("crate::")
            .expect("fixture modules must be crate children");
        declarations.push_str(&format!("mod {name};\n"));
    }
    declarations.push_str("fn main() {}\n");
    declarations
}

fn write_component_fixture(
    fixture: &ScratchDirectory,
    design: &ModuleBoundaryDesign,
    dependencies: &BTreeSet<(String, String)>,
) {
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"self-boundary-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    fixture.write("src/main.rs", &fixture_entrypoint(design));
    for module in design.component_modules.values() {
        let mut body = String::new();
        for (_, target) in dependencies.iter().filter(|(source, _)| source == module) {
            body.push_str(&format!("use {target};\n"));
        }
        body.push_str("pub fn marker() {}\n");
        fixture.write(&module_file(module), &body);
    }
}

fn module_file(module: &str) -> String {
    format!(
        "src/{}.rs",
        module
            .strip_prefix("crate::")
            .expect("fixture modules must be crate children")
            .replace("::", "/")
    )
}

fn run_check(project_root: &Path, policy: &Path, package: &str, target: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_a3-rust-cruiser"))
        .arg("check")
        .arg(project_root)
        .arg("--policy")
        .arg(policy)
        .arg("--package")
        .arg(package)
        .arg("--target")
        .arg(target)
        .arg("--target-triple")
        .arg(host_triple())
        .arg("--analysis-profile")
        .arg("source-union")
        .arg("--format")
        .arg("json")
        .output()
        .expect("a3-rust-cruiser must execute")
}

fn host_triple() -> String {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("rustc must be available while running cargo tests");
    assert!(output.status.success(), "rustc -vV must succeed");
    String::from_utf8(output.stdout)
        .expect("rustc output must be UTF-8")
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .expect("rustc output must contain the host triple")
}

fn report_from(output: &Output, expected_exit_codes: &[i32]) -> Value {
    assert!(
        output
            .status
            .code()
            .is_some_and(|code| expected_exit_codes.contains(&code)),
        "unexpected exit status {:?}; stdout: {}; stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "a3-rust-cruiser must return JSON: {error}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn observed_top_level_modules(report: &Value) -> BTreeSet<String> {
    report["module_declarations"]
        .as_array()
        .expect("module_declarations must be an array")
        .iter()
        .filter(|declaration| declaration["parent"] == "crate")
        .map(|declaration| {
            declaration["child"]
                .as_str()
                .expect("module declaration child must be text")
                .to_owned()
        })
        .collect()
}

fn observed_component_dependencies(
    report: &Value,
    design: &ModuleBoundaryDesign,
) -> BTreeSet<(String, String)> {
    report["dependencies"]
        .as_array()
        .expect("dependencies must be an array")
        .iter()
        .filter_map(|dependency| {
            let source = dependency["source"].as_str()?;
            let target = dependency["target"].as_str()?;
            let source_component = component_for(source, design)?;
            let target_component = component_for(target, design)?;
            (source_component != target_component)
                .then(|| (source_component.to_owned(), target_component.to_owned()))
        })
        .collect()
}

fn component_for<'a>(module: &str, design: &'a ModuleBoundaryDesign) -> Option<&'a str> {
    design
        .component_modules
        .values()
        .filter(|component| {
            module == component.as_str()
                || module
                    .strip_prefix(component.as_str())
                    .is_some_and(|suffix| suffix.starts_with("::"))
        })
        .max_by_key(|component| component.len())
        .map(String::as_str)
}

struct ScratchDirectory {
    path: PathBuf,
}

impl ScratchDirectory {
    fn new(label: &str) -> Self {
        let sequence = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "a3-rust-cruiser-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("scratch directory must be new and creatable");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("scratch parent must be creatable");
        }
        fs::write(&path, content).expect("scratch file must be writable");
        path
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let cleanup = fs::remove_dir_all(&self.path);
        if !std::thread::panicking() {
            cleanup.unwrap_or_else(|error| {
                panic!(
                    "failed to remove scratch directory {}: {error}",
                    self.path.display()
                )
            });
        }
    }
}
