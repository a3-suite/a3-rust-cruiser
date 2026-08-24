use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::Value;

static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

mod logical_module_boundary_contract_cases;

fn dependency_fixture(label: &str, target: &str) -> TempFixture {
    let fixture = TempFixture::new(label);
    fixture.write(
        "src/main.rs",
        &format!("mod application;\nmod {target};\nfn main() {{}}\n"),
    );
    fixture.write(
        "src/application.rs",
        &format!("use crate::{target}::Thing;\npub fn run() -> Thing {{ Thing }}\n"),
    );
    fixture.write(&format!("src/{target}.rs"), "pub struct Thing;\n");
    fixture
}

fn forbidden_fixture(label: &str, with_issue: bool) -> TempFixture {
    let fixture = dependency_fixture(label, "infrastructure");
    if with_issue {
        fixture.write(
            "src/main.rs",
            "mod application;\nmod infrastructure;\nfn main() { println!(\"incomplete\"); }\n",
        );
    }
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbidden-application\n    kind: forbidden\n    severity: error\n    from: [crate::application]\n    to: [crate::infrastructure]\n",
    );
    fixture
}

fn run_check(fixture: &TempFixture, format: &str) -> Output {
    run_check_with(fixture, "bin", format, &[])
}

fn run_check_with(
    fixture: &TempFixture,
    target_kind: &str,
    format: &str,
    extra_args: &[&str],
) -> Output {
    let target = if target_kind == "bin" {
        fixture.target()
    } else {
        target_kind.to_owned()
    };
    let mut args = extra_args.to_vec();
    args.extend(["--format", format]);
    run_custom(
        fixture,
        fixture.package(),
        &target,
        "x86_64-unknown-linux-gnu",
        &args,
    )
}

fn run_custom(
    fixture: &TempFixture,
    package: &str,
    target: &str,
    target_triple: &str,
    extra_args: &[&str],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_a3-rust-cruiser"));
    command
        .arg("check")
        .arg(fixture.path())
        .arg("--policy")
        .arg(fixture.join("policy.yaml"))
        .arg("--package")
        .arg(package)
        .arg("--target")
        .arg(target)
        .arg("--target-triple")
        .arg(target_triple)
        .args(extra_args);
    command.output().expect("binary should run")
}

fn completed_json(output: &Output, exit_code: i32) -> Value {
    assert_eq!(output.status.code(), Some(exit_code));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid JSON report")
}

fn completed_text(output: &Output, exit_code: i32) -> String {
    assert_eq!(output.status.code(), Some(exit_code));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).expect("text report is UTF-8")
}

fn assert_completed_output(output: &Output, exit_code: i32, decision: &str) {
    let report = completed_json(output, exit_code);
    assert_eq!(report["decision"], decision);
}

fn assert_execution_failure(output: &Output, expected: &str) {
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "stderr should contain {expected}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_issue(report: &Value, code: &str) {
    let issues = report["analysis_issues"].as_array().expect("issues array");
    let issue = issues
        .iter()
        .find(|issue| issue["code"] == code)
        .expect("expected issue code");
    assert!(
        issue["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
    assert!(
        issue["affected_scope"]
            .as_str()
            .is_some_and(|scope| !scope.is_empty())
    );
    assert_eq!(report["decision"], "indeterminate");
}

fn array_contains(array: &Value, field: &str, expected: &str) -> bool {
    array
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item[field] == expected))
}

fn dependency_exists(report: &Value, source: &str, target: &str, kind: &str, scope: &str) -> bool {
    report["dependencies"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["source"] == source
                && item["target"] == target
                && item["kind"] == kind
                && item["target_scope"] == scope
        })
    })
}

fn dependency_target_exists(report: &Value, source: &str, target: &str) -> bool {
    report["dependencies"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["source"] == source && item["target"] == target)
    })
}

struct TempFixture {
    root: PathBuf,
    package: String,
}

impl TempFixture {
    fn new(label: &str) -> Self {
        let package = format!("fixture-{label}");
        let root = std::env::temp_dir().join(format!(
            "a3-rust-cruiser-{label}-{}-{}",
            std::process::id(),
            FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("src")).expect("fixture directory should be created");
        let fixture = Self { root, package };
        fixture.write_manifest("");
        fixture.write("src/main.rs", "fn main() {}\n");
        fixture.write("policy.yaml", "rules: []\n");
        fixture
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn join(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    fn package(&self) -> &str {
        &self.package
    }

    fn target(&self) -> String {
        format!("bin:{}", self.package)
    }

    fn write_manifest(&self, suffix: &str) {
        self.write(
            "Cargo.toml",
            &format!(
                "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{suffix}",
                self.package
            ),
        );
    }

    fn write(&self, path: &str, content: &str) {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent should be created");
        }
        fs::write(path, content).expect("fixture file should be written");
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
