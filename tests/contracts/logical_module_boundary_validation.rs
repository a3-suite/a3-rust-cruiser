use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

use a3_rust_cruiser::{
    boundary_evaluation::{decide, evaluate},
    boundary_validation,
    model::{
        AnalysisContext, AnalysisIssue, BoundaryPolicy, BoundaryRule, BoundaryViolation,
        Dependency, DependencyKind, LogicalModule, ModuleDeclaration, ObservedDependencyGraph,
        PolicyEvaluation, RuleKind, Severity, SourceEvidence, TargetScope, ValidationDecision,
        ValidationReport, ViolationKind,
    },
    report,
};

static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

// contract_id: contract.logical-module-boundary.dependency-graph
// integration_id: logical-module-boundary-dependency-graph
#[test]
fn typed_dependencies_and_declarations_are_reported() {
    // Arrange
    let fixture = TempFixture::new("typed-dependencies");
    fixture.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [dependencies]\nserde-json = \"1\"\n",
            fixture.package()
        ),
    );
    fixture.write(
        "src/main.rs",
        "use serde_json;\nmod application;\nmod domain;\nmod excluded;\nfn main() {}\n",
    );
    fixture.write(
        "src/application.rs",
        "use crate::domain::Thing;\npub use crate::domain::Thing as PublicThing;\npub fn run(_: crate::domain::Thing) -> Thing { Thing }\n",
    );
    fixture.write(
        "src/domain.rs",
        "pub struct Thing;\npub fn make() -> Thing { Thing }\n",
    );
    fixture.write("src/excluded.rs", "pub struct Ignored;\n");
    let mut context = analysis_context(fixture.package());
    context.exclude = vec!["crate::excluded".to_owned()];

    // Act
    let result = boundary_validation::validate(
        fixture.path(),
        context.clone(),
        &fixture.join("policy.yaml"),
        "json",
    )
    .expect("validation should succeed");

    // Assert
    assert_eq!(result.analysis_context, context);
    assert!(
        result
            .modules
            .iter()
            .any(|module| module.id == "crate::application")
    );
    assert!(
        result
            .modules
            .iter()
            .any(|module| module.id == "crate::domain")
    );
    assert!(result.module_declarations.iter().any(|declaration| {
        declaration.parent == "crate" && declaration.child == "crate::application"
    }));
    let kinds: BTreeSet<_> = result
        .dependencies
        .iter()
        .filter(|dependency| {
            dependency.source == "crate::application" && dependency.target == "crate::domain"
        })
        .map(|dependency| dependency.kind)
        .collect();
    assert_eq!(
        kinds,
        BTreeSet::from([
            DependencyKind::Import,
            DependencyKind::ReExport,
            DependencyKind::PathReference,
        ])
    );
    assert!(result.dependencies.iter().any(|dependency| {
        dependency.target == "serde_json"
            && dependency.kind == DependencyKind::Import
            && dependency.target_scope == TargetScope::ExternalCrate
    }));
    assert!(result.dependencies.iter().any(|dependency| {
        dependency.target == "crate::domain"
            && dependency.target_scope == TargetScope::InternalModule
    }));
    assert!(!result.dependencies.iter().any(|dependency| {
        dependency.source == "crate"
            && matches!(
                dependency.target.as_str(),
                "crate::application" | "crate::domain" | "crate::excluded"
            )
    }));
    assert_eq!(result.excluded_scopes, ["crate::excluded"]);
}

// contract_id: contract.logical-module-boundary.policy-evaluation
// integration_id: logical-module-boundary-policy-evaluation
#[test]
fn forbidden_rule_overrides_matching_allowed_rule() {
    // Arrange
    let graph = graph_with_dependencies(vec![dependency(
        "crate::application",
        "crate::infrastructure",
        DependencyKind::Import,
        3,
        5,
    )]);
    let policy = BoundaryPolicy {
        rules: vec![
            boundary_rule(
                "forbidden-application-to-infrastructure",
                RuleKind::Forbidden,
                Severity::Error,
                &["crate::application"],
                &["crate::infrastructure"],
                &[],
            ),
            boundary_rule(
                "forbidden-second-match",
                RuleKind::Forbidden,
                Severity::Warning,
                &["crate::application"],
                &["crate::infrastructure"],
                &[],
            ),
            boundary_rule(
                "allowed-application-targets",
                RuleKind::Allowed,
                Severity::Warning,
                &["crate::application"],
                &["crate::infrastructure"],
                &[],
            ),
        ],
    };

    // Act
    let (violations, evaluation) = evaluate(&graph, &policy);

    // Assert
    assert_eq!(violations.len(), 2);
    assert!(violations.iter().all(|violation| {
        violation.kind == ViolationKind::Forbidden
            && violation.source == "crate::application"
            && violation.target == "crate::infrastructure"
            && violation.dependency_kind == DependencyKind::Import
            && violation.evidence.path == "src/application.rs"
            && violation.evidence.line == 3
            && violation.evidence.column == 5
    }));
    let mut rule_ids: Vec<_> = violations
        .iter()
        .map(|violation| violation.rule_id.as_str())
        .collect();
    rule_ids.sort_unstable();
    assert_eq!(
        rule_ids,
        [
            "forbidden-application-to-infrastructure",
            "forbidden-second-match",
        ]
    );
    assert_eq!(
        violations
            .iter()
            .find(|violation| violation.rule_id == "forbidden-application-to-infrastructure")
            .map(|violation| violation.severity),
        Some(Severity::Error)
    );
    assert_eq!(
        violations
            .iter()
            .find(|violation| violation.rule_id == "forbidden-second-match")
            .map(|violation| violation.severity),
        Some(Severity::Warning)
    );
    assert_eq!(evaluation.dependency_outcome, "forbidden");
    assert_eq!(evaluation.forbidden_match_handling, "all-matches");
}

#[test]
fn applicable_allowed_rules_use_target_union() {
    // Arrange
    let graph = graph_with_dependencies(vec![dependency(
        "crate::application",
        "crate::domain",
        DependencyKind::PathReference,
        4,
        9,
    )]);
    let policy = BoundaryPolicy {
        rules: vec![
            boundary_rule(
                "allowed-application-shared",
                RuleKind::Allowed,
                Severity::Warning,
                &["crate::application"],
                &["crate::shared"],
                &[],
            ),
            boundary_rule(
                "allowed-application-domain",
                RuleKind::Allowed,
                Severity::Warning,
                &["crate::application"],
                &["crate::domain"],
                &[],
            ),
        ],
    };

    // Act
    let (violations, evaluation) = evaluate(&graph, &policy);

    // Assert
    assert!(violations.is_empty());
    assert_eq!(evaluation.dependency_outcome, "allowed");
    assert_eq!(evaluation.allowed_match_handling, "union");
}

#[test]
fn unmatched_applicable_allowed_rules_produce_violation() {
    // Arrange
    let graph = graph_with_dependencies(vec![dependency(
        "crate::application",
        "crate::infrastructure",
        DependencyKind::Import,
        7,
        11,
    )]);
    let policy = BoundaryPolicy {
        rules: vec![boundary_rule(
            "allowed-application-domain",
            RuleKind::Allowed,
            Severity::Error,
            &["crate::application"],
            &["crate::domain"],
            &[],
        )],
    };

    // Act
    let (violations, evaluation) = evaluate(&graph, &policy);

    // Assert
    assert_eq!(violations.len(), 1);
    assert_violation(
        &violations[0],
        ViolationKind::NotAllowed,
        "allowed-application-domain",
        Severity::Error,
        "crate::application",
        "crate::infrastructure",
        DependencyKind::Import,
        7,
        11,
    );
    assert_eq!(evaluation.dependency_outcome, "not-allowed");
    assert_eq!(evaluation.allowed_match_handling, "union");
}

#[test]
fn circular_rule_is_independent_from_allowed_rules() {
    // Arrange
    let graph = graph_with_dependencies(vec![
        dependency(
            "crate::application",
            "crate::domain",
            DependencyKind::Import,
            2,
            1,
        ),
        dependency(
            "crate::domain",
            "crate::application",
            DependencyKind::ReExport,
            5,
            3,
        ),
    ]);
    let policy = BoundaryPolicy {
        rules: vec![
            boundary_rule(
                "allowed-application-domain",
                RuleKind::Allowed,
                Severity::Warning,
                &["crate::application"],
                &["crate::domain"],
                &[],
            ),
            boundary_rule(
                "allowed-domain-application",
                RuleKind::Allowed,
                Severity::Warning,
                &["crate::domain"],
                &["crate::application"],
                &[],
            ),
            boundary_rule(
                "circular-core-modules",
                RuleKind::Circular,
                Severity::Error,
                &[],
                &[],
                &["crate::application", "crate::domain"],
            ),
        ],
    };

    // Act
    let (violations, evaluation) = evaluate(&graph, &policy);

    // Assert
    assert_eq!(violations.len(), 1);
    let violation = &violations[0];
    assert_eq!(violation.kind, ViolationKind::Circular);
    assert_eq!(violation.rule_id, "circular-core-modules");
    assert_eq!(violation.severity, Severity::Error);
    assert_eq!(violation.source, "crate::application");
    assert_eq!(violation.target, "crate::domain");
    assert_eq!(violation.dependency_kind, DependencyKind::Import);
    assert_eq!(violation.evidence.path, "src/application.rs");
    assert!(violation.evidence.line > 0);
    assert!(violation.evidence.column > 0);
    assert_eq!(
        violation.cycle_path.as_deref(),
        Some(
            [
                "crate::application".to_owned(),
                "crate::domain".to_owned(),
                "crate::application".to_owned(),
            ]
            .as_slice()
        )
    );
    assert_eq!(evaluation.dependency_outcome, "allowed");
    assert_eq!(evaluation.cycle_outcome, "circular");
    assert_eq!(evaluation.cycle_handling, "independent");
}

// contract_id: contract.logical-module-boundary.validation-report
// integration_id: logical-module-boundary-validation-report
#[test]
fn complete_graph_without_violations_is_conformant() {
    // Arrange
    let issue_count = 0;
    let violation_count = 0;

    // Act
    let decision = decide(issue_count, violation_count);

    // Assert
    assert_eq!(decision, ValidationDecision::Conformant);
}

#[test]
fn complete_graph_with_violations_is_non_conformant() {
    // Arrange
    let issue_count = 0;
    let violation_count = 1;

    // Act
    let decision = decide(issue_count, violation_count);

    // Assert
    assert_eq!(decision, ValidationDecision::NonConformant);
}

#[test]
fn incomplete_analysis_is_indeterminate_even_with_violations() {
    // Arrange
    let issue_count = 1;
    let violation_count = 1;

    // Act
    let decision = decide(issue_count, violation_count);

    // Assert
    assert_eq!(decision, ValidationDecision::Indeterminate);
}

#[test]
fn json_report_projects_common_validation_result() {
    // Arrange
    let validation_report = sample_report("json");

    // Act
    let payload = report::to_json(&validation_report).expect("JSON serialization should succeed");
    let document: serde_json::Value =
        serde_json::from_str(&payload).expect("serialized report should be valid JSON");

    // Assert
    assert_eq!(document["schema_version"], "1");
    assert_eq!(document["serialization"], "json");
    assert_eq!(document["ordering"], "canonical");
    assert_eq!(document["decision"], "indeterminate");
    assert_eq!(
        document["analysis_context"]["features"],
        serde_json::json!(["alpha", "zeta"])
    );
    assert_eq!(
        document["analysis_context"]["cfg"],
        serde_json::json!(["target_os=alpha", "target_os=zeta"])
    );
    assert_eq!(
        document["analysis_context"]["include"],
        serde_json::json!(["crate::zeta", "crate::alpha"])
    );
    assert_eq!(
        document["analysis_context"]["exclude"],
        serde_json::json!(["crate::omega", "crate::beta"])
    );
    let module_ids: Vec<_> = document["modules"]
        .as_array()
        .expect("modules should be an array")
        .iter()
        .map(|module| module["id"].as_str().expect("module ID should be text"))
        .collect();
    assert_eq!(
        module_ids,
        ["crate::application", "crate::domain", "crate::shared"]
    );
    let declaration_children: Vec<_> = document["module_declarations"]
        .as_array()
        .expect("declarations should be an array")
        .iter()
        .map(|declaration| {
            declaration["child"]
                .as_str()
                .expect("declaration child should be text")
        })
        .collect();
    assert_eq!(declaration_children, ["crate::domain", "crate::shared"]);
    let dependency_targets: Vec<_> = document["dependencies"]
        .as_array()
        .expect("dependencies should be an array")
        .iter()
        .map(|dependency| {
            dependency["target"]
                .as_str()
                .expect("dependency target should be text")
        })
        .collect();
    assert_eq!(dependency_targets, ["crate::domain", "crate::shared"]);
    let excluded_scopes: Vec<_> = document["excluded_scopes"]
        .as_array()
        .expect("excluded scopes should be an array")
        .iter()
        .map(|scope| scope.as_str().expect("excluded scope should be text"))
        .collect();
    assert_eq!(excluded_scopes, ["crate::alpha", "crate::zeta"]);
    let violation_ids: Vec<_> = document["violations"]
        .as_array()
        .expect("violations should be an array")
        .iter()
        .map(|violation| {
            violation["rule_id"]
                .as_str()
                .expect("rule ID should be text")
        })
        .collect();
    assert_eq!(
        violation_ids,
        [
            "canonical-tie",
            "canonical-tie",
            "canonical-tie",
            "canonical-tie",
            "forbidden-domain",
            "forbidden-shared",
        ]
    );
    let canonical_ties: Vec<_> = document["violations"]
        .as_array()
        .expect("violations should be an array")
        .iter()
        .take(4)
        .map(|violation| {
            (
                violation["kind"].as_str().expect("kind should be text"),
                violation["severity"]
                    .as_str()
                    .expect("severity should be text"),
                violation.get("cycle_path").cloned(),
            )
        })
        .collect();
    assert_eq!(
        canonical_ties,
        [
            ("forbidden", "error", None),
            (
                "circular",
                "error",
                Some(serde_json::json!(["crate::alpha", "crate::shared"]))
            ),
            (
                "circular",
                "error",
                Some(serde_json::json!(["crate::zeta", "crate::shared"]))
            ),
            (
                "circular",
                "warning",
                Some(serde_json::json!(["crate::alpha", "crate::shared"]))
            ),
        ]
    );
    let issue_codes: Vec<_> = document["analysis_issues"]
        .as_array()
        .expect("analysis issues should be an array")
        .iter()
        .map(|issue| issue["code"].as_str().expect("issue code should be text"))
        .collect();
    assert_eq!(issue_codes, ["analysis.fixture", "analysis.zeta"]);
    assert_eq!(document["analysis_issues"][0]["code"], "analysis.fixture");
    assert_eq!(document["analysis_issues"][0]["reason"], "fixture issue");
    assert_eq!(
        document["analysis_issues"][0]["affected_scope"],
        "crate::application"
    );
}

#[test]
fn text_report_projects_common_validation_result() {
    // Arrange
    let validation_report = sample_report("text");

    // Act
    let payload = report::to_text(&validation_report);

    // Assert
    assert!(payload.contains("serialization: text"));
    assert!(payload.contains("ordering: canonical"));
    assert!(payload.contains("decision: indeterminate"));
    assert!(payload.contains("features:\n  - alpha\n  - zeta\n"));
    assert!(payload.contains("cfg:\n  - target_os=alpha\n  - target_os=zeta\n"));
    assert!(payload.contains("include:\n  - crate::zeta\n  - crate::alpha\n"));
    assert!(payload.contains("exclude:\n  - crate::omega\n  - crate::beta\n"));
    assert!(
        payload
            .contains("modules:\n  - crate::application\n  - crate::domain\n  - crate::shared\n")
    );
    assert!(payload.contains(
        "module_declarations:\n  - crate -> crate::domain [src/main.rs:1:1]\n  - crate -> crate::shared [src/main.rs:2:1]\n"
    ));
    assert!(payload.contains(
        "dependencies:\n  - crate::application -> crate::domain (import, internal-module) [src/application.rs:3:5]\n  - crate::application -> crate::shared (path-reference, internal-module) [src/application.rs:4:9]\n"
    ));
    assert!(payload.contains("excluded_scopes:\n  - crate::alpha\n  - crate::zeta\n"));
    let forbidden_domain = payload
        .find("forbidden-domain forbidden")
        .expect("first violation should be present");
    let forbidden_shared = payload
        .find("forbidden-shared forbidden")
        .expect("second violation should be present");
    let fixture_issue = payload
        .find("analysis.fixture [crate::application] fixture issue")
        .expect("first issue should be present");
    let zeta_issue = payload
        .find("analysis.zeta [crate::shared] zeta issue")
        .expect("second issue should be present");
    assert!(forbidden_domain < forbidden_shared);
    assert!(fixture_issue < zeta_issue);
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
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_indeterminate_issue(&result, "analysis.unresolved-reference");
}

#[test]
fn unresolved_single_segment_type_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unresolved-single-segment-type");
    fixture.write("src/main.rs", "fn consume(_: Missing) {}\nfn main() {}\n");

    // Act
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_indeterminate_issue(&result, "analysis.unsupported-name-resolution");
}

#[test]
fn aliases_and_multiple_super_segments_are_resolved() {
    // Arrange
    let fixture = TempFixture::new("aliases-and-multiple-super");
    fixture.write("src/main.rs", "mod outer;\nmod shared;\nfn main() {}\n");
    fixture.write("src/outer.rs", "pub mod inner;\npub mod sibling;\n");
    fixture.write(
        "src/outer/inner.rs",
        "use crate::outer::sibling::Choice as Alias;\n\
         pub fn choose(_: Alias) -> super::super::shared::Thing {\n\
             super::super::shared::Thing\n\
         }\n",
    );
    fixture.write("src/outer/sibling.rs", "pub enum Choice { One }\n");
    fixture.write("src/shared.rs", "pub struct Thing;\n");

    // Act
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_eq!(result.decision, ValidationDecision::Conformant);
    assert!(result.dependencies.iter().any(|dependency| {
        dependency.source == "crate::outer::inner" && dependency.target == "crate::outer::sibling"
    }));
    assert!(result.dependencies.iter().any(|dependency| {
        dependency.source == "crate::outer::inner" && dependency.target == "crate::shared"
    }));
}

#[test]
fn block_scoped_use_is_indeterminate_until_scope_resolution_is_supported() {
    // Arrange
    let fixture = TempFixture::new("block-scoped-use");
    fixture.write(
        "src/main.rs",
        "mod forbidden;\n\
         fn main() {\n\
             use crate::forbidden::run as invoke;\n\
             invoke();\n\
         }\n",
    );
    fixture.write("src/forbidden.rs", "pub fn run() {}\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbidden-block-import\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: [crate::forbidden]\n",
    );

    // Act
    let result = boundary_validation::validate(
        fixture.path(),
        analysis_context(fixture.package()),
        &fixture.join("policy.yaml"),
        "json",
    )
    .expect("unsupported block-scoped use should produce a report");

    // Assert
    assert_indeterminate_issue(&result, "analysis.unsupported-name-resolution");
}

#[test]
fn repeated_json_report_is_byte_stable() {
    // Arrange
    let first_report = sample_report("json");
    let second_report = rebuild_with_reversed_inputs(sample_report("json"));

    // Act
    let first = report::to_json(&first_report).expect("first serialization should succeed");
    let second = report::to_json(&second_report).expect("second serialization should succeed");

    // Assert
    assert_eq!(first.as_bytes(), second.as_bytes());
    assert!(first.contains("\"schema_version\": \"1\""));
    assert!(
        first.find("forbidden-domain").expect("first violation")
            < first.find("forbidden-shared").expect("second violation")
    );
    assert!(
        first.find("analysis.fixture").expect("first issue")
            < first.find("analysis.zeta").expect("second issue")
    );
}

#[test]
fn unsupported_macro_expansion_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unsupported-macro");
    fixture.write("src/main.rs", "fn main() { println!(\"hello\"); }\n");

    // Act
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_indeterminate_issue(&result, "analysis.unsupported-macro-expansion");
}

#[test]
fn unsupported_derive_macro_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unsupported-derive-macro");
    fixture.write(
        "src/main.rs",
        "#[derive(Debug)]\nstruct Value;\nfn main() {}\n",
    );

    // Act
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_indeterminate_issue(&result, "analysis.unsupported-macro-expansion");
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
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_indeterminate_issue(&result, "analysis.unsupported-name-resolution");
}

#[test]
fn unsupported_expression_name_resolution_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unsupported-expression-name-resolution");
    fixture.write(
        "src/main.rs",
        "fn main() { let _ = missing_module::Thing; }\n",
    );

    // Act
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_indeterminate_issue(&result, "analysis.unsupported-name-resolution");
}

#[test]
fn unsupported_syntax_is_indeterminate() {
    // Arrange
    let fixture = TempFixture::new("unsupported-syntax");
    fixture.write("src/main.rs", "fn main( {\n");

    // Act
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_indeterminate_issue(&result, "analysis.unsupported-syntax");
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
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_indeterminate_issue(&result, "analysis.unsupported-analysis-context");
}

// contract_id: contract.logical-module-boundary.cli-outcome
// integration_id: logical-module-boundary-cli-outcome
#[test]
fn conformant_result_exits_with_zero() {
    // Arrange
    let fixture = TempFixture::new("conformant-exit");
    fixture.write("src/main.rs", "fn main() {}\n");

    // Act
    let output = run_check(&fixture, "json");

    // Assert
    assert_completed_output(&output, 0, "conformant");
}

#[test]
fn non_conformant_result_exits_with_one() {
    // Arrange
    let fixture = TempFixture::new("non-conformant-exit");
    fixture.write(
        "src/main.rs",
        "mod application;\nmod infrastructure;\nfn main() {}\n",
    );
    fixture.write(
        "src/application.rs",
        "use crate::infrastructure::Thing;\npub fn run() -> Thing { Thing }\n",
    );
    fixture.write("src/infrastructure.rs", "pub struct Thing;\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbidden-application\n    kind: forbidden\n    severity: error\n    from: [crate::application]\n    to: [crate::infrastructure]\n",
    );

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
    fixture.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [lib]\npath = \"sources/library.rs\"\n\
             [[bin]]\nname = \"tool\"\npath = \"sources/tool.rs\"\n\
             [[example]]\nname = \"demo\"\npath = \"sources/demo.rs\"\n\
             [[test]]\nname = \"contract\"\npath = \"sources/contract.rs\"\n\
             [[bench]]\nname = \"performance\"\npath = \"sources/performance.rs\"\n",
            fixture.package()
        ),
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
    let results: Vec<_> = targets
        .iter()
        .map(|target| validate_target(&fixture, target))
        .collect();

    // Assert
    for (target, result) in targets.iter().zip(results) {
        let result = result.expect("explicit compilation target should be resolved");
        assert_eq!(result.analysis_context.compilation_target, *target);
        assert_eq!(result.decision, ValidationDecision::Conformant);
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
        "bin:tool",
        "bin:nested",
        "example:demo",
        "example:nested",
        "test:contract",
        "bench:performance",
    ];

    // Act
    let results: Vec<_> = targets
        .iter()
        .map(|target| validate_target(&fixture, target))
        .collect();

    // Assert
    assert!(results.iter().all(Result::is_ok));
}

#[test]
fn target_feature_requirements_and_auto_discovery_are_fail_closed() {
    // Arrange
    let fixture = TempFixture::new("target-selection-context");
    fixture.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\nautobins = false\n\
             [[bin]]\nname = \"tool\"\npath = \"src/bin/tool.rs\"\nrequired-features = [\"cli\"]\n\
             [[bin]]\nname = \"missing\"\npath = \"src/bin/missing.rs\"\n",
            fixture.package()
        ),
    );
    fixture.write("src/bin/tool.rs", "fn main() {}\n");
    fixture.write("src/bin/hidden.rs", "fn main() {}\n");

    // Act
    let required_feature_result = validate_target(&fixture, "bin:tool")
        .expect("missing required features should produce a report");
    let mut active_feature_context = analysis_context(fixture.package());
    active_feature_context.compilation_target = "bin:tool".to_owned();
    active_feature_context.features = vec!["cli".to_owned()];
    let active_feature_result = boundary_validation::validate(
        fixture.path(),
        active_feature_context,
        &fixture.join("policy.yaml"),
        "json",
    )
    .expect("active required features should select the target");
    let disabled_auto_target = validate_target(&fixture, "bin:hidden");
    let missing_target_source = validate_target(&fixture, "bin:missing");
    fixture.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\nautolib = false\n",
            fixture.package()
        ),
    );
    fixture.write("src/lib.rs", "pub fn library() {}\n");
    let disabled_auto_library = validate_target(&fixture, "lib");

    // Assert
    assert_indeterminate_issue(
        &required_feature_result,
        "analysis.unsupported-analysis-context",
    );
    assert_eq!(
        active_feature_result.decision,
        ValidationDecision::Conformant
    );
    assert!(
        disabled_auto_target
            .expect_err("autobins=false should disable implicit binaries")
            .to_string()
            .contains("bin target not found")
    );
    assert!(
        missing_target_source
            .expect_err("missing explicit target source should fail")
            .to_string()
            .contains("target source file not found")
    );
    assert!(
        disabled_auto_library
            .expect_err("autolib=false should disable the implicit library")
            .to_string()
            .contains("library target not found")
    );
}

#[test]
fn inline_nested_and_declared_external_dependencies_are_observed() {
    // Arrange
    let fixture = TempFixture::new("module-shapes");
    fixture.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [dependencies]\nserde-json = \"1\"\n",
            fixture.package()
        ),
    );
    fixture.write("src/main.rs", "mod outer;\nuse serde_json;\nfn main() {}\n");
    fixture.write(
        "src/outer.rs",
        "pub mod child;\npub mod inline {\n    pub mod nested;\n    use super::child::{Thing as Alias, *};\n    pub fn make() -> super::child::Thing { super::child::Thing }\n}\n",
    );
    fixture.write("src/outer/child.rs", "pub struct Thing;\n");
    fixture.write("src/outer/inline/nested.rs", "pub struct Nested;\n");
    let mut context = analysis_context(fixture.package());
    context.include = vec!["crate::**".to_owned()];

    // Act
    let result = boundary_validation::validate(
        fixture.path(),
        context,
        &fixture.join("policy.yaml"),
        "json",
    )
    .expect("module shapes should be observed");

    // Assert
    assert!(
        result
            .modules
            .iter()
            .any(|module| module.id == "crate::outer::inline")
    );
    assert!(
        result
            .modules
            .iter()
            .any(|module| module.id == "crate::outer::child")
    );
    assert!(
        result
            .modules
            .iter()
            .any(|module| module.id == "crate::outer::inline::nested")
    );
    assert!(result.modules.iter().any(|module| module.id == "crate"));
    assert!(result.dependencies.iter().any(|dependency| {
        dependency.source == "crate"
            && dependency.target == "serde_json"
            && dependency.kind == DependencyKind::Import
            && dependency.target_scope == TargetScope::ExternalCrate
    }));
    assert!(result.dependencies.iter().any(|dependency| {
        dependency.source == "crate::outer::inline"
            && dependency.target == "crate::outer::child"
            && dependency.kind == DependencyKind::Import
    }));
    assert!(!result.dependencies.iter().any(|dependency| {
        dependency.source == "crate" && dependency.target == "crate::outer::child"
    }));
}

#[test]
fn leading_colon_selects_the_external_crate_namespace() {
    // Arrange
    let fixture = TempFixture::new("absolute-external-path");
    fixture.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [dependencies]\nserde = \"1\"\n",
            fixture.package()
        ),
    );
    fixture.write(
        "src/main.rs",
        "mod serde;\nuse ::serde::Serialize;\nfn main() {}\n",
    );
    fixture.write("src/serde.rs", "pub struct Local;\n");

    // Act
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_eq!(result.decision, ValidationDecision::Conformant);
    assert!(result.dependencies.iter().any(|dependency| {
        dependency.target == "serde"
            && dependency.target_scope == TargetScope::ExternalCrate
            && dependency.kind == DependencyKind::Import
    }));
    assert!(!result.dependencies.iter().any(|dependency| {
        dependency.target == "crate::serde" && dependency.kind == DependencyKind::Import
    }));
}

#[test]
fn declared_item_kinds_support_complete_path_resolution() {
    // Arrange
    let fixture = TempFixture::new("declared-item-kinds");
    fixture.write(
        "src/main.rs",
        "mod domain;\nfn consume(_: crate::domain::Model) {}\nfn main() {}\n",
    );
    fixture.write(
        "src/domain.rs",
        "pub const VALUE: usize = 1;\n\
         pub enum Choice { One }\n\
         extern crate core as core_alias;\n\
         pub fn make() {}\n\
         macro_rules! named { () => {} }\n\
         pub mod nested {}\n\
         pub static STATE: usize = 1;\n\
         pub struct Model;\n\
         pub trait Contract {}\n\
         pub trait Alias = Contract;\n\
         pub type Identifier = usize;\n\
         pub union Number { value: usize }\n\
         use core::fmt::{self, Debug as DebugAlias, *};\n",
    );

    // Act
    let result = validate_empty_policy(&fixture);

    // Assert
    assert!(result.dependencies.iter().any(|dependency| {
        dependency.source == "crate"
            && dependency.target == "crate::domain"
            && dependency.kind == DependencyKind::PathReference
    }));
}

#[test]
fn external_dependencies_follow_the_selected_compilation_target() {
    // Arrange
    let fixture = TempFixture::new("target-dependencies");
    fixture.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [dev-dependencies]\ntest-only = \"1\"\n\
             [target.x86_64-unknown-linux-gnu.dependencies]\ntarget-only = \"1\"\n",
            fixture.package()
        ),
    );
    fixture.write("src/main.rs", "use test_only;\nfn main() {}\n");
    fixture.write("tests/contract.rs", "use test_only;\nfn contract() {}\n");

    // Act
    let bin_result = validate_empty_policy(&fixture);
    let test_result = validate_target(&fixture, "test:contract")
        .expect("test target dependencies should be observed");

    // Assert
    assert_indeterminate_issue(&bin_result, "analysis.unsupported-name-resolution");
    assert_eq!(test_result.decision, ValidationDecision::Conformant);
    assert!(test_result.dependencies.iter().any(|dependency| {
        dependency.target == "test_only" && dependency.target_scope == TargetScope::ExternalCrate
    }));
}

#[test]
fn exact_target_dependency_is_active_for_the_selected_triple() {
    // Arrange
    let fixture = TempFixture::new("exact-target-dependency");
    fixture.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [target.x86_64-unknown-linux-gnu.dependencies]\ntarget-only = \"1\"\n",
            fixture.package()
        ),
    );
    fixture.write("src/main.rs", "use target_only;\nfn main() {}\n");

    // Act
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_eq!(result.decision, ValidationDecision::Conformant);
    assert!(result.dependencies.iter().any(|dependency| {
        dependency.target == "target_only" && dependency.target_scope == TargetScope::ExternalCrate
    }));
}

#[test]
fn cfg_selected_dependency_is_indeterminate_without_cfg_evaluation() {
    // Arrange
    let fixture = TempFixture::new("cfg-target-dependency");
    fixture.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [target.'cfg(unix)'.dependencies]\ncfg-only = \"1\"\n",
            fixture.package()
        ),
    );
    fixture.write("src/main.rs", "use cfg_only;\nfn main() {}\n");

    // Act
    let result = validate_empty_policy(&fixture);

    // Assert
    assert_indeterminate_issue(&result, "analysis.unsupported-analysis-context");
}

#[test]
fn optional_dependency_is_indeterminate_without_feature_evaluation() {
    // Arrange
    let fixture = TempFixture::new("optional-dependency");
    fixture.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [dependencies]\noptional-crate = {{ version = \"1\", optional = true }}\n",
            fixture.package()
        ),
    );
    fixture.write("src/main.rs", "use optional_crate;\nfn main() {}\n");
    let mut context = analysis_context(fixture.package());
    context.features = vec!["optional-crate".to_owned()];

    // Act
    let result = boundary_validation::validate(
        fixture.path(),
        context,
        &fixture.join("policy.yaml"),
        "json",
    )
    .expect("unsupported feature evaluation should produce a report");

    // Assert
    assert_indeterminate_issue(&result, "analysis.unsupported-analysis-context");
}

#[test]
fn invalid_manifest_and_target_requests_fail_with_diagnostics() {
    // Arrange
    let fixture = TempFixture::new("invalid-targets");
    let mismatched_package = AnalysisContext {
        package: "other-package".to_owned(),
        ..analysis_context(fixture.package())
    };
    let mut empty_bin_name = analysis_context(fixture.package());
    empty_bin_name.compilation_target = "bin:".to_owned();
    let mut empty_example_name = analysis_context(fixture.package());
    empty_example_name.compilation_target = "example:".to_owned();
    let mut unsupported_target = analysis_context(fixture.package());
    unsupported_target.compilation_target = "proc-macro:fixture".to_owned();

    // Act
    let mismatch = boundary_validation::validate(
        fixture.path(),
        mismatched_package,
        &fixture.join("policy.yaml"),
        "json",
    );
    let empty_bin = boundary_validation::validate(
        fixture.path(),
        empty_bin_name,
        &fixture.join("policy.yaml"),
        "json",
    );
    let empty_example = boundary_validation::validate(
        fixture.path(),
        empty_example_name,
        &fixture.join("policy.yaml"),
        "json",
    );
    let unsupported = boundary_validation::validate(
        fixture.path(),
        unsupported_target,
        &fixture.join("policy.yaml"),
        "json",
    );
    fixture.write("Cargo.toml", "not valid TOML = [\n");
    let invalid_manifest = validate_target(&fixture, "lib");

    // Assert
    assert!(
        mismatch
            .expect_err("package mismatch should fail")
            .to_string()
            .contains("does not match")
    );
    assert!(
        empty_bin
            .expect_err("empty bin target should fail")
            .to_string()
            .contains("requires a name")
    );
    assert!(
        empty_example
            .expect_err("empty example target should fail")
            .to_string()
            .contains("requires a name")
    );
    assert!(
        unsupported
            .expect_err("unsupported target should fail")
            .to_string()
            .contains("unsupported compilation target")
    );
    assert!(
        invalid_manifest
            .expect_err("invalid manifest should fail")
            .to_string()
            .contains("invalid Cargo.toml")
    );
}

#[test]
fn manifest_target_and_dependency_types_are_rejected() {
    // Arrange
    let invalid_auto = TempFixture::new("invalid-auto-flag");
    invalid_auto.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\nautobins = \"false\"\n",
            invalid_auto.package()
        ),
    );
    invalid_auto.write("src/bin/hidden.rs", "fn main() {}\n");

    let invalid_required = TempFixture::new("invalid-required-features");
    invalid_required.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [[bin]]\nname = \"tool\"\npath = \"src/main.rs\"\nrequired-features = \"cli\"\n",
            invalid_required.package()
        ),
    );

    let invalid_optional = TempFixture::new("invalid-optional-flag");
    invalid_optional.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             [dependencies]\noptional-crate = {{ version = \"1\", optional = \"true\" }}\n",
            invalid_optional.package()
        ),
    );
    invalid_optional.write("src/main.rs", "fn main() {}\n");

    // Act
    let auto_result = validate_target(&invalid_auto, "bin:hidden");
    let required_result = validate_target(&invalid_required, "bin:tool");
    let optional_result = validate_empty_policy_result(&invalid_optional);

    // Assert
    assert!(
        auto_result
            .expect_err("non-boolean auto flag should fail")
            .to_string()
            .contains("must be a boolean")
    );
    assert!(
        required_result
            .expect_err("non-array required-features should fail")
            .to_string()
            .contains("must be an array")
    );
    assert!(
        optional_result
            .expect_err("non-boolean optional flag should fail")
            .to_string()
            .contains("optional must be a boolean")
    );
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
        (
            "target-selector",
            "[target]\nbad = \"value\"\n",
            "target.bad must be a table",
        ),
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

    for (label, manifest_fragment, expected) in cases {
        let fixture = TempFixture::new(label);
        fixture.write(
            "Cargo.toml",
            &format!(
                "{manifest_fragment}[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
                fixture.package()
            ),
        );
        fixture.write("src/main.rs", "fn main() {}\n");

        // Act
        let result = validate_empty_policy_result(&fixture);

        // Assert
        assert!(
            result
                .expect_err("malformed manifest shape should fail")
                .to_string()
                .contains(expected),
            "case {label} should contain {expected}"
        );
    }
}

#[test]
fn missing_required_analysis_context_is_rejected() {
    // Arrange
    let fixture = TempFixture::new("missing-analysis-context");
    let mut context = analysis_context(fixture.package());
    context.target_triple.clear();

    // Act
    let result = boundary_validation::validate(
        fixture.path(),
        context,
        &fixture.join("policy.yaml"),
        "json",
    );

    // Assert
    assert!(
        result
            .expect_err("incomplete analysis context should fail")
            .to_string()
            .contains("package, target, and target_triple are required")
    );
}

fn analysis_context(package: &str) -> AnalysisContext {
    AnalysisContext {
        package: package.to_owned(),
        compilation_target: format!("bin:{package}"),
        features: Vec::new(),
        cfg: Vec::new(),
        target_triple: "x86_64-unknown-linux-gnu".to_owned(),
        include: Vec::new(),
        exclude: Vec::new(),
    }
}

fn dependency(
    source: &str,
    target: &str,
    kind: DependencyKind,
    line: usize,
    column: usize,
) -> Dependency {
    Dependency {
        candidate_id: format!("dep:{source}:{target}:{}", kind.as_str()),
        source: source.to_owned(),
        target: target.to_owned(),
        kind,
        target_scope: TargetScope::InternalModule,
        evidence: SourceEvidence {
            path: format!("src/{}.rs", source.rsplit("::").next().unwrap_or("main")),
            line,
            column,
        },
    }
}

fn graph_with_dependencies(dependencies: Vec<Dependency>) -> ObservedDependencyGraph {
    let module_ids: BTreeSet<String> = dependencies
        .iter()
        .flat_map(|dependency| [dependency.source.clone(), dependency.target.clone()])
        .collect();
    ObservedDependencyGraph {
        analysis_context: analysis_context("fixture"),
        modules: module_ids
            .into_iter()
            .map(|id| LogicalModule { id })
            .collect(),
        module_declarations: Vec::new(),
        dependencies,
        excluded_scopes: Vec::new(),
    }
}

fn boundary_rule(
    id: &str,
    kind: RuleKind,
    severity: Severity,
    from: &[&str],
    to: &[&str],
    modules: &[&str],
) -> BoundaryRule {
    BoundaryRule {
        id: id.to_owned(),
        kind,
        severity,
        from: from.iter().map(|value| (*value).to_owned()).collect(),
        to: to.iter().map(|value| (*value).to_owned()).collect(),
        modules: modules.iter().map(|value| (*value).to_owned()).collect(),
        dependency_kinds: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn assert_violation(
    violation: &BoundaryViolation,
    kind: ViolationKind,
    rule_id: &str,
    severity: Severity,
    source: &str,
    target: &str,
    dependency_kind: DependencyKind,
    line: usize,
    column: usize,
) {
    assert_eq!(violation.kind, kind);
    assert_eq!(violation.rule_id, rule_id);
    assert_eq!(violation.severity, severity);
    assert_eq!(violation.source, source);
    assert_eq!(violation.target, target);
    assert_eq!(violation.dependency_kind, dependency_kind);
    assert!(!violation.evidence.path.is_empty());
    assert_eq!(violation.evidence.line, line);
    assert_eq!(violation.evidence.column, column);
}

fn sample_report(serialization: &str) -> ValidationReport {
    let mut graph = graph_with_dependencies(vec![
        dependency(
            "crate::application",
            "crate::shared",
            DependencyKind::PathReference,
            4,
            9,
        ),
        dependency(
            "crate::application",
            "crate::domain",
            DependencyKind::Import,
            3,
            5,
        ),
    ]);
    graph.analysis_context.features = vec!["zeta".to_owned(), "alpha".to_owned()];
    graph.analysis_context.cfg = vec!["target_os=zeta".to_owned(), "target_os=alpha".to_owned()];
    graph.analysis_context.include = vec!["crate::zeta".to_owned(), "crate::alpha".to_owned()];
    graph.analysis_context.exclude = vec!["crate::omega".to_owned(), "crate::beta".to_owned()];
    graph.modules.reverse();
    graph.module_declarations = vec![
        ModuleDeclaration {
            declaration_id: "decl:crate:shared".to_owned(),
            parent: "crate".to_owned(),
            child: "crate::shared".to_owned(),
            evidence: SourceEvidence {
                path: "src/main.rs".to_owned(),
                line: 2,
                column: 1,
            },
        },
        ModuleDeclaration {
            declaration_id: "decl:crate:domain".to_owned(),
            parent: "crate".to_owned(),
            child: "crate::domain".to_owned(),
            evidence: SourceEvidence {
                path: "src/main.rs".to_owned(),
                line: 1,
                column: 1,
            },
        },
    ];
    graph.excluded_scopes = vec!["crate::zeta".to_owned(), "crate::alpha".to_owned()];
    let violations = vec![
        BoundaryViolation {
            kind: ViolationKind::Circular,
            rule_id: "canonical-tie".to_owned(),
            severity: Severity::Warning,
            source: "crate::application".to_owned(),
            target: "crate::shared".to_owned(),
            dependency_kind: DependencyKind::PathReference,
            evidence: SourceEvidence {
                path: "src/application.rs".to_owned(),
                line: 4,
                column: 9,
            },
            cycle_path: Some(vec!["crate::alpha".to_owned(), "crate::shared".to_owned()]),
        },
        BoundaryViolation {
            kind: ViolationKind::Circular,
            rule_id: "canonical-tie".to_owned(),
            severity: Severity::Error,
            source: "crate::application".to_owned(),
            target: "crate::shared".to_owned(),
            dependency_kind: DependencyKind::PathReference,
            evidence: SourceEvidence {
                path: "src/application.rs".to_owned(),
                line: 4,
                column: 9,
            },
            cycle_path: Some(vec!["crate::zeta".to_owned(), "crate::shared".to_owned()]),
        },
        BoundaryViolation {
            kind: ViolationKind::Forbidden,
            rule_id: "canonical-tie".to_owned(),
            severity: Severity::Error,
            source: "crate::application".to_owned(),
            target: "crate::shared".to_owned(),
            dependency_kind: DependencyKind::PathReference,
            evidence: SourceEvidence {
                path: "src/application.rs".to_owned(),
                line: 4,
                column: 9,
            },
            cycle_path: None,
        },
        BoundaryViolation {
            kind: ViolationKind::Circular,
            rule_id: "canonical-tie".to_owned(),
            severity: Severity::Error,
            source: "crate::application".to_owned(),
            target: "crate::shared".to_owned(),
            dependency_kind: DependencyKind::PathReference,
            evidence: SourceEvidence {
                path: "src/application.rs".to_owned(),
                line: 4,
                column: 9,
            },
            cycle_path: Some(vec!["crate::alpha".to_owned(), "crate::shared".to_owned()]),
        },
        BoundaryViolation {
            kind: ViolationKind::Forbidden,
            rule_id: "forbidden-shared".to_owned(),
            severity: Severity::Warning,
            source: "crate::application".to_owned(),
            target: "crate::shared".to_owned(),
            dependency_kind: DependencyKind::PathReference,
            evidence: SourceEvidence {
                path: "src/application.rs".to_owned(),
                line: 4,
                column: 9,
            },
            cycle_path: None,
        },
        BoundaryViolation {
            kind: ViolationKind::Forbidden,
            rule_id: "forbidden-domain".to_owned(),
            severity: Severity::Error,
            source: "crate::application".to_owned(),
            target: "crate::domain".to_owned(),
            dependency_kind: DependencyKind::Import,
            evidence: SourceEvidence {
                path: "src/application.rs".to_owned(),
                line: 3,
                column: 5,
            },
            cycle_path: None,
        },
    ];
    report::build(
        graph,
        violations,
        PolicyEvaluation {
            dependency_outcome: "forbidden".to_owned(),
            cycle_outcome: "acyclic".to_owned(),
            forbidden_match_handling: "all-matches".to_owned(),
            allowed_match_handling: "union".to_owned(),
            cycle_handling: "independent".to_owned(),
        },
        vec![
            AnalysisIssue {
                code: "analysis.zeta".to_owned(),
                reason: "zeta issue".to_owned(),
                affected_scope: "crate::shared".to_owned(),
            },
            AnalysisIssue {
                code: "analysis.fixture".to_owned(),
                reason: "fixture issue".to_owned(),
                affected_scope: "crate::application".to_owned(),
            },
        ],
        ValidationDecision::Indeterminate,
        serialization,
    )
}

fn rebuild_with_reversed_inputs(mut input: ValidationReport) -> ValidationReport {
    input.analysis_context.features.reverse();
    input.analysis_context.cfg.reverse();
    input.modules.reverse();
    input.module_declarations.reverse();
    input.dependencies.reverse();
    input.excluded_scopes.reverse();
    input.violations.reverse();
    input.analysis_issues.reverse();

    report::build(
        ObservedDependencyGraph {
            analysis_context: input.analysis_context,
            modules: input.modules,
            module_declarations: input.module_declarations,
            dependencies: input.dependencies,
            excluded_scopes: input.excluded_scopes,
        },
        input.violations,
        input.policy_evaluation,
        input.analysis_issues,
        input.decision,
        &input.serialization,
    )
}

fn validate_empty_policy(fixture: &TempFixture) -> ValidationReport {
    validate_empty_policy_result(fixture).expect("validation should produce a report")
}

fn validate_empty_policy_result(
    fixture: &TempFixture,
) -> a3_rust_cruiser::error::Result<ValidationReport> {
    boundary_validation::validate(
        fixture.path(),
        analysis_context(fixture.package()),
        &fixture.join("policy.yaml"),
        "json",
    )
}

fn validate_target(
    fixture: &TempFixture,
    target: &str,
) -> a3_rust_cruiser::error::Result<ValidationReport> {
    let mut context = analysis_context(fixture.package());
    context.compilation_target = target.to_owned();
    boundary_validation::validate(
        fixture.path(),
        context,
        &fixture.join("policy.yaml"),
        "json",
    )
}

fn assert_indeterminate_issue(result: &ValidationReport, code: &str) {
    assert_eq!(result.decision, ValidationDecision::Indeterminate);
    let issue = result
        .analysis_issues
        .iter()
        .find(|issue| issue.code == code)
        .expect("expected analysis issue should be present");
    assert!(!issue.reason.is_empty());
    assert!(!issue.affected_scope.is_empty());
}

fn run_check(fixture: &TempFixture, format: &str) -> Output {
    let target = format!("bin:{}", fixture.package());
    Command::new(env!("CARGO_BIN_EXE_a3-rust-cruiser"))
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
            &target,
            "--target-triple",
            "x86_64-unknown-linux-gnu",
            "--format",
            format,
        ])
        .output()
        .expect("binary should run")
}

fn assert_completed_output(output: &Output, exit_code: i32, decision: &str) {
    assert_eq!(output.status.code(), Some(exit_code));
    assert!(!output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let document: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should contain a JSON report");
    assert_eq!(document["decision"], decision);
}

struct TempFixture {
    root: PathBuf,
    package: String,
}

impl TempFixture {
    fn new(label: &str) -> Self {
        let sequence = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let package = format!("{label}-{sequence}");
        let root = std::env::temp_dir().join(format!(
            "a3-rust-cruiser-contract-{}-{package}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).expect("fixture directory should be created");
        fs::write(
            root.join("Cargo.toml"),
            format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        )
        .expect("fixture manifest should be written");
        fs::write(root.join("policy.yaml"), "rules: []\n")
            .expect("fixture policy should be written");
        Self { root, package }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn package(&self) -> &str {
        &self.package
    }

    fn join(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    fn write(&self, path: &str, content: &str) {
        let destination = self.root.join(path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).expect("fixture parent directory should be created");
        }
        fs::write(destination, content).expect("fixture file should be written");
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
