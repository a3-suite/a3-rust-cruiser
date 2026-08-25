// contract_id: contract.logical-module-boundary.boundary-policy-input
// integration_id: logical-module-boundary-policy-input

use super::*;

#[test]
fn sdd_design_is_projected_to_a_boundary_policy() {
    // Arrange
    let fixture = TempFixture::new("sdd-policy-input");
    fixture.write(
        "src/main.rs",
        "mod application;\nmod domain;\nfn main() {}\n",
    );
    fixture.write("src/application.rs", "pub fn run() {}\n");
    fixture.write("src/domain.rs", "pub struct Entity;\n");
    write_sdd_policy_input(&fixture);

    // Act
    let output = run_sdd_check(&fixture, true);

    // Assert
    assert_completed_output(&output, 0, "conformant");
}

#[test]
fn many_to_one_sdd_projection_with_non_rust_nodes_is_accepted() {
    // Arrange
    let fixture = many_to_one_sdd_fixture();

    // Act
    let output = run_sdd_check(&fixture, true);

    // Assert
    assert_completed_output(&output, 1, "non-conformant");
}

#[test]
fn incomplete_sdd_design_input_is_rejected() {
    // Arrange
    let fixture = TempFixture::new("incomplete-sdd-policy-input");
    write_sdd_policy_input(&fixture);

    // Act
    let output = run_sdd_check(&fixture, false);

    // Assert
    assert_execution_failure(&output, "SDD boundary policy input requires");
}

#[test]
fn circular_sdd_component_dependencies_are_rejected() {
    // Arrange
    let fixture = TempFixture::new("circular-sdd-policy-input");
    write_sdd_policy_input(&fixture);
    fixture.write(
        "logical-structure.sdd.yml",
        "version: 1\nname: logical-structure\nstructure_id: fixture\ncomponents:\n  - component_id: application\n  - component_id: domain\ndependencies:\n  - from_component_id: application\n    to_component_id: domain\n  - from_component_id: domain\n    to_component_id: application\n",
    );

    // Act
    let output = run_sdd_check(&fixture, true);

    // Assert
    assert_execution_failure(
        &output,
        "logical component dependency graph must be acyclic",
    );
}

#[test]
fn cargo_target_root_component_projections_are_rejected() {
    // Arrange
    let conventional = TempFixture::new("conventional-target-root-sdd-policy-input");
    write_sdd_policy_input(&conventional);
    conventional.write(
        "directory-structure.sdd.yml",
        "nodes:\n  - node_id: src-application\n    path: src/main.rs\n  - node_id: src-domain\n    path: src/domain.rs\n",
    );
    let custom = TempFixture::new("custom-target-root-sdd-policy-input");
    custom.write_manifest(&format!(
        "[[bin]]\nname = \"{}\"\npath = \"src/entry.rs\"\n",
        custom.package()
    ));
    custom.write("src/entry.rs", "fn main() {}\n");
    write_sdd_policy_input(&custom);
    custom.write(
        "directory-structure.sdd.yml",
        "nodes:\n  - node_id: src-application\n    path: src/entry.rs\n  - node_id: src-domain\n    path: src/domain.rs\n",
    );

    // Act
    let outputs = [
        run_sdd_check(&conventional, true),
        run_sdd_check(&custom, true),
    ];

    // Assert
    for output in outputs {
        assert_execution_failure(&output, "selected Cargo target root");
    }
}

#[test]
fn overlapping_rust_module_projections_are_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("overlapping-rust-module-sdd-policy-input");
    write_sdd_policy_input(&fixture);
    fixture.write(
        "directory-structure.sdd.yml",
        "nodes:\n  - node_id: src-application\n    path: src/application\n  - node_id: src-domain\n    path: src/application/domain.rs\n",
    );

    // Act
    let output = run_sdd_check(&fixture, true);

    // Assert
    let report = completed_json(&output, 2);
    assert_issue(&report, "analysis.ambiguous-component-attribution");
}

#[test]
fn inconsistent_sdd_design_inputs_are_rejected() {
    // Arrange
    let cases = [
        (
            "empty-components",
            "logical-structure.sdd.yml",
            "components: []\ndependencies: []\n",
            "logical-structure requires a component",
        ),
        (
            "duplicate-component",
            "logical-structure.sdd.yml",
            "components:\n  - component_id: application\n  - component_id: application\ndependencies: []\n",
            "duplicate logical component ID",
        ),
        (
            "unknown-dependency-source",
            "logical-structure.sdd.yml",
            "components:\n  - component_id: application\n  - component_id: domain\ndependencies:\n  - from_component_id: unknown\n    to_component_id: domain\n",
            "unknown source component",
        ),
        (
            "unknown-dependency-target",
            "logical-structure.sdd.yml",
            "components:\n  - component_id: application\n  - component_id: domain\ndependencies:\n  - from_component_id: application\n    to_component_id: unknown\n",
            "unknown target component",
        ),
        (
            "missing-component-projection",
            "projection.sdd.yml",
            "mappings:\n  - component_id: application\n    node_id: src-application\n",
            "each logical component must have exactly one projection",
        ),
        (
            "duplicate-component-projection",
            "projection.sdd.yml",
            "mappings:\n  - component_id: application\n    node_id: src-application\n  - component_id: application\n    node_id: src-domain\n",
            "duplicate projected component ID",
        ),
        (
            "unknown-directory-node",
            "projection.sdd.yml",
            "mappings:\n  - component_id: application\n    node_id: unknown\n  - component_id: domain\n    node_id: src-domain\n",
            "unknown directory node",
        ),
        (
            "duplicate-directory-node",
            "directory-structure.sdd.yml",
            "nodes:\n  - node_id: src-application\n    path: src/application.rs\n  - node_id: src-application\n    path: src/domain.rs\n",
            "duplicate directory node ID",
        ),
        (
            "duplicate-rust-module",
            "directory-structure.sdd.yml",
            "nodes:\n  - node_id: src-application\n    path: src/application.rs\n  - node_id: src-domain\n    path: src/application.rs\n",
            "same Rust module",
        ),
        (
            "non-normalized-project-path",
            "directory-structure.sdd.yml",
            "nodes:\n  - node_id: src-application\n    path: ../application.rs\n  - node_id: src-domain\n    path: src/domain.rs\n",
            "component path must be a normalized path",
        ),
        (
            "unsupported-module-segment",
            "directory-structure.sdd.yml",
            "nodes:\n  - node_id: src-application\n    path: src/application.rs\n  - node_id: src-domain\n    path: src/domain-name.rs\n",
            "unsupported Rust module segment",
        ),
    ];

    // Act
    let outputs = cases
        .into_iter()
        .map(|(label, path, content, expected)| {
            let fixture = TempFixture::new(label);
            write_sdd_policy_input(&fixture);
            fixture.write(path, content);
            (run_sdd_check(&fixture, true), expected)
        })
        .collect::<Vec<_>>();

    // Assert
    for (output, expected) in outputs {
        assert_execution_failure(&output, expected);
    }
}

#[test]
fn documented_boundary_policy_shape_is_accepted() {
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
fn omitted_boundary_policy_collections_default_to_empty() {
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
fn unknown_boundary_policy_field_is_rejected() {
    // Arrange
    let fixture = TempFixture::new("unknown-policy-field");
    fixture.write("policy.yaml", "rules: []\nunknown_field: true\n");

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_execution_failure(&output, "invalid boundary policy YAML");
}

#[test]
fn unsupported_boundary_policy_enum_is_rejected() {
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
fn invalid_boundary_policy_rules_are_rejected() {
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
            (run_check(&fixture, "json"), expected)
        })
        .collect::<Vec<_>>();

    // Assert
    for (output, expected) in outputs {
        assert_execution_failure(&output, expected);
    }
}

fn write_sdd_policy_input(fixture: &TempFixture) {
    fixture.write(
        "logical-structure.sdd.yml",
        "version: 1\nname: logical-structure\nstructure_id: fixture\ncomponents:\n  - component_id: application\n  - component_id: domain\ndependencies:\n  - from_component_id: application\n    to_component_id: domain\n",
    );
    fixture.write(
        "projection.sdd.yml",
        "version: 1\nname: projection\nprojection_id: fixture-source\nmappings:\n  - component_id: application\n    node_id: src-application\n  - component_id: domain\n    node_id: src-domain\n",
    );
    fixture.write(
        "directory-structure.sdd.yml",
        "version: 1\nname: directory-structure\nstructure_id: fixture-source\nnodes:\n  - node_id: src-application\n    path: src/application.rs\n  - node_id: src-domain\n    path: src/domain.rs\n",
    );
}
