// contract_id: contract.logical-module-boundary.boundary-policy-input
// integration_id: logical-module-boundary-policy-input

use super::*;

#[test]
fn should_resolved_attribution_is_exclusive_with_other_policy_sources() {
    // Arrange
    let fixture = TempFixture::new("resolved-attribution-exclusive-input");
    write_resolved_implementation_attribution(&fixture);

    // Act
    let output = run_resolved_attribution_check(&fixture, &["--policy", "policy.yaml"]);

    // Assert
    assert_execution_failure(&output, "boundary policy inputs are mutually exclusive");
}

#[test]
fn should_public_resolved_implementation_attribution_is_accepted() {
    // Arrange
    let fixture = TempFixture::new("resolved-attribution-public-input");
    write_resolved_implementation_attribution(&fixture);

    // Act
    let report = completed_json(
        &run_resolved_attribution_check(&fixture, &["--include", "crate"]),
        0,
    );

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(
        report["normalized_sdd_coverage"]["source_provenance"]["producer"],
        "owner-resolved-current-family"
    );
    assert_eq!(
        report["normalized_sdd_coverage"]["covered_modules"],
        serde_json::json!(["crate"])
    );
    assert_eq!(
        report["analysis_context"]["include"],
        serde_json::json!(["crate"])
    );
}

#[test]
fn should_public_resolved_attribution_accept_multiple_scopes_and_artifacts() {
    // Arrange
    let fixture = TempFixture::new("resolved-attribution-multiple-scopes");
    fixture.write(
        "src/main.rs",
        "mod provider;\nuse crate::provider::Provider;\nfn main() { let _ = Provider; }\n",
    );
    fixture.write("src/provider.rs", "pub struct Provider;\n");
    let attribution = resolved_implementation_attribution(&fixture)
        .replace("scope_path: fixture", "scope_path: scope-a")
        .replace("canonical_scope: fixture", "canonical_scope: scope-a")
        .replace(
            "    - dsl_name: physical-structure\n",
            "    - dsl_name: logical-structure\n      version: 2\n      artifact_identity:\n        axes:\n          scope_path: scope-b\n      sha256: dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\n    - dsl_name: projection\n      version: 2\n      artifact_identity:\n        axes:\n          scope_path: scope-b\n      sha256: eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\n    - dsl_name: physical-structure\n",
        )
        .replace(
            "file_attributions:\n  - observed_file_path: src/main.rs\n    canonical_scope: scope-a\n    component_id: application\n    element_id: main",
            "file_attributions:\n  - observed_file_path: src/main.rs\n    canonical_scope: scope-a\n    component_id: application\n    element_id: main\n  - observed_file_path: src/provider.rs\n    canonical_scope: scope-b\n    component_id: provider\n    element_id: provider-file",
        )
        .replace(
            "scope_dependencies: []",
            "scope_dependencies:\n  - from_scope: scope-a\n    to_scope: scope-b\n    contract_refs:\n      - subject_ref: provider\n        contract_id: contract.example",
        );
    fixture.write("resolved-implementation-attribution.sdd.yml", &attribution);

    // Act
    let report = completed_json(&run_resolved_attribution_check(&fixture, &[]), 0);
    let artifacts = report["normalized_sdd_coverage"]["source_provenance"]["artifacts"]
        .as_array()
        .expect("provenance artifacts array");

    // Assert
    assert_eq!(report["decision"], "conformant");
    assert_eq!(artifacts.len(), 5);
    assert_eq!(
        report["normalized_sdd_coverage"]["covered_modules"],
        serde_json::json!(["crate", "crate::provider"])
    );
    assert_eq!(
        report["normalized_sdd_coverage"]["unresolved_modules"],
        serde_json::json!([])
    );

    // Arrange
    let no_edge_fixture = TempFixture::new("resolved-attribution-missing-scope-edge");
    no_edge_fixture.write(
        "src/main.rs",
        "mod provider;\nuse crate::provider::Provider;\nfn main() { let _ = Provider; }\n",
    );
    no_edge_fixture.write("src/provider.rs", "pub struct Provider;\n");
    no_edge_fixture.write(
        "resolved-implementation-attribution.sdd.yml",
        &attribution.replace(
            "scope_dependencies:\n  - from_scope: scope-a\n    to_scope: scope-b\n    contract_refs:\n      - subject_ref: provider\n        contract_id: contract.example",
            "scope_dependencies: []",
        ),
    );

    // Act
    let report_without_edge =
        completed_json(&run_resolved_attribution_check(&no_edge_fixture, &[]), 1);

    // Assert
    assert_eq!(report_without_edge["decision"], "non-conformant");
}

#[test]
fn should_public_resolved_attribution_is_indeterminate_when_unresolved() {
    // Arrange
    let fixture = TempFixture::new("resolved-attribution-public-unresolved");
    let content = resolved_implementation_attribution(&fixture).replacen(
        "file_attributions:\n  - observed_file_path: src/main.rs\n    canonical_scope: fixture\n    component_id: application\n    element_id: main",
        "file_attributions: []",
        1,
    );
    fixture.write("resolved-implementation-attribution.sdd.yml", &content);

    // Act
    let report = completed_json(&run_resolved_attribution_check(&fixture, &[]), 2);

    // Assert
    assert_eq!(report["decision"], "indeterminate");
    assert_eq!(
        report["normalized_sdd_coverage"]["unresolved_modules"][0]["module_id"],
        "crate"
    );
}

#[test]
fn should_public_resolved_attribution_forbidden_scope_edge_is_non_conformant() {
    // Arrange
    let fixture = TempFixture::new("resolved-attribution-public-forbidden");
    fixture.write(
        "src/main.rs",
        "mod provider;\nuse crate::provider::Thing;\nfn main() { let _ = Thing; }\n",
    );
    fixture.write("src/provider.rs", "pub struct Thing;\n");
    let content = resolved_implementation_attribution(&fixture)
        .replace("canonical_scope: fixture", "canonical_scope: scope-a")
        .replacen(
            "component_dependencies: []",
            "  - observed_file_path: src/provider.rs\n    canonical_scope: scope-b\n    component_id: provider\n    element_id: provider\ncomponent_dependencies: []",
            1,
        );
    fixture.write("resolved-implementation-attribution.sdd.yml", &content);

    // Act
    let report = completed_json(&run_resolved_attribution_check(&fixture, &[]), 1);

    // Assert
    assert_eq!(report["decision"], "non-conformant");
}

#[test]
fn should_invalid_resolved_attribution_is_rejected_before_analysis() {
    // Arrange
    let cases = [
        ("malformed", "invalid resolved attribution YAML"),
        (
            "unknown-version",
            "unsupported resolved attribution version: 9",
        ),
        (
            "unknown-name",
            "unexpected resolved attribution name: unknown-attribution",
        ),
        ("provenance-mismatch", "unsupported provenance version"),
        ("legacy-contracts-field", "unknown field `contracts`"),
        ("legacy-coverage-field", "unknown field `coverage`"),
        (
            "missing-component-dependencies",
            "missing field `component_dependencies`",
        ),
        (
            "missing-scope-dependencies",
            "missing field `scope_dependencies`",
        ),
        ("empty-file-scope", "canonical scope must not be empty"),
        ("empty-component-id", "component ID must not be empty"),
        ("empty-element-id", "element ID must not be empty"),
        (
            "empty-component-dependency-scope",
            "component dependency scope must not be empty",
        ),
        (
            "empty-component-dependency-source",
            "component dependency source must not be empty",
        ),
        (
            "empty-component-dependency-target",
            "component dependency target must not be empty",
        ),
        (
            "empty-scope-dependency-source",
            "scope dependency source must not be empty",
        ),
        (
            "empty-scope-dependency-target",
            "scope dependency target must not be empty",
        ),
        (
            "empty-contract-subject",
            "scope dependency subject reference must not be empty",
        ),
        (
            "empty-contract-id",
            "scope dependency contract ID must not be empty",
        ),
    ];

    // Act
    let outputs = cases.map(|(label, expected)| {
        let fixture = TempFixture::new(label);
        let valid = resolved_implementation_attribution(&fixture);
        let content = match label {
            "malformed" => "not: [valid".to_owned(),
            "unknown-version" => valid.replacen("version: 1", "version: 9", 1),
            "unknown-name" => valid.replacen(
                "name: resolved-implementation-attribution",
                "name: unknown-attribution",
                1,
            ),
            "provenance-mismatch" => valid.replacen(
                "    - dsl_name: logical-structure\n      version: 2",
                "    - dsl_name: logical-structure\n      version: 9",
                1,
            ),
            "legacy-contracts-field" => {
                valid.replacen("source_provenance:", "contracts: {}\nsource_provenance:", 1)
            }
            "legacy-coverage-field" => {
                valid.replacen("file_attributions:", "coverage: {}\nfile_attributions:", 1)
            }
            "missing-component-dependencies" => {
                valid.replacen("component_dependencies: []\n", "", 1)
            }
            "missing-scope-dependencies" => valid.replacen("scope_dependencies: []\n", "", 1),
            "empty-file-scope" => {
                valid.replacen("canonical_scope: fixture", "canonical_scope: ''", 1)
            }
            "empty-component-id" => {
                valid.replacen("component_id: application", "component_id: ''", 1)
            }
            "empty-element-id" => valid.replacen("element_id: main", "element_id: ''", 1),
            "empty-component-dependency-scope" => valid.replacen(
                "component_dependencies: []",
                "component_dependencies:\n  - canonical_scope: ''\n    from_component_id: application\n    to_component_id: provider",
                1,
            ),
            "empty-component-dependency-source" => valid.replacen(
                "component_dependencies: []",
                "component_dependencies:\n  - canonical_scope: fixture\n    from_component_id: ''\n    to_component_id: provider",
                1,
            ),
            "empty-component-dependency-target" => valid.replacen(
                "component_dependencies: []",
                "component_dependencies:\n  - canonical_scope: fixture\n    from_component_id: application\n    to_component_id: ''",
                1,
            ),
            "empty-scope-dependency-source" => valid.replacen(
                "scope_dependencies: []",
                "scope_dependencies:\n  - from_scope: ''\n    to_scope: scope-b\n    contract_refs:\n      - subject_ref: provider\n        contract_id: contract.example",
                1,
            ),
            "empty-scope-dependency-target" => valid.replacen(
                "scope_dependencies: []",
                "scope_dependencies:\n  - from_scope: fixture\n    to_scope: ''\n    contract_refs:\n      - subject_ref: provider\n        contract_id: contract.example",
                1,
            ),
            "empty-contract-subject" => valid.replacen(
                "scope_dependencies: []",
                "scope_dependencies:\n  - from_scope: fixture\n    to_scope: scope-b\n    contract_refs:\n      - subject_ref: ''\n        contract_id: contract.example",
                1,
            ),
            "empty-contract-id" => valid.replacen(
                "scope_dependencies: []",
                "scope_dependencies:\n  - from_scope: fixture\n    to_scope: scope-b\n    contract_refs:\n      - subject_ref: provider\n        contract_id: ''",
                1,
            ),
            _ => unreachable!("known normalized input case"),
        };
        fixture.write("resolved-implementation-attribution.sdd.yml", &content);
        fixture.write("Cargo.toml", "not valid TOML = [\n");
        (run_resolved_attribution_check(&fixture, &[]), expected)
    });

    // Assert
    for (output, expected) in outputs {
        assert_execution_failure(&output, expected);
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("invalid Cargo.toml"),
            "resolved attribution diagnostic must precede dependency observation: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn should_documented_boundary_policy_shape_is_accepted() {
    // Arrange
    let fixture = TempFixture::new("documented-policy-shape");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-application-to-infrastructure\n    kind: forbidden\n    severity: error\n    from: [crate::application]\n    to: [crate::infrastructure]\n    modules: []\n    dependency_kinds: [import]\n",
    );

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_completed_output(&output, 0, "conformant");
}

#[test]
fn should_omitted_boundary_policy_collections_default_to_empty() {
    // Arrange
    let fixture = TempFixture::new("policy-default-collections");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-application-to-infrastructure\n    kind: forbidden\n    severity: error\n    from: [crate::application]\n    to: [crate::infrastructure]\n",
    );

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_completed_output(&output, 0, "conformant");
}

#[test]
fn should_unknown_boundary_policy_field_is_rejected() {
    // Arrange
    let fixture = TempFixture::new("unknown-policy-field");
    fixture.write("policy.yaml", "rules: []\nunknown_field: true\n");

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_execution_failure(&output, "invalid boundary policy YAML");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            fixture
                .join("policy.yaml")
                .to_str()
                .expect("policy path is UTF-8")
        )
    );
}

#[test]
fn should_unsupported_boundary_policy_enum_is_rejected() {
    // Arrange
    let fixture = TempFixture::new("unsupported-policy-enum");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: required-dependency\n    kind: required\n    severity: error\n",
    );

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_execution_failure(&output, "invalid boundary policy YAML");
}

#[test]
fn should_malformed_resolved_attribution_report_its_path() {
    // Arrange
    let fixture = TempFixture::new("malformed-resolved-attribution-path");
    let path = fixture.join("resolved-implementation-attribution.sdd.yml");
    fixture.write(
        "resolved-implementation-attribution.sdd.yml",
        "not: [valid\n",
    );

    // Act
    let output = run_resolved_attribution_check(&fixture, &[]);

    // Assert
    assert_execution_failure(&output, "invalid resolved attribution YAML");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains(path.to_str().expect("attribution path is UTF-8"))
    );
}

#[test]
fn should_invalid_boundary_policy_rules_are_rejected() {
    // Arrange
    let cases = [
        (
            "invalid-policy-rule-id",
            "rules:\n  - id: not_kebab_case\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: [crate::domain]\n",
            "invalid or duplicate boundary policy rule id",
        ),
        (
            "duplicate-policy-rule-id",
            "rules:\n  - id: duplicate-rule\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: [crate::domain]\n  - id: duplicate-rule\n    kind: allowed\n    severity: warning\n    from: [crate]\n    to: [crate::domain]\n",
            "invalid or duplicate boundary policy rule id",
        ),
        (
            "missing-policy-from",
            "rules:\n  - id: missing-from\n    kind: forbidden\n    severity: error\n    to: [crate::domain]\n",
            "requires at least one from pattern",
        ),
        (
            "missing-policy-to",
            "rules:\n  - id: missing-to\n    kind: allowed\n    severity: warning\n    from: [crate]\n",
            "requires at least one to pattern",
        ),
        (
            "missing-circular-modules",
            "rules:\n  - id: missing-modules\n    kind: circular\n    severity: error\n",
            "requires at least one modules pattern",
        ),
        (
            "circular-policy-with-from",
            "rules:\n  - id: circular-with-from\n    kind: circular\n    severity: error\n    from: [crate]\n    modules: [crate::**]\n",
            "cannot define from or to patterns",
        ),
        (
            "forbidden-policy-with-modules",
            "rules:\n  - id: forbidden-with-modules\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: [crate::domain]\n    modules: [crate::**]\n",
            "cannot define modules patterns",
        ),
        (
            "invalid-policy-pattern",
            "rules:\n  - id: invalid-pattern\n    kind: forbidden\n    severity: error\n    from: ['crate::?']\n    to: [crate::domain]\n",
            "invalid logical module pattern",
        ),
    ];

    // Act
    let outputs = cases
        .into_iter()
        .map(|(label, policy, expected)| {
            let fixture = TempFixture::new(label);
            fixture.write("policy.yaml", policy);
            let path = fixture.join("policy.yaml");
            (run_check(&fixture, "json"), expected, path)
        })
        .collect::<Vec<_>>();

    // Assert
    for (output, expected, path) in outputs {
        assert_execution_failure(&output, expected);
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains(path.to_str().expect("policy path is UTF-8"))
        );
    }
}
