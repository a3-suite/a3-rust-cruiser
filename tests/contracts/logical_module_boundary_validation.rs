use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::Value;

static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

// contract_id: contract.logical-module-boundary.dependency-graph
// integration_id: logical-module-boundary-dependency-graph
#[test]
fn typed_dependencies_and_declarations_are_reported() {
    // Arrange
    let fixture = TempFixture::new("typed-dependencies");
    fixture.write_manifest("[dependencies]\nserde-json = \"1\"\n");
    fixture.write(
        "src/main.rs",
        "use serde_json;\nmod application;\nmod domain;\nmod excluded;\nfn main() {}\n",
    );
    fixture.write(
        "src/application.rs",
        "use crate::domain::Thing;\npub use crate::domain::Thing as PublicThing;\npub fn run(_: crate::domain::Thing) -> Thing { Thing }\n",
    );
    fixture.write("src/domain.rs", "pub struct Thing;\n");
    fixture.write("src/excluded.rs", "pub struct Ignored;\n");

    // Act
    let output = run_check_with(&fixture, "bin", "json", &["--exclude", "crate::excluded"]);
    let report = completed_json(&output, 0);

    // Assert
    assert_eq!(
        report["analysis_context"]["analysis_profile"],
        "compilation"
    );
    assert_eq!(report["analysis_context"]["package"], fixture.package());
    assert_eq!(
        report["analysis_context"]["compilation_target"],
        fixture.target()
    );
    assert!(array_contains(
        &report["modules"],
        "id",
        "crate::application"
    ));
    assert!(array_contains(&report["modules"], "id", "crate::domain"));
    assert!(
        report["module_declarations"]
            .as_array()
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item["parent"] == "crate" && item["child"] == "crate::application")
            })
    );
    for kind in ["import", "re-export", "path-reference"] {
        assert!(dependency_exists(
            &report,
            "crate::application",
            "crate::domain",
            kind,
            "internal-module"
        ));
    }
    assert!(dependency_exists(
        &report,
        "crate",
        "serde_json",
        "import",
        "external-crate"
    ));
    assert_eq!(
        report["excluded_scopes"],
        serde_json::json!(["crate::excluded"])
    );
}

// contract_id: contract.logical-module-boundary.policy-evaluation
// integration_id: logical-module-boundary-policy-evaluation
#[test]
fn forbidden_rule_overrides_matching_allowed_rule() {
    // Arrange
    let fixture = dependency_fixture("forbidden-precedence", "infrastructure");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbidden-first\n    kind: forbidden\n    severity: error\n    from: [crate::application]\n    to: [crate::infrastructure]\n    dependency_kinds: [import]\n  - id: forbidden-second\n    kind: forbidden\n    severity: warning\n    from: [crate::application]\n    to: [crate::infrastructure]\n    dependency_kinds: [import]\n  - id: allowed-same\n    kind: allowed\n    severity: warning\n    from: [crate::application]\n    to: [crate::infrastructure]\n    dependency_kinds: [import]\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 1);
    let violations = report["violations"].as_array().expect("violations array");

    // Assert
    assert_eq!(violations.len(), 2);
    assert_eq!(violations[0]["kind"], "forbidden");
    assert_eq!(violations[0]["rule_id"], "forbidden-first");
    assert_eq!(violations[0]["severity"], "error");
    assert_eq!(violations[0]["source"], "crate::application");
    assert_eq!(violations[0]["target"], "crate::infrastructure");
    assert_eq!(violations[0]["dependency_kind"], "import");
    assert_eq!(violations[0]["evidence"]["path"], "src/application.rs");
    assert!(
        violations[0]["evidence"]["line"]
            .as_u64()
            .is_some_and(|line| line > 0)
    );
    assert!(
        violations[0]["evidence"]["column"]
            .as_u64()
            .is_some_and(|column| column > 0)
    );
    assert_eq!(
        report["policy_evaluation"]["dependency_outcome"],
        "forbidden"
    );
    assert_eq!(
        report["policy_evaluation"]["forbidden_match_handling"],
        "all-matches"
    );
}

#[test]
fn applicable_allowed_rules_use_target_union() {
    // Arrange
    let fixture = dependency_fixture("allowed-union", "domain");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: allowed-shared\n    kind: allowed\n    severity: warning\n    from: [crate::application]\n    to: [crate::shared]\n  - id: allowed-domain\n    kind: allowed\n    severity: warning\n    from: [crate::application]\n    to: [crate::domain]\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["violations"], serde_json::json!([]));
    assert_eq!(report["policy_evaluation"]["dependency_outcome"], "allowed");
    assert_eq!(
        report["policy_evaluation"]["allowed_match_handling"],
        "union"
    );
}

#[test]
fn unmatched_applicable_allowed_rules_produce_violation() {
    // Arrange
    let fixture = dependency_fixture("allowed-rejection", "infrastructure");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: allowed-domain\n    kind: allowed\n    severity: error\n    from: [crate::application]\n    to: [crate::domain]\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 1);
    let violation = &report["violations"][0];

    // Assert
    assert_eq!(violation["kind"], "not-allowed");
    assert_eq!(violation["rule_id"], "allowed-domain");
    assert_eq!(violation["severity"], "error");
    assert_eq!(violation["source"], "crate::application");
    assert_eq!(violation["target"], "crate::infrastructure");
    assert_eq!(violation["dependency_kind"], "import");
    assert_eq!(violation["evidence"]["path"], "src/application.rs");
    assert_eq!(
        report["policy_evaluation"]["allowed_match_handling"],
        "union"
    );
}

#[test]
fn circular_rule_is_independent_from_allowed_rules() {
    // Arrange
    let fixture = TempFixture::new("circular-independent");
    fixture.write(
        "src/main.rs",
        "mod application;\nmod domain;\nfn main() {}\n",
    );
    fixture.write(
        "src/application.rs",
        "use crate::domain;\npub fn run() { let _ = domain::run; }\n",
    );
    fixture.write(
        "src/domain.rs",
        "use crate::application;\npub fn run() { let _ = application::run; }\n",
    );
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: allowed-application\n    kind: allowed\n    severity: warning\n    from: [crate::application]\n    to: [crate::domain]\n  - id: allowed-domain\n    kind: allowed\n    severity: warning\n    from: [crate::domain]\n    to: [crate::application]\n  - id: no-cycle\n    kind: circular\n    severity: error\n    modules: [crate::application, crate::domain]\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 1);
    let violations = report["violations"].as_array().expect("violations array");

    // Assert
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0]["kind"], "circular");
    assert_eq!(violations[0]["rule_id"], "no-cycle");
    assert_eq!(violations[0]["severity"], "error");
    assert!(
        violations[0]["cycle_path"]
            .as_array()
            .is_some_and(|path| path.len() >= 3)
    );
    assert_eq!(report["policy_evaluation"]["cycle_outcome"], "circular");
    assert_eq!(report["policy_evaluation"]["cycle_handling"], "independent");
}

// contract_id: contract.logical-module-boundary.validation-report
// integration_id: logical-module-boundary-validation-report
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
fn aliases_and_multiple_super_segments_are_resolved() {
    // Arrange
    let fixture = TempFixture::new("aliases-and-super");
    fixture.write("src/main.rs", "mod outer;\nmod shared;\nfn main() {}\n");
    fixture.write("src/outer.rs", "pub mod inner;\npub mod sibling;\n");
    fixture.write(
        "src/outer/inner.rs",
        "use crate::outer::sibling::Choice as Alias;\npub fn choose(_: Alias) -> super::super::shared::Thing { super::super::shared::Thing }\n",
    );
    fixture.write("src/outer/sibling.rs", "pub enum Choice { One }\n");
    fixture.write("src/shared.rs", "pub struct Thing;\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert!(dependency_target_exists(
        &report,
        "crate::outer::inner",
        "crate::outer::sibling"
    ));
    assert!(dependency_target_exists(
        &report,
        "crate::outer::inner",
        "crate::shared"
    ));
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
fn source_union_observes_source_authored_dependencies_without_expansion_issues() {
    // Arrange
    let fixture = TempFixture::new("source-union");
    fixture.write(
        "src/main.rs",
        "mod left;\nmod right;\n#[cfg(feature = \"left\")]\nmod platform { use crate::left as selected; pub fn run() { let _ = selected::Thing; } }\n#[cfg(not(feature = \"left\"))]\nmod platform { use crate::right as selected; pub fn run() { let _ = selected::Thing; } }\nfn main() { println!(\"hello\"); }\n",
    );
    fixture.write("src/left.rs", "pub struct Thing;\n");
    fixture.write("src/right.rs", "pub struct Thing;\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-platform-right\n    kind: forbidden\n    severity: error\n    from: [crate::platform]\n    to: [crate::right]\n    dependency_kinds: [path-reference]\n",
    );

    // Act
    let report = completed_json(
        &run_check_with(
            &fixture,
            "bin",
            "json",
            &["--analysis-profile", "source-union"],
        ),
        1,
    );

    // Assert
    assert_eq!(
        report["analysis_context"]["analysis_profile"],
        "source-union"
    );
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
    assert_eq!(report["decision"], "non-conformant");
    assert!(dependency_exists(
        &report,
        "crate::platform",
        "crate::left",
        "path-reference",
        "internal-module"
    ));
    assert!(dependency_exists(
        &report,
        "crate::platform",
        "crate::right",
        "path-reference",
        "internal-module"
    ));
    assert_eq!(report["violations"][0]["rule_id"], "forbid-platform-right");
}

#[test]
fn source_union_unions_cfg_gated_alias_candidates() {
    // Arrange
    let fixture = TempFixture::new("source-union-cfg-aliases");
    fixture.write(
        "src/main.rs",
        "mod left;\nmod right;\n#[cfg(feature = \"left\")]\nuse crate::left as selected;\n#[cfg(not(feature = \"left\"))]\nuse crate::right as selected;\nfn main() { let _ = selected::Thing; }\n",
    );
    fixture.write("src/left.rs", "pub struct Thing;\n");
    fixture.write("src/right.rs", "pub struct Thing;\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-selected-right\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: [crate::right]\n    dependency_kinds: [path-reference]\n",
    );

    // Act
    let report = completed_json(
        &run_check_with(
            &fixture,
            "bin",
            "json",
            &["--analysis-profile", "source-union"],
        ),
        1,
    );

    // Assert
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
    assert!(dependency_exists(
        &report,
        "crate",
        "crate::left",
        "path-reference",
        "internal-module"
    ));
    assert!(dependency_exists(
        &report,
        "crate",
        "crate::right",
        "path-reference",
        "internal-module"
    ));
    assert_eq!(report["violations"][0]["rule_id"], "forbid-selected-right");
}

#[test]
fn source_union_includes_conditional_external_dependencies() {
    // Arrange
    let fixture = TempFixture::new("source-union-conditional-external");
    fixture.write_manifest("[dependencies]\noptional-dep = { version = \"1\", optional = true }\n");
    fixture.write(
        "src/main.rs",
        "#[cfg(feature = \"optional\")]\nuse optional_dep::Value;\nfn main() {}\n",
    );

    // Act
    let report = completed_json(
        &run_check_with(
            &fixture,
            "bin",
            "json",
            &["--analysis-profile", "source-union"],
        ),
        0,
    );

    // Assert
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
    assert_eq!(report["decision"], "conformant");
    assert!(dependency_exists(
        &report,
        "crate",
        "optional_dep",
        "import",
        "external-crate"
    ));
}

#[test]
fn source_union_preserves_conditional_dependency_kind_boundaries() {
    // Arrange
    let fixture = TempFixture::new("source-union-target-cfg-dependencies");
    fixture.write_manifest(
        "[target.'cfg(unix)'.dependencies]\ncfg-runtime = \"1\"\n[target.'cfg(unix)'.dev-dependencies]\ncfg-dev = \"1\"\n[target.'cfg(unix)'.build-dependencies]\ncfg-build = \"1\"\n",
    );
    let source = "use cfg_runtime;\nuse cfg_dev;\nuse cfg_build;\nfn run() {}\n";
    fixture.write("src/main.rs", source);
    fixture.write("tests/contract.rs", source);

    // Act
    let binary_report = completed_json(
        &run_check_with(
            &fixture,
            "bin",
            "json",
            &["--analysis-profile", "source-union"],
        ),
        2,
    );
    let test_report = completed_json(
        &run_check_with(
            &fixture,
            "test:contract",
            "json",
            &["--analysis-profile", "source-union"],
        ),
        2,
    );

    // Assert
    assert_eq!(
        binary_report["analysis_context"]["analysis_profile"],
        "source-union"
    );
    assert!(dependency_target_exists(
        &binary_report,
        "crate",
        "cfg_runtime"
    ));
    assert!(!dependency_target_exists(
        &binary_report,
        "crate",
        "cfg_dev"
    ));
    assert!(!dependency_target_exists(
        &binary_report,
        "crate",
        "cfg_build"
    ));
    assert_issue(&binary_report, "analysis.unsupported-analysis-context");

    assert!(dependency_target_exists(
        &test_report,
        "crate",
        "cfg_runtime"
    ));
    assert!(dependency_target_exists(&test_report, "crate", "cfg_dev"));
    assert!(!dependency_target_exists(
        &test_report,
        "crate",
        "cfg_build"
    ));
    assert_issue(&test_report, "analysis.unsupported-analysis-context");
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

// contract_id: contract.logical-module-boundary.cli-outcome
// integration_id: logical-module-boundary-cli-outcome
#[test]
fn conformant_result_exits_with_zero() {
    // Arrange
    let fixture = TempFixture::new("conformant-exit");

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_completed_output(&output, 0, "conformant");
}

#[test]
fn non_conformant_result_exits_with_one() {
    // Arrange
    let fixture = forbidden_fixture("non-conformant-exit", false);

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_completed_output(&output, 1, "non-conformant");
}

#[test]
fn indeterminate_result_exits_with_two() {
    // Arrange
    let fixture = TempFixture::new("indeterminate-exit");
    fixture.write("src/main.rs", "fn main() { println!(\"hello\"); }\n");

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_completed_output(&output, 2, "indeterminate");
}

#[test]
fn execution_failure_exits_with_three() {
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

#[test]
fn supported_compilation_targets_select_their_entry_files() {
    // Arrange
    let fixture = TempFixture::new("explicit-targets");
    fixture.write_manifest(
        "[lib]\npath = \"sources/library.rs\"\n[[bin]]\nname = \"tool\"\npath = \"sources/tool.rs\"\n[[example]]\nname = \"demo\"\npath = \"sources/demo.rs\"\n[[test]]\nname = \"contract\"\npath = \"sources/contract.rs\"\n[[bench]]\nname = \"performance\"\npath = \"sources/performance.rs\"\n",
    );
    for path in [
        "sources/library.rs",
        "sources/tool.rs",
        "sources/demo.rs",
        "sources/contract.rs",
        "sources/performance.rs",
    ] {
        fixture.write(path, "pub fn entry() {}\n");
    }
    let targets = [
        "lib",
        "bin:tool",
        "example:demo",
        "test:contract",
        "bench:performance",
    ];

    // Act
    let reports: Vec<_> = targets
        .iter()
        .map(|target| completed_json(&run_check_with(&fixture, target, "json", &[]), 0))
        .collect();

    // Assert
    for (target, report) in targets.iter().zip(reports) {
        assert_eq!(report["analysis_context"]["compilation_target"], *target);
        assert_eq!(report["decision"], "conformant");
    }
}

#[test]
fn conventional_compilation_target_paths_are_supported() {
    // Arrange
    let fixture = TempFixture::new("conventional-targets");
    fixture.write("src/lib.rs", "pub fn library() {}\n");
    fixture.write("src/bin/tool.rs", "fn main() {}\n");
    fixture.write("src/bin/nested/main.rs", "fn main() {}\n");
    fixture.write("examples/demo.rs", "fn main() {}\n");
    fixture.write("examples/nested/main.rs", "fn main() {}\n");
    fixture.write("tests/contract.rs", "fn contract() {}\n");
    fixture.write("benches/performance.rs", "fn benchmark() {}\n");
    let targets = [
        "lib",
        "bin:fixture-conventional-targets",
        "bin:tool",
        "bin:nested",
        "example:demo",
        "example:nested",
        "test:contract",
        "bench:performance",
    ];

    // Act
    let outputs: Vec<_> = targets
        .iter()
        .map(|target| run_check_with(&fixture, target, "json", &[]))
        .collect();

    // Assert
    assert!(outputs.iter().all(|output| output.status.code() == Some(0)));
}

#[test]
fn target_feature_requirements_and_auto_discovery_are_fail_closed() {
    // Arrange
    let fixture = TempFixture::new("target-selection-context");
    fixture.write_manifest("autobins = false\n[[bin]]\nname = \"tool\"\npath = \"src/bin/tool.rs\"\nrequired-features = [\"cli\"]\n[[bin]]\nname = \"missing\"\npath = \"src/bin/missing.rs\"\n");
    fixture.write("src/bin/tool.rs", "fn main() {}\n");
    fixture.write("src/bin/hidden.rs", "fn main() {}\n");

    // Act
    let missing_feature = run_check_with(&fixture, "bin:tool", "json", &[]);
    let active_feature = run_check_with(&fixture, "bin:tool", "json", &["--feature", "cli"]);
    let hidden = run_check_with(&fixture, "bin:hidden", "json", &[]);
    let missing = run_check_with(&fixture, "bin:missing", "json", &[]);

    // Assert
    assert_issue(
        &completed_json(&missing_feature, 2),
        "analysis.unsupported-analysis-context",
    );
    assert_eq!(active_feature.status.code(), Some(0));
    assert_execution_failure(&hidden, "bin target not found");
    assert_execution_failure(&missing, "target source file not found");
}

#[test]
fn inline_nested_and_declared_external_dependencies_are_observed() {
    // Arrange
    let fixture = TempFixture::new("module-shapes");
    fixture.write_manifest("[dependencies]\nserde-json = \"1\"\n");
    fixture.write("src/main.rs", "mod outer;\nuse serde_json;\nfn main() {}\n");
    fixture.write("src/outer.rs", "pub mod child;\npub mod inline { pub mod nested; use super::child::Thing as Alias; pub fn make() -> Alias { super::child::Thing } }\n");
    fixture.write("src/outer/child.rs", "pub struct Thing;\n");
    fixture.write("src/outer/inline/nested.rs", "pub struct Nested;\n");

    // Act
    let report = completed_json(
        &run_check_with(&fixture, "bin", "json", &["--include", "crate::**"]),
        0,
    );

    // Assert
    for module in [
        "crate",
        "crate::outer::inline",
        "crate::outer::child",
        "crate::outer::inline::nested",
    ] {
        assert!(array_contains(&report["modules"], "id", module));
    }
    assert!(dependency_exists(
        &report,
        "crate",
        "serde_json",
        "import",
        "external-crate"
    ));
    assert!(dependency_target_exists(
        &report,
        "crate::outer::inline",
        "crate::outer::child"
    ));
}

#[test]
fn leading_colon_selects_the_external_crate_namespace() {
    // Arrange
    let fixture = TempFixture::new("absolute-external");
    fixture.write_manifest("[dependencies]\nserde = \"1\"\n");
    fixture.write(
        "src/main.rs",
        "mod serde;\nuse ::serde::Serialize;\nfn main() {}\n",
    );
    fixture.write("src/serde.rs", "pub struct Local;\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert!(dependency_exists(
        &report,
        "crate",
        "serde",
        "import",
        "external-crate"
    ));
    assert!(!dependency_target_exists(&report, "crate", "crate::serde"));
}

#[test]
fn declared_item_kinds_support_complete_path_resolution() {
    // Arrange
    let fixture = TempFixture::new("declared-items");
    fixture.write(
        "src/main.rs",
        "mod domain;\nfn consume(_: crate::domain::Model) {}\nfn main() {}\n",
    );
    fixture.write(
        "src/domain.rs",
        "pub const VALUE: usize = 1;\npub enum Choice { One }\nextern crate core as core_alias;\npub fn make() {}\nmacro_rules! named { () => {} }\npub mod nested {}\npub static STATE: usize = 1;\npub struct Model;\npub trait Contract {}\npub trait Alias = Contract;\npub type Identifier = usize;\npub union Number { value: usize }\nuse core::fmt::{self, Debug as DebugAlias, *};\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert!(dependency_exists(
        &report,
        "crate",
        "crate::domain",
        "path-reference",
        "internal-module"
    ));
}

#[test]
fn external_dependencies_follow_the_selected_compilation_target() {
    // Arrange
    let fixture = TempFixture::new("target-dependencies");
    fixture.write_manifest("[dev-dependencies]\ntest-only = \"1\"\n[target.x86_64-unknown-linux-gnu.dependencies]\ntarget-only = \"1\"\n");
    fixture.write("src/main.rs", "use test_only;\nfn main() {}\n");
    fixture.write("tests/contract.rs", "use test_only;\nfn contract() {}\n");

    // Act
    let binary_report = completed_json(&run_check(&fixture, "json"), 2);
    let test_report = completed_json(&run_check_with(&fixture, "test:contract", "json", &[]), 0);

    // Assert
    assert_issue(&binary_report, "analysis.unsupported-name-resolution");
    assert!(dependency_exists(
        &test_report,
        "crate",
        "test_only",
        "import",
        "external-crate"
    ));
}

#[test]
fn exact_target_dependency_is_active_for_the_selected_triple() {
    // Arrange
    let fixture = TempFixture::new("exact-target-dependency");
    fixture.write_manifest("[target.x86_64-unknown-linux-gnu.dependencies]\ntarget-only = \"1\"\n");
    fixture.write("src/main.rs", "use target_only;\nfn main() {}\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert!(dependency_exists(
        &report,
        "crate",
        "target_only",
        "import",
        "external-crate"
    ));
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

#[test]
fn invalid_manifest_and_target_requests_fail_with_diagnostics() {
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
fn manifest_target_and_dependency_types_are_rejected() {
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
fn malformed_manifest_shapes_fail_before_analysis() {
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

    // Act / Assert
    for (label, fragment, expected) in cases {
        let fixture = TempFixture::new(label);
        fixture.write(
            "Cargo.toml",
            &format!(
                "{fragment}[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
                fixture.package()
            ),
        );
        assert_execution_failure(&run_check(&fixture, "json"), expected);
    }
}

#[test]
fn missing_required_analysis_context_is_rejected() {
    // Arrange
    let fixture = TempFixture::new("missing-context");

    // Act
    let output = run_custom(&fixture, fixture.package(), &fixture.target(), "", &[]);

    // Assert
    assert_execution_failure(&output, "package, target, and target_triple are required");
}

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
