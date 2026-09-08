// contract_id: contract.logical-module-boundary.cli-outcome
// integration_id: logical-module-boundary-cli-outcome

use super::*;

#[test]
fn should_root_help_exits_with_zero() {
    // Arrange
    let binary = env!("CARGO_BIN_EXE_a3-rust-cruiser");

    // Act
    let output = Command::new(binary)
        .arg("--help")
        .output()
        .expect("binary should run");

    // Assert
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("Usage: a3-rust-cruiser <COMMAND>"));
    assert!(stdout.contains("check"));
}

#[test]
fn should_check_help_exits_with_zero() {
    // Arrange
    let binary = env!("CARGO_BIN_EXE_a3-rust-cruiser");

    // Act
    for option in ["--help", "-h"] {
        let output = Command::new(binary)
            .args(["check", option])
            .output()
            .expect("binary should run");

        // Assert
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
        assert!(stdout.contains("Usage: a3-rust-cruiser check <PROJECT_ROOT>"));
        assert!(stdout.contains(
            "--resolved-implementation-attribution <RESOLVED_IMPLEMENTATION_ATTRIBUTION_PATH>"
        ));
        assert!(stdout.contains("--analysis-profile <compilation|source-union>"));
    }
}

#[test]
fn should_version_exits_with_zero() {
    // Arrange
    let binary = env!("CARGO_BIN_EXE_a3-rust-cruiser");

    // Act
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .expect("binary should run");

    // Assert
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version is UTF-8"),
        format!("a3-rust-cruiser {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn should_conformant_result_exits_with_zero() {
    // Arrange
    let fixture = TempFixture::new("conformant-exit");

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_completed_output(&output, 0, "conformant");
}

#[test]
fn should_non_conformant_result_exits_with_one() {
    // Arrange
    let fixture = forbidden_fixture("non-conformant-exit", false);

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_completed_output(&output, 1, "non-conformant");
}

#[test]
fn should_indeterminate_result_exits_with_two() {
    // Arrange
    let fixture = policy_sensitive_fixture("indeterminate-exit");
    fixture.write("src/main.rs", "fn main() { custom_macro!(); }\n");

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_completed_output(&output, 2, "indeterminate");
}

#[test]
fn should_execution_failure_exits_with_three() {
    // Arrange
    let binary = env!("CARGO_BIN_EXE_a3-rust-cruiser");

    // Act
    let output = Command::new(binary)
        .arg("unknown")
        .output()
        .expect("binary should run");

    // Assert
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn should_stdout_write_failure_exits_with_three() {
    // Arrange
    use std::{
        fs::File,
        os::unix::io::{FromRawFd, IntoRawFd},
        os::unix::net::UnixStream,
        process::Stdio,
    };

    let binary = env!("CARGO_BIN_EXE_a3-rust-cruiser");
    let (child_stdout, peer) = UnixStream::pair().expect("socket pair should be created");
    drop(peer);
    let child_stdout = unsafe { File::from_raw_fd(child_stdout.into_raw_fd()) };

    // Act
    let output = Command::new(binary)
        .arg("--help")
        .stdout(Stdio::from(child_stdout))
        .output()
        .expect("binary should run");

    // Assert
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to write validation result"));
}

#[test]
fn should_missing_boundary_policy_source_is_rejected() {
    // Arrange
    let fixture = TempFixture::new("missing-boundary-policy-source");
    let target = fixture.target();

    // Act
    let output = run_check_without_policy(
        &fixture,
        fixture.package(),
        &target,
        "x86_64-unknown-linux-gnu",
        &["--format", "json"],
    );

    // Assert
    assert_execution_failure(
        &output,
        "--policy or --resolved-implementation-attribution is required",
    );
}

#[test]
fn should_project_root_is_required_for_check() {
    // Arrange
    let binary = env!("CARGO_BIN_EXE_a3-rust-cruiser");

    // Act
    let output = Command::new(binary)
        .arg("check")
        .output()
        .expect("binary should run");

    // Assert
    assert_execution_failure(&output, "Usage: a3-rust-cruiser check <PROJECT_ROOT>");
}

#[test]
fn should_invalid_cli_options_are_rejected() {
    // Arrange
    let fixture = TempFixture::new("invalid-cli-options");

    // Act
    let outputs = [
        (
            run_check_with(&fixture, "bin", "xml", &[]),
            "unsupported output format",
        ),
        (
            run_check_with(&fixture, "bin", "json", &["--unknown"]),
            "unknown option",
        ),
        (
            run_check_with(&fixture, "bin", "json", &["--include", "crate::?"]),
            "invalid logical module pattern",
        ),
    ];

    // Assert
    for (output, expected) in outputs {
        assert_execution_failure(&output, expected);
    }
}

#[test]
fn should_disabled_auto_discovery_rejects_hidden_target() {
    // Arrange
    let fixture = TempFixture::new("disabled-auto-discovery");
    fixture.write_manifest("autobins = false\n");
    fixture.write("src/bin/hidden.rs", "fn main() {}\n");

    // Act
    let output = run_check_with(&fixture, "bin:hidden", "json", &[]);

    // Assert
    assert_execution_failure(&output, "bin target not found");
}

#[test]
fn should_missing_target_source_is_rejected() {
    // Arrange
    let fixture = TempFixture::new("missing-target-source");
    fixture.write_manifest("[[bin]]\nname = \"missing\"\npath = \"src/bin/missing.rs\"\n");

    // Act
    let output = run_check_with(&fixture, "bin:missing", "json", &[]);

    // Assert
    assert_execution_failure(&output, "target source file not found");
}

#[test]
fn should_invalid_manifest_and_target_requests_fail_with_diagnostics() {
    // Arrange
    let fixture = TempFixture::new("invalid-targets");

    // Act
    let mismatch = run_custom(
        &fixture,
        "other-package",
        &fixture.target(),
        "x86_64-unknown-linux-gnu",
        &[],
    );
    let empty_bin = run_custom(
        &fixture,
        fixture.package(),
        "bin:",
        "x86_64-unknown-linux-gnu",
        &[],
    );
    let empty_example = run_custom(
        &fixture,
        fixture.package(),
        "example:",
        "x86_64-unknown-linux-gnu",
        &[],
    );
    let unsupported = run_custom(
        &fixture,
        fixture.package(),
        "proc-macro:fixture",
        "x86_64-unknown-linux-gnu",
        &[],
    );
    fixture.write("Cargo.toml", "not valid TOML = [\n");
    let invalid_manifest = run_check_with(&fixture, "lib", "json", &[]);

    // Assert
    assert_execution_failure(&mismatch, "does not match");
    assert_execution_failure(&empty_bin, "requires a name");
    assert_execution_failure(&empty_example, "requires a name");
    assert_execution_failure(&unsupported, "unsupported compilation target");
    assert_execution_failure(&invalid_manifest, "invalid Cargo.toml");
}

#[test]
fn should_manifest_target_and_dependency_types_are_rejected() {
    // Arrange
    let invalid_auto = TempFixture::new("invalid-auto");
    invalid_auto.write_manifest("autobins = \"false\"\n");
    invalid_auto.write("src/bin/hidden.rs", "fn main() {}\n");
    let invalid_required = TempFixture::new("invalid-required");
    invalid_required.write_manifest(
        "[[bin]]\nname = \"tool\"\npath = \"src/main.rs\"\nrequired-features = \"cli\"\n",
    );
    let invalid_optional = TempFixture::new("invalid-optional");
    invalid_optional.write_manifest(
        "[dependencies]\noptional-crate = { version = \"1\", optional = \"true\" }\n",
    );

    // Act
    let auto = run_check_with(&invalid_auto, "bin:hidden", "json", &[]);
    let required = run_check_with(&invalid_required, "bin:tool", "json", &[]);
    let optional = run_check(&invalid_optional, "json");

    // Assert
    assert_execution_failure(&auto, "must be a boolean");
    assert_execution_failure(&required, "must be an array");
    assert_execution_failure(&optional, "optional must be a boolean");
}

#[test]
fn should_malformed_manifest_shapes_fail_before_analysis() {
    // Arrange
    let cases = [
        (
            "dependency-value",
            "[dependencies]\nfoo = 1\n",
            "version string or table",
        ),
        (
            "dependency-table",
            "dependencies = []\n",
            "dependencies must be a table",
        ),
        ("target-table", "target = []\n", "target must be a table"),
        ("lib-table", "lib = []\n", "lib target must be a table"),
        ("lib-path", "[lib]\npath = 1\n", "lib.path must be text"),
        (
            "bin-array",
            "bin = { name = \"tool\" }\n",
            "bin targets must be an array",
        ),
        ("bin-table", "bin = [1]\n", "bin target must be a table"),
        (
            "bin-name",
            "bin = [{ path = \"src/main.rs\" }]\n",
            "target.name must be text",
        ),
        (
            "bin-path",
            "bin = [{ name = \"tool\", path = 1 }]\n",
            "target.path must be text",
        ),
        (
            "feature-entry",
            "bin = [{ name = \"tool\", required-features = [1] }]\n",
            "entries must be text",
        ),
    ];

    // Act
    let outputs = cases
        .into_iter()
        .map(|(label, fragment, expected)| {
            let fixture = TempFixture::new(label);
            fixture.write(
                "Cargo.toml",
                &format!(
                    "{fragment}[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
                    fixture.package()
                ),
            );
            (run_check(&fixture, "json"), expected)
        })
        .collect::<Vec<_>>();

    // Assert
    for (output, expected) in outputs {
        assert_execution_failure(&output, expected);
    }
}

#[test]
fn should_missing_required_analysis_context_is_rejected() {
    // Arrange
    let fixture = TempFixture::new("missing-context");
    let package = fixture.package().to_owned();
    let target = fixture.target();
    let target_triple = "x86_64-unknown-linux-gnu".to_owned();
    let cases = [
        (String::new(), target.clone(), target_triple.clone()),
        (package.clone(), String::new(), target_triple.clone()),
        (package, target, String::new()),
    ];

    // Act
    let outputs = cases
        .into_iter()
        .map(|(package, target, target_triple)| {
            run_custom(&fixture, &package, &target, &target_triple, &[])
        })
        .collect::<Vec<_>>();

    // Assert
    for output in outputs {
        assert_execution_failure(&output, "package, target, and target_triple are required");
    }
}
