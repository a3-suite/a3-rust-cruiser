// integration_id: logical-module-boundary-cli-working-directory

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(unix)]
use serde_json::Value;

#[cfg(unix)]
static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[cfg(unix)]
#[test]
fn should_rustc_is_invoked_from_the_project_root_when_cli_runs_from_another_directory() {
    // Arrange
    let fixture = TempFixture::new("rustc-project-root");
    let shim_directory = fixture.join("rustc-shim");
    let unrelated_directory = fixture.join("unrelated");
    let cwd_log = fixture.join("rustc-cwd.log");
    let expected_project_root =
        fs::canonicalize(fixture.path()).expect("fixture root should be canonicalizable");
    fs::create_dir_all(&unrelated_directory).expect("unrelated directory should be created");
    fixture.write(
        "rustc-shim/rustc",
        r#"#!/bin/sh
printf '%s\n' "$PWD" > "$A3_RUST_CRUISER_CWD_FILE"
if [ "$PWD" != "$A3_RUST_CRUISER_EXPECTED_CWD" ]; then exit 97; fi
printf '%s\n' 'target_arch="aarch64"' 'target_os="macos"' 'target_family="unix"' 'unix' 'target_pointer_width="64"'
"#,
    );
    let shim = shim_directory.join("rustc");
    let mut permissions = fs::metadata(&shim)
        .expect("rustc shim should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shim, permissions).expect("rustc shim should be executable");
    let mut path = std::ffi::OsString::from(&shim_directory);
    if let Some(existing) = std::env::var_os("PATH") {
        path.push(":");
        path.push(existing);
    }

    // Act
    let target = fixture.target();
    let output = Command::new(env!("CARGO_BIN_EXE_a3-rust-cruiser"))
        .current_dir(&unrelated_directory)
        .env("PATH", path)
        .env("A3_RUST_CRUISER_CWD_FILE", &cwd_log)
        .env("A3_RUST_CRUISER_EXPECTED_CWD", &expected_project_root)
        .args([
            "check",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "--policy",
            fixture
                .join("policy.yaml")
                .to_str()
                .expect("policy path is UTF-8"),
            "--package",
            fixture.package(),
            "--target",
            target.as_str(),
            "--target-triple",
            "aarch64-apple-darwin",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");

    // Assert
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON report");
    assert_eq!(report["decision"], "conformant");
    assert_eq!(
        fs::read_to_string(cwd_log)
            .expect("rustc shim should record its cwd")
            .trim(),
        expected_project_root
            .to_str()
            .expect("fixture path is UTF-8")
    );
}

#[cfg(unix)]
struct TempFixture {
    root: PathBuf,
    package: String,
}

#[cfg(unix)]
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

#[cfg(unix)]
impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
