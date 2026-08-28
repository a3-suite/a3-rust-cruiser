// integration_id: logical-module-boundary-check-command

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[test]
fn should_check_accepts_sdd_design_as_the_boundary_policy_source() {
    // Arrange
    let fixture = TempFixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"sdd-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    fixture.write(
        "src/main.rs",
        "mod application;\nmod infrastructure;\nfn main() {}\n",
    );
    fixture.write(
        "src/application.rs",
        "use crate::infrastructure::Thing;\npub fn run() -> Thing { Thing }\n",
    );
    fixture.write("src/infrastructure.rs", "pub struct Thing;\n");
    fixture.write(
        "logical-structure.sdd.yml",
        "version: 1\nname: logical-structure\nstructure_id: fixture\ncomponents:\n  - component_id: application\n    responsibility: application\n    kind_ref: application-service\n  - component_id: infrastructure\n    responsibility: infrastructure\n    kind_ref: project-input-adapter\ndependencies: []\n",
    );
    fixture.write(
        "projection.sdd.yml",
        "version: 1\nname: projection\nprojection_id: fixture-source\nmappings:\n  - component_id: application\n    node_id: src-application\n  - component_id: infrastructure\n    node_id: src-infrastructure\n",
    );
    fixture.write(
        "directory-structure.sdd.yml",
        "version: 1\nname: directory-structure\nstructure_id: fixture-source\nnodes:\n  - node_id: src-application\n    path: src/application.rs\n    primary_area_kind: source\n    area_kinds: [source]\n    responsibility: application\n  - node_id: src-infrastructure\n    path: src/infrastructure.rs\n    primary_area_kind: source\n    area_kinds: [source]\n    responsibility: infrastructure\n",
    );

    // Act
    let output = Command::new(env!("CARGO_BIN_EXE_a3-rust-cruiser"))
        .arg("check")
        .arg(fixture.path())
        .arg("--logical-structure")
        .arg(fixture.join("logical-structure.sdd.yml"))
        .arg("--projection")
        .arg(fixture.join("projection.sdd.yml"))
        .arg("--directory-structure")
        .arg(fixture.join("directory-structure.sdd.yml"))
        .args([
            "--package",
            "sdd-fixture",
            "--target",
            "bin:sdd-fixture",
            "--target-triple",
            "x86_64-unknown-linux-gnu",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");

    // Assert
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON report");
    assert_eq!(report["decision"], "non-conformant");
    assert_eq!(report["violations"][0]["source"], "crate::application");
    assert_eq!(report["violations"][0]["target"], "crate::infrastructure");
}

#[test]
fn should_check_reports_forbidden_dependency_as_json_and_returns_one() {
    // Arrange
    let fixture = TempFixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    fixture.write(
        "src/main.rs",
        "mod application;\nmod infrastructure;\nfn main() {}\n",
    );
    fixture.write(
        "src/application.rs",
        "use crate::infrastructure::Thing;\npub fn run() -> Thing { Thing }\n",
    );
    fixture.write("src/infrastructure.rs", "pub struct Thing;\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbidden-application\n    kind: forbidden\n    severity: error\n    from: [crate::application]\n    to: [crate::infrastructure]\n",
    );

    // Act
    let output = Command::new(env!("CARGO_BIN_EXE_a3-rust-cruiser"))
        .args([
            "check",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "--policy",
            fixture
                .join("policy.yaml")
                .to_str()
                .expect("policy path is UTF-8"),
            "--package",
            "fixture",
            "--target",
            "bin:fixture",
            "--target-triple",
            "x86_64-unknown-linux-gnu",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");

    // Assert
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON report");
    assert_eq!(report["decision"], "non-conformant");
    assert_eq!(report["violations"][0]["rule_id"], "forbidden-application");
    assert_eq!(report["violations"][0]["dependency_kind"], "import");

    let text_output = Command::new(env!("CARGO_BIN_EXE_a3-rust-cruiser"))
        .args([
            "check",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "--policy",
            fixture
                .join("policy.yaml")
                .to_str()
                .expect("policy path is UTF-8"),
            "--package",
            "fixture",
            "--target",
            "bin:fixture",
            "--target-triple",
            "x86_64-unknown-linux-gnu",
            "--format",
            "text",
        ])
        .output()
        .expect("binary should run");
    assert_eq!(text_output.status.code(), Some(1));
    assert!(text_output.stderr.is_empty());
    let text = String::from_utf8(text_output.stdout).expect("text report is UTF-8");
    assert!(text.contains("decision: non-conformant"));
    assert!(text.contains("forbidden-application"));

    let excluded_output = Command::new(env!("CARGO_BIN_EXE_a3-rust-cruiser"))
        .args([
            "check",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "--policy",
            fixture
                .join("policy.yaml")
                .to_str()
                .expect("policy path is UTF-8"),
            "--package",
            "fixture",
            "--target",
            "bin:fixture",
            "--target-triple",
            "x86_64-unknown-linux-gnu",
            "--exclude",
            "crate::infrastructure",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert_eq!(excluded_output.status.code(), Some(0));
    let excluded_report: serde_json::Value =
        serde_json::from_slice(&excluded_output.stdout).expect("valid JSON report");
    assert_eq!(excluded_report["decision"], "conformant");
    assert_eq!(
        excluded_report["excluded_scopes"][0],
        "crate::infrastructure"
    );
}

struct TempFixture {
    root: PathBuf,
}

impl TempFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "a3-rust-cruiser-fixture-{}-{}",
            std::process::id(),
            FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("src")).expect("fixture directory should be created");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn join(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    fn write(&self, path: &str, content: &str) {
        fs::write(self.root.join(path), content).expect("fixture file should be written");
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
