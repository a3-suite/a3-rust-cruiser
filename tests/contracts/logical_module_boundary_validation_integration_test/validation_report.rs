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
fn confirmed_violations_are_non_conformant_even_with_possible_issues() {
    // Arrange
    let fixture = forbidden_fixture("incomplete-with-violation", true);

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 1);

    // Assert
    assert_eq!(report["decision"], "non-conformant");
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
    assert!(report["analysis_issues"].as_array().is_some_and(|issues| {
        issues
            .iter()
            .any(|issue| issue["policy_impact"]["outcome"] == "possible")
    }));
}

#[test]
fn ambiguous_component_attribution_retains_safe_violations_as_non_conformant() {
    // Arrange
    let fixture = many_to_one_sdd_fixture();

    // Act
    let report = completed_json(&run_sdd_check(&fixture, true), 1);

    // Assert
    assert_eq!(report["decision"], "non-conformant");
    assert_eq!(report["ordering"], "canonical");
    assert_issue(&report, "analysis.ambiguous-component-attribution");
    assert_eq!(
        report["analysis_issues"][0]["affected_scope"],
        "crate::shared"
    );
    assert_eq!(
        report["analysis_issues"][0]["reason"],
        "Rust module crate::shared represents multiple logical components: application, domain"
    );
    let violations = report["violations"]
        .as_array()
        .expect("violations should be an array");
    assert!(violations.iter().any(|violation| {
        violation["source"] == "crate::reporting" && violation["target"] == "crate::infrastructure"
    }));
    assert!(violations.iter().all(|violation| {
        violation["source"] != "crate::shared" && violation["target"] != "crate::shared"
    }));
}

#[test]
fn invariant_allowed_component_assignments_are_conformant() {
    // Arrange
    let fixture = many_to_one_sdd_fixture();
    fixture.write(
        "logical-structure.sdd.yml",
        "version: 1\nname: logical-structure\nstructure_id: fixture\ncomponents:\n  - component_id: domain\n  - component_id: application\n  - component_id: infrastructure\n  - component_id: reporting\n  - component_id: ci-publish-script-adapter\ndependencies:\n  - from_component_id: application\n    to_component_id: infrastructure\n  - from_component_id: domain\n    to_component_id: infrastructure\n  - from_component_id: reporting\n    to_component_id: infrastructure\n",
    );

    // Act
    let report = completed_json(&run_sdd_check(&fixture, true), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(
        report["analysis_issues"][0]["policy_impact"]["outcome"],
        "none"
    );
    assert_eq!(report["violations"], serde_json::json!([]));
}

#[test]
fn invariant_forbidden_component_assignments_are_non_conformant() {
    // Arrange
    let fixture = many_to_one_sdd_fixture();
    fixture.write(
        "logical-structure.sdd.yml",
        "version: 1\nname: logical-structure\nstructure_id: fixture\ncomponents:\n  - component_id: domain\n  - component_id: application\n  - component_id: infrastructure\n  - component_id: reporting\n  - component_id: ci-publish-script-adapter\ndependencies: []\n",
    );

    // Act
    let report = completed_json(&run_sdd_check(&fixture, true), 1);

    // Assert
    assert_eq!(report["decision"], "non-conformant");
    assert_eq!(
        report["analysis_issues"][0]["policy_impact"]["outcome"],
        "none"
    );
    assert!(report["violations"].as_array().is_some_and(|violations| {
        violations.iter().any(|violation| {
            violation["source"] == "crate::shared" && violation["target"] == "crate::infrastructure"
        })
    }));
}

#[test]
fn assignment_dependent_component_outcomes_remain_indeterminate_without_safe_violations() {
    // Arrange
    let fixture = many_to_one_sdd_fixture();
    fixture.write("src/reporting.rs", "pub fn report() {}\n");

    // Act
    let report = completed_json(&run_sdd_check(&fixture, true), 2);

    // Assert
    assert_eq!(report["decision"], "indeterminate");
    assert_eq!(
        report["analysis_issues"][0]["policy_impact"]["outcome"],
        "possible"
    );
    assert_eq!(report["violations"], serde_json::json!([]));
}

#[test]
fn component_assignment_proof_avoids_exhaustive_enumeration() {
    // Arrange
    let fixture = TempFixture::new("component-assignment-proof-exhaustion");
    let mut main_source = String::new();
    let mut logical =
        "version: 1\nname: logical-structure\nstructure_id: fixture\ncomponents:\n".to_owned();
    let mut projection =
        "version: 1\nname: projection\nprojection_id: fixture-source\nmappings:\n".to_owned();
    let mut directories =
        "version: 1\nname: directory-structure\nstructure_id: fixture-source\nnodes:\n".to_owned();
    for index in 0..13 {
        main_source.push_str(&format!("mod scope{index};\n"));
        fixture.write(&format!("src/scope{index}.rs"), "pub fn run() {}\n");
        for side in ["left", "right"] {
            logical.push_str(&format!("  - component_id: {side}-{index}\n"));
            projection.push_str(&format!(
                "  - component_id: {side}-{index}\n    node_id: scope-{index}\n"
            ));
        }
        directories.push_str(&format!(
            "  - node_id: scope-{index}\n    path: src/scope{index}.rs\n"
        ));
    }
    main_source.push_str("fn main() {}\n");
    logical.push_str("dependencies: []\n");
    fixture.write("src/main.rs", &main_source);
    fixture.write("logical-structure.sdd.yml", &logical);
    fixture.write("projection.sdd.yml", &projection);
    fixture.write("directory-structure.sdd.yml", &directories);

    // Act
    let report = completed_json(&run_sdd_check(&fixture, true), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert!(report["analysis_issues"].as_array().is_some_and(|issues| {
        issues
            .iter()
            .all(|issue| issue["policy_impact"]["outcome"] == "none")
    }));
}

#[test]
fn json_report_projects_common_validation_result() {
    // Arrange
    let fixture = forbidden_fixture("json-report", true);

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 1);

    // Assert
    assert_eq!(report["schema_version"], "2");
    assert_eq!(report["serialization"], "json");
    assert_eq!(report["ordering"], "canonical");
    assert_eq!(
        report["analysis_context"]["analysis_profile"],
        "compilation"
    );
    assert_eq!(report["decision"], "non-conformant");
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
    assert_eq!(
        issue["observation"]["outcome"],
        "unresolved-dependency-candidate"
    );
    assert_eq!(issue["policy_impact"]["outcome"], "possible");
}

#[test]
fn text_report_projects_common_validation_result() {
    // Arrange
    let fixture = forbidden_fixture("text-report", true);

    // Act
    let output = run_check(&fixture, "text");
    let payload = completed_text(&output, 1);

    // Assert
    assert!(payload.contains("schema_version: 2"));
    assert!(payload.contains("serialization: text"));
    assert!(payload.contains("ordering: canonical"));
    assert!(payload.contains("analysis_profile: compilation"));
    assert!(payload.contains("decision: non-conformant"));
    assert!(payload.contains("violations:\n  - forbidden-application forbidden"));
    assert!(payload.contains("analysis_issues:\n  - analysis."));
    assert!(payload.contains("observation: unresolved-dependency-candidate"));
    assert!(payload.contains("policy_impact: possible [possible-crate-cycle]"));
}

#[test]
fn unresolved_candidate_outside_the_policy_is_conformant() {
    // Arrange
    let fixture = TempFixture::new("unresolved-policy-none");
    fixture.write(
        "src/main.rs",
        "mod domain;\nfn consume(_: crate::domain::Missing) {}\nfn main() {}\n",
    );
    fixture.write("src/domain.rs", "pub struct Present;\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbidden-infrastructure\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: ['crate::infrastructure']\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_issue(&report, "analysis.unresolved-reference");
    let issue = &report["analysis_issues"][0];
    assert_eq!(
        issue["observation"]["outcome"],
        "unresolved-dependency-candidate"
    );
    assert_eq!(issue["policy_impact"]["outcome"], "none");
    assert_eq!(issue["policy_impact"]["rule_ids"], serde_json::json!([]));
    assert_eq!(
        issue["observation"]["candidate_targets"],
        serde_json::json!(["crate::domain"])
    );
}

#[test]
fn unresolved_reference_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("unresolved-reference");
    fixture.write(
        "src/main.rs",
        "mod domain;\nfn consume(_: crate::domain::Missing) {}\nfn main() {}\n",
    );
    fixture.write("src/domain.rs", "pub struct Present;\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unresolved-reference");
    let issue = &report["analysis_issues"][0];
    assert_eq!(issue["policy_impact"]["outcome"], "possible");
    assert_eq!(
        issue["policy_impact"]["rule_ids"],
        serde_json::json!(["possible-crate-cycle"])
    );
}

#[test]
fn unresolved_single_segment_type_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("unresolved-single-segment");
    fixture.write("src/main.rs", "fn consume(_: Missing) {}\nfn main() {}\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
}

#[test]
fn block_scoped_use_is_resolved_within_its_lexical_scope() {
    // Arrange
    let fixture = policy_sensitive_fixture("block-scoped-use");
    fixture.write(
        "src/main.rs",
        "mod forbidden;\nfn main() { use crate::forbidden::run as invoke; invoke(); }\n",
    );
    fixture.write("src/forbidden.rs", "pub fn run() {}\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn lexical_pattern_bindings_are_not_reported_as_unresolved() {
    // Arrange
    let fixture = policy_sensitive_fixture("lexical-pattern-bindings");
    fixture.write(
        "src/main.rs",
        "fn consume(values: &[usize]) -> usize {\n    let mapped = values.iter().map(|value| *value).next();\n    if let Some(selected) = mapped { selected } else { 0 }\n}\nfn main() { let _ = consume(&[1]); }\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn package_crate_alias_is_resolved_for_library_targets() {
    // Arrange
    let fixture = TempFixture::new("package-crate-alias");
    let alias = fixture.package().replace('-', "_");
    fixture.write(
        "src/lib.rs",
        &format!(
            "extern crate self as {alias};\npub mod api;\npub fn make() -> {alias}::api::Thing {{ {alias}::api::Thing }}\n"
        ),
    );
    fixture.write("src/api.rs", "pub struct Thing;\n");

    // Act
    let report = completed_json(&run_check_with(&fixture, "lib", "json", &[]), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
    assert!(dependency_exists(
        &report,
        "crate",
        "crate::api",
        "path-reference",
        "internal-module"
    ));
}

#[test]
fn undeclared_package_name_is_not_treated_as_a_library_alias() {
    // Arrange
    let fixture = policy_sensitive_fixture("undeclared-package-crate-alias");
    let alias = fixture.package().replace('-', "_");
    fixture.write(
        "src/lib.rs",
        &format!("pub mod api;\npub fn make() -> {alias}::api::Thing {{ {alias}::api::Thing }}\n"),
    );
    fixture.write("src/api.rs", "pub struct Thing;\n");

    // Act
    let report = completed_json(&run_check_with(&fixture, "lib", "json", &[]), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
    assert_eq!(report["decision"], "indeterminate");
}

#[test]
fn package_name_is_not_external_without_a_library_target() {
    // Arrange
    let fixture = policy_sensitive_fixture("package-name-without-library");
    let alias = fixture.package().replace('-', "_");
    fixture.write(
        "src/main.rs",
        &format!("fn main() {{ let _ = {alias}::Thing; }}\n"),
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
    assert_eq!(report["decision"], "indeterminate");
}

#[test]
fn known_dependency_observing_macros_and_safe_serde_attributes_are_conformant() {
    // Arrange
    let fixture = TempFixture::new("known-macros-and-serde");
    fixture.write_manifest(
        "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde-json = \"1\"\n",
    );
    fixture.write(
        "src/main.rs",
        "use serde::Deserialize;\n#[derive(Deserialize)]\n#[serde(default, rename = \"value\")]\nstruct Model { #[serde(default)] value: usize }\nfn main() {\n    let value = serde_json::json!({\"value\": crate::build_value()});\n    let _ = matches!(value, serde_json::Value::Object(_));\n}\nfn build_value() -> usize { 1 }\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn repeated_json_report_is_byte_stable() {
    // Arrange
    let fixture = forbidden_fixture("stable-json", true);

    // Act
    let first = run_check(&fixture, "json");
    let second = run_check(&fixture, "json");

    // Assert
    assert_eq!(first.status.code(), Some(1));
    assert_eq!(second.status.code(), Some(1));
    assert_eq!(first.stdout, second.stdout);
    assert!(first.stderr.is_empty());
    assert!(second.stderr.is_empty());
}

#[test]
fn dependency_neutral_standard_macro_is_conformant() {
    // Arrange
    let fixture = policy_sensitive_fixture("unsupported-macro");
    fixture.write("src/main.rs", "fn main() { println!(\"hello\"); }\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn shadowed_builtin_macro_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("shadowed-builtin-macro");
    fixture.write(
        "src/main.rs",
        "macro_rules! println { ($($arg:tt)*) => {}; }\nfn main() { println!(\"hello\"); }\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-macro-expansion");
}

#[test]
fn unsupported_macro_expansion_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("unsupported-macro");
    fixture.write("src/main.rs", "fn main() { custom_macro!(); }\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-macro-expansion");
}

#[test]
fn unsupported_derive_macro_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("unsupported-derive");
    fixture.write(
        "src/main.rs",
        "#[derive(Custom)]\nstruct Model;\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-macro-expansion");
}

#[test]
fn source_union_keeps_unresolved_internal_references_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("source-union-unresolved-internal");
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
fn dependency_neutral_builtin_derive_is_conformant() {
    // Arrange
    let fixture = policy_sensitive_fixture("unsupported-derive");
    fixture.write(
        "src/main.rs",
        "#[derive(Debug)]\nstruct Value;\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn external_derive_macros_with_proven_crate_identity_are_conformant() {
    // Arrange
    let fixture = policy_sensitive_fixture("external-derive");
    fixture
        .write_manifest("[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n");
    fixture.write(
        "src/main.rs",
        "use serde::Serialize;\nuse serde::Serialize as Renamed;\n#[derive(Renamed)]\nstruct RenamedValue;\n#[derive(Serialize, serde::Deserialize)]\nstruct Value;\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn unknown_external_derive_macro_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("unknown-external-derive");
    fixture.write_manifest("[dependencies]\nevil = \"1\"\n");
    fixture.write(
        "src/main.rs",
        "#[derive(evil::Serialize)]\nstruct Value;\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-macro-expansion");
}

#[test]
fn unknown_external_single_segment_derive_alias_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("unknown-external-derive-alias");
    fixture.write_manifest("[dependencies]\nevil = \"1\"\n");
    fixture.write(
        "src/main.rs",
        "use evil::Debug;\n#[derive(Debug)]\nstruct Value;\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-macro-expansion");
}

#[test]
fn mixed_external_derive_alias_candidates_are_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("mixed-external-derive-aliases");
    fixture.write_manifest(
        "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nevil = \"1\"\n",
    );
    fixture.write(
        "src/main.rs",
        "use serde::Serialize as Derive;\nuse evil::Serialize as Derive;\n#[derive(Derive)]\nstruct Value;\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-macro-expansion");
}

#[test]
fn unsupported_name_resolution_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("unsupported-name-resolution");
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
    let fixture = policy_sensitive_fixture("unsupported-expression-resolution");
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
fn generic_type_parameters_are_dependency_neutral() {
    // Arrange
    let fixture = policy_sensitive_fixture("generic-type-parameter");
    fixture.write(
        "src/main.rs",
        "fn identity<T>(value: T) -> T { value }\nstruct Wrapper;\nimpl Wrapper { fn map<T>(&self, value: T) -> T { value } }\ntrait Contract { fn convert<E>(&self, value: E) -> E; }\nfn main() { let _ = identity(1); }\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn generic_associated_paths_are_dependency_neutral() {
    // Arrange
    let fixture = policy_sensitive_fixture("generic-associated-path");
    fixture.write(
        "src/main.rs",
        "fn deserialize<D>(_: D) -> Result<(), D::Error> { todo!() }\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn no_implicit_prelude_keeps_prelude_names_unresolved() {
    // Arrange
    let fixture = policy_sensitive_fixture("no-implicit-prelude");
    fixture.write(
        "src/main.rs",
        "#![no_implicit_prelude]\nfn main() { let _: Option<()>; }\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
}

#[test]
fn internal_module_named_like_a_prelude_type_is_observed() {
    // Arrange
    let fixture = TempFixture::new("internal-prelude-name");
    fixture.write(
        "src/main.rs",
        "mod Option;\nfn main() { let _ = Option::value(); }\n",
    );
    fixture.write("src/Option.rs", "pub fn value() -> usize { 1 }\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbidden-prelude-name-module\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: [crate::Option]\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 1);

    // Assert
    assert_eq!(report["decision"], "non-conformant");
    assert!(dependency_target_exists(&report, "crate", "crate::Option"));
}

#[test]
fn renamed_extern_crate_is_resolved_as_external() {
    // Arrange
    let fixture = policy_sensitive_fixture("renamed-extern-crate");
    fixture.write(
        "src/main.rs",
        "extern crate std as runtime_std;\nfn main() { let _ = runtime_std::fmt::Error; }\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
    assert!(dependency_exists(
        &report,
        "crate",
        "runtime_std",
        "path-reference",
        "external-crate"
    ));
}

#[test]
fn unsupported_syntax_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("unsupported-syntax");
    fixture.write("src/main.rs", "fn main( {\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-syntax");
}

#[test]
fn path_module_declaration_is_indeterminate_for_child_policy() {
    // Arrange
    let fixture = TempFixture::new("path-module-child-policy");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-restricted-to-forbidden\n    kind: forbidden\n    severity: error\n    from: [crate::restricted]\n    to: [crate::forbidden]\n",
    );
    fixture.write(
        "src/main.rs",
        "#[path = \"restricted.rs\"]\nmod restricted;\nfn main() {}\n",
    );
    fixture.write("src/restricted.rs", "use crate::forbidden::Thing;\n");
    fixture.write("src/forbidden.rs", "pub struct Thing;\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    let issue = &report["analysis_issues"][0];
    assert_eq!(issue["code"], "analysis.unsupported-syntax");
    assert_eq!(issue["affected_scope"], "crate::restricted");
    assert_eq!(issue["observation"]["source_module"], "crate::restricted");
    assert_eq!(issue["policy_impact"]["outcome"], "possible");
    assert_eq!(
        issue["policy_impact"]["rule_ids"],
        serde_json::json!(["forbid-restricted-to-forbidden"])
    );
}

#[test]
fn active_cfg_attr_path_module_is_indeterminate_for_child_policy() {
    // Arrange
    let fixture = TempFixture::new("cfg-attr-path-module-child-policy");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-restricted-to-forbidden\n    kind: forbidden\n    severity: error\n    from: [crate::restricted]\n    to: [crate::forbidden]\n",
    );
    fixture.write(
        "src/main.rs",
        "#[cfg_attr(unix, path = \"alternate.rs\")]\nmod restricted;\nfn main() {}\n",
    );
    fixture.write("src/restricted.rs", "pub struct DefaultSource;\n");
    fixture.write("src/alternate.rs", "use crate::forbidden::Thing;\n");
    fixture.write("src/forbidden.rs", "pub struct Thing;\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    let issue = &report["analysis_issues"][0];
    assert_eq!(issue["code"], "analysis.unsupported-syntax");
    assert_eq!(issue["affected_scope"], "crate::restricted");
    assert_eq!(issue["observation"]["source_module"], "crate::restricted");
    assert_eq!(issue["policy_impact"]["outcome"], "possible");
    assert_eq!(
        issue["policy_impact"]["rule_ids"],
        serde_json::json!(["forbid-restricted-to-forbidden"])
    );
}

#[test]
fn inactive_cfg_attr_path_module_is_ignored() {
    // Arrange
    let fixture = TempFixture::new("inactive-cfg-attr-path-module");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-restricted-to-forbidden\n    kind: forbidden\n    severity: error\n    from: [crate::restricted]\n    to: [crate::forbidden]\n",
    );
    fixture.write(
        "src/main.rs",
        "#[cfg_attr(windows, path = \"alternate.rs\")]\nmod restricted;\nfn main() {}\n",
    );
    fixture.write("src/restricted.rs", "pub struct DefaultSource;\n");
    fixture.write("src/alternate.rs", "use crate::forbidden::Thing;\n");
    fixture.write("src/forbidden.rs", "pub struct Thing;\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn unsupported_analysis_context_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("unsupported-context");
    fixture.write(
        "src/main.rs",
        "#[cfg(custom_predicate(value))]\nmod gated { pub fn run() {} }\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-analysis-context");
}

#[test]
fn missing_target_required_feature_is_indeterminate() {
    // Arrange
    let fixture = policy_sensitive_fixture("missing-target-feature");
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
    let fixture = policy_sensitive_fixture("binary-target-dependencies");
    fixture.write_manifest("[dev-dependencies]\ntest-only = \"1\"\n[target.x86_64-unknown-linux-gnu.dependencies]\ntarget-only = \"1\"\n");
    fixture.write("src/main.rs", "use test_only;\nfn main() {}\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
}

#[test]
fn cfg_selected_dependency_uses_the_rustc_target_context() {
    // Arrange
    let fixture = policy_sensitive_fixture("cfg-dependency");
    fixture.write_manifest("[target.'cfg(unix)'.dependencies]\ncfg-only = \"1\"\n");
    fixture.write("src/main.rs", "use cfg_only;\nfn main() {}\n");

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn selected_optional_dependency_uses_the_cargo_feature_context() {
    // Arrange
    let fixture = policy_sensitive_fixture("optional-dependency");
    fixture
        .write_manifest("[dependencies]\noptional-crate = { version = \"1\", optional = true }\n");
    fixture.write("src/main.rs", "use optional_crate;\nfn main() {}\n");

    // Act
    let report = completed_json(
        &run_check_with(&fixture, "bin", "json", &["--feature", "optional-crate"]),
        0,
    );

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn inactive_cfg_items_are_excluded_before_dependency_observation() {
    // Arrange
    let fixture = policy_sensitive_fixture("inactive-cfg-item");
    fixture.write(
        "src/main.rs",
        "#[cfg(target_os = \"definitely-not-a-target\")]\nmod inactive { fn run() { let _ = crate::missing::Thing; } }\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn inactive_cfg_macro_statements_are_excluded_before_macro_observation() {
    // Arrange
    let fixture = policy_sensitive_fixture("inactive-cfg-macro-statement");
    fixture.write(
        "src/main.rs",
        "#[cfg(target_os = \"definitely-not-a-target\")]\ncustom_macro!();\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn inactive_cfg_fields_are_excluded_before_name_observation() {
    // Arrange
    let fixture = policy_sensitive_fixture("inactive-cfg-field");
    fixture.write(
        "src/main.rs",
        "struct Model { #[cfg(test)] value: MissingType }\nfn main() {}\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn standard_macro_arguments_are_observed_recursively() {
    // Arrange
    let fixture = TempFixture::new("macro-arguments");
    fixture.write(
        "src/main.rs",
        "mod forbidden;\nfn main() { println!(\"{}\", crate::forbidden::value()); }\n",
    );
    fixture.write("src/forbidden.rs", "pub fn value() -> usize { 1 }\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbidden-macro-argument\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: [crate::forbidden]\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 1);

    // Assert
    assert_eq!(report["decision"], "non-conformant");
    assert!(report["analysis_issues"].as_array().is_some_and(|issues| {
        issues
            .iter()
            .all(|issue| issue["policy_impact"]["outcome"] == "none")
    }));
}

#[test]
fn local_binding_shadowing_does_not_create_a_module_dependency() {
    // Arrange
    let fixture = TempFixture::new("local-binding-shadowing");
    fixture.write(
        "src/main.rs",
        "mod domain;\nfn main() { let domain = 1; let _ = domain; }\n",
    );
    fixture.write("src/domain.rs", "pub struct Entity;\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbidden-shadowed-name\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: [crate::domain]\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
    assert!(!dependency_target_exists(&report, "crate", "crate::domain"));
}
