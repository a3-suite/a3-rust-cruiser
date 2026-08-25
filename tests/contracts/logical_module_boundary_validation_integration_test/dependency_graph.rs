// contract_id: contract.logical-module-boundary.dependency-graph
// integration_id: logical-module-boundary-dependency-graph

use super::*;

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
    assert!(dependency_exists(
        &report,
        "crate::outer::inner",
        "crate::outer::sibling",
        "import",
        "internal-module"
    ));
    assert!(dependency_exists(
        &report,
        "crate::outer::inner",
        "crate::shared",
        "path-reference",
        "internal-module"
    ));
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
    let fixture = policy_sensitive_fixture("source-union-target-cfg-dependencies");
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
    assert_issue(&binary_report, "analysis.unsupported-name-resolution");

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
    assert_issue(&test_report, "analysis.unsupported-name-resolution");
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
        assert_eq!(
            report["analysis_context"]["analysis_profile"],
            "compilation"
        );
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
    let reports: Vec<_> = targets
        .iter()
        .map(|target| completed_json(&run_check_with(&fixture, target, "json", &[]), 0))
        .collect();

    // Assert
    for (target, report) in targets.iter().zip(reports) {
        assert_eq!(report["analysis_context"]["compilation_target"], *target);
        assert_eq!(
            report["analysis_context"]["analysis_profile"],
            "compilation"
        );
    }
}

#[test]
fn required_feature_activates_the_selected_target() {
    // Arrange
    let fixture = TempFixture::new("active-target-feature");
    fixture.write_manifest(
        "[[bin]]\nname = \"tool\"\npath = \"src/bin/tool.rs\"\nrequired-features = [\"cli\"]\n",
    );
    fixture.write("src/bin/tool.rs", "fn main() {}\n");

    // Act
    let output = run_check_with(&fixture, "bin:tool", "json", &["--feature", "cli"]);

    // Assert
    let report = completed_json(&output, 0);
    assert_eq!(
        report["analysis_context"]["features"],
        serde_json::json!(["cli"])
    );
    assert_eq!(report["analysis_context"]["compilation_target"], "bin:tool");
    assert_eq!(
        report["analysis_context"]["analysis_profile"],
        "compilation"
    );
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
    assert!(
        report["module_declarations"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| {
                item["parent"] == "crate::outer::inline"
                    && item["child"] == "crate::outer::inline::nested"
            }))
    );
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
    let fixture = policy_sensitive_fixture("declared-items");
    fixture.write(
        "src/main.rs",
        "mod domain;\nfn consume(_: crate::domain::Model) {}\nfn main() {}\n",
    );
    fixture.write(
        "src/domain.rs",
        "pub const VALUE: usize = 1;\npub enum Choice { One }\nextern crate core as core_alias;\npub fn make() {}\nmacro_rules! named { () => {} }\npub mod nested {}\npub static STATE: usize = 1;\npub struct Model;\npub trait Contract {}\npub trait Alias = Contract;\npub type Identifier = usize;\npub union Number { value: usize }\nuse core::fmt::{self, Debug as DebugAlias, *};\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 0);

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
fn dev_dependency_is_available_to_test_target() {
    // Arrange
    let fixture = TempFixture::new("test-target-dependencies");
    fixture.write_manifest("[dev-dependencies]\ntest-only = \"1\"\n");
    fixture.write("tests/contract.rs", "use test_only;\nfn contract() {}\n");

    // Act
    let report = completed_json(&run_check_with(&fixture, "test:contract", "json", &[]), 0);

    // Assert
    assert_eq!(
        report["analysis_context"]["compilation_target"],
        "test:contract"
    );
    assert!(dependency_exists(
        &report,
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
    assert_eq!(
        report["analysis_context"]["target_triple"],
        "x86_64-unknown-linux-gnu"
    );
    assert!(dependency_exists(
        &report,
        "crate",
        "target_only",
        "import",
        "external-crate"
    ));
}
