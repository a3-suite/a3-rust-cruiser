// contract_id: contract.logical-module-boundary.validation-report
// integration_id: logical-module-boundary-validation-report

use super::*;

#[test]
fn complete_graph_without_violations_is_conformant() {
    // Arrange
    let fixture = TempFixture::new("complete-conformant");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
}

#[test]
fn complete_graph_with_violations_is_non_conformant() {
    // Arrange
    let fixture = forbidden_fixture("complete-non-conformant", false);

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 1);

    // Assert
    assert_eq!(report["decision"], "non-conformant");
}

#[test]
fn incomplete_analysis_is_indeterminate_even_with_violations() {
    // Arrange
    let fixture = forbidden_fixture("incomplete-with-violation", true);

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_eq!(report["decision"], "indeterminate");
    assert!(
        !report["violations"]
            .as_array()
            .expect("violations array")
            .is_empty()
    );
    assert!(
        !report["analysis_issues"]
            .as_array()
            .expect("issues array")
            .is_empty()
    );
}

#[test]
fn json_report_projects_common_validation_result() {
    // Arrange
    let fixture = forbidden_fixture("json-report", true);

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_eq!(report["schema_version"], "1");
    assert_eq!(report["serialization"], "json");
    assert_eq!(report["ordering"], "canonical");
    assert_eq!(
        report["analysis_context"]["analysis_profile"],
        "compilation"
    );
    assert_eq!(report["decision"], "indeterminate");
    assert!(
        !report["violations"]
            .as_array()
            .expect("violations array")
            .is_empty()
    );
    let issue = &report["analysis_issues"][0];
    assert!(
        issue["code"]
            .as_str()
            .is_some_and(|code| code.starts_with("analysis."))
    );
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
}

#[test]
fn text_report_projects_common_validation_result() {
    // Arrange
    let fixture = forbidden_fixture("text-report", true);

    // Act
    let output = run_check(&fixture, "text");
    let payload = completed_text(&output, 2);

    // Assert
    assert!(payload.contains("schema_version: 1"));
    assert!(payload.contains("serialization: text"));
    assert!(payload.contains("ordering: canonical"));
    assert!(payload.contains("analysis_profile: compilation"));
    assert!(payload.contains("decision: indeterminate"));
    assert!(payload.contains("violations:\n  - forbidden-application forbidden"));
    assert!(payload.contains("analysis_issues:\n  - analysis."));
}

#[test]
fn unresolved_reference_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unresolved-reference");
    fixture.write(
        "src/main.rs",
        "mod domain;\nfn consume(_: crate::domain::Missing) {}\nfn main() {}\n",
    );
    fixture.write("src/domain.rs", "pub struct Present;\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unresolved-reference");
}

#[test]
fn unresolved_single_segment_type_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unresolved-single-segment");
    fixture.write("src/main.rs", "fn consume(_: Missing) {}\nfn main() {}\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
}

#[test]
fn block_scoped_use_is_indeterminate_until_scope_resolution_is_supported() {
    // Arrange
    let fixture = TempFixture::new("block-scoped-use");
    fixture.write(
        "src/main.rs",
        "mod forbidden;\nfn main() { use crate::forbidden::run as invoke; invoke(); }\n",
    );
    fixture.write("src/forbidden.rs", "pub fn run() {}\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
}

#[test]
fn repeated_json_report_is_byte_stable() {
    // Arrange
    let fixture = forbidden_fixture("stable-json", true);

    // Act
    let first = run_check(&fixture, "json");
    let second = run_check(&fixture, "json");

    // Assert
    assert_eq!(first.status.code(), Some(2));
    assert_eq!(second.status.code(), Some(2));
    assert_eq!(first.stdout, second.stdout);
    assert!(first.stderr.is_empty());
    assert!(second.stderr.is_empty());
}

#[test]
fn unsupported_macro_expansion_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unsupported-macro");
    fixture.write("src/main.rs", "fn main() { println!(\"hello\"); }\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-macro-expansion");
}

#[test]
fn source_union_keeps_unresolved_internal_references_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("source-union-unresolved-internal");
    fixture.write(
        "src/main.rs",
        "fn main() { let _ = crate::missing::Thing; }\n",
    );

    // Act
    let report = completed_json(
        &run_check_with(
            &fixture,
            "bin",
            "json",
            &["--analysis-profile", "source-union"],
        ),
        2,
    );

    // Assert
    assert_eq!(
        report["analysis_context"]["analysis_profile"],
        "source-union"
    );
    assert_issue(&report, "analysis.unresolved-reference");
}

#[test]
fn unsupported_derive_macro_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unsupported-derive");
    fixture.write(
        "src/main.rs",
        "#[derive(Debug)]\nstruct Value;\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-macro-expansion");
}

#[test]
fn unsupported_name_resolution_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unsupported-name-resolution");
    fixture.write(
        "src/main.rs",
        "use missing_crate::Thing;\nfn main() { let _ = core::mem::size_of::<Thing>(); }\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
}

#[test]
fn unsupported_expression_name_resolution_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unsupported-expression-resolution");
    fixture.write(
        "src/main.rs",
        "fn main() { let _ = missing_module::Thing; }\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
}

#[test]
fn unsupported_syntax_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unsupported-syntax");
    fixture.write("src/main.rs", "fn main( {\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-syntax");
}

#[test]
fn unsupported_analysis_context_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unsupported-context");
    fixture.write(
        "src/main.rs",
        "#[cfg(feature = \"optional\")]\nmod gated { pub fn run() {} }\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-analysis-context");
}

#[test]
fn missing_target_required_feature_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("missing-target-feature");
    fixture.write_manifest(
        "[[bin]]\nname = \"tool\"\npath = \"src/bin/tool.rs\"\nrequired-features = [\"cli\"]\n",
    );
    fixture.write("src/bin/tool.rs", "fn main() {}\n");

    // Act
    let report = completed_json(&run_check_with(&fixture, "bin:tool", "json", &[]), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-analysis-context");
}

#[test]
fn dev_dependency_is_unavailable_to_binary_target() {
    // Arrange
    let fixture = TempFixture::new("binary-target-dependencies");
    fixture.write_manifest("[dev-dependencies]\ntest-only = \"1\"\n[target.x86_64-unknown-linux-gnu.dependencies]\ntarget-only = \"1\"\n");
    fixture.write("src/main.rs", "use test_only;\nfn main() {}\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
}

#[test]
fn cfg_selected_dependency_is_indeterminate_without_cfg_evaluation() {
    // Arrange
    let fixture = TempFixture::new("cfg-dependency");
    fixture.write_manifest("[target.'cfg(unix)'.dependencies]\ncfg-only = \"1\"\n");
    fixture.write("src/main.rs", "use cfg_only;\nfn main() {}\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-analysis-context");
}

#[test]
fn optional_dependency_is_indeterminate_without_feature_evaluation() {
    // Arrange
    let fixture = TempFixture::new("optional-dependency");
    fixture
        .write_manifest("[dependencies]\noptional-crate = { version = \"1\", optional = true }\n");
    fixture.write("src/main.rs", "use optional_crate;\nfn main() {}\n");

    // Act
    let report = completed_json(
        &run_check_with(&fixture, "bin", "json", &["--feature", "optional-crate"]),
        2,
    );

    // Assert
    assert_issue(&report, "analysis.unsupported-analysis-context");
}
