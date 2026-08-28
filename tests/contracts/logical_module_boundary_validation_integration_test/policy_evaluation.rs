// contract_id: contract.logical-module-boundary.policy-evaluation
// integration_id: logical-module-boundary-policy-evaluation

use super::*;

#[test]
fn should_forbidden_rule_overrides_matching_allowed_rule() {
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
fn should_applicable_allowed_rules_use_target_union() {
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
fn should_unmatched_applicable_allowed_rules_produce_violation() {
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
fn should_circular_rule_is_independent_from_allowed_rules() {
    // Arrange
    let fixture = circular_fixture("circular-independent");

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
