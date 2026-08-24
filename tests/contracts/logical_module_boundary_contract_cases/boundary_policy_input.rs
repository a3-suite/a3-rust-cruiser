// contract_id: contract.logical-module-boundary.boundary-policy-input
// integration_id: logical-module-boundary-policy-input

use super::*;

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
