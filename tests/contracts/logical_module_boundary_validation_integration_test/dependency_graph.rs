// contract_id: contract.logical-module-boundary.dependency-graph
// integration_id: logical-module-boundary-dependency-graph

use super::*;

#[test]
fn should_typed_dependencies_and_declarations_are_reported() {
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
fn should_aliases_and_multiple_super_segments_are_resolved() {
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
fn should_source_union_observes_source_authored_dependencies_without_expansion_issues() {
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
fn should_glob_import_and_reexport_chain_resolve_in_both_profiles() {
    // Arrange
    let fixture = TempFixture::new("glob-import-reexport");
    fixture.write(
        "src/main.rs",
        "mod api;\nmod facade;\nmod direct;\nmod chain;\nfn main() {}\n",
    );
    fixture.write("src/api.rs", "pub struct Public;\npub trait Contract {}\n");
    fixture.write("src/facade.rs", "pub use crate::api::*;\n");
    fixture.write(
        "src/direct.rs",
        "use crate::api::*;\npub fn accept(_: Public) {}\n",
    );
    fixture.write(
        "src/chain.rs",
        "use crate::facade::*;\npub fn accept(_: Public) {}\n",
    );
    fixture.write("policy.yaml", "rules: []\n");

    // Act
    let compilation = completed_json(&run_check(&fixture, "json"), 0);
    let source_union = completed_json(
        &run_check_with(
            &fixture,
            "bin",
            "json",
            &["--analysis-profile", "source-union"],
        ),
        0,
    );

    // Assert
    for report in [&compilation, &source_union] {
        assert_eq!(report["analysis_issues"], serde_json::json!([]));
        assert_eq!(report["decision"], "conformant");
        assert!(dependency_exists(
            report,
            "crate::direct",
            "crate::api",
            "import",
            "internal-module"
        ));
        assert!(dependency_exists(
            report,
            "crate::facade",
            "crate::api",
            "re-export",
            "internal-module"
        ));
        assert!(dependency_exists(
            report,
            "crate::chain",
            "crate::facade",
            "import",
            "internal-module"
        ));
    }
}

#[test]
fn should_crate_visible_glob_reexport_resolves_in_both_profiles() {
    // Arrange
    let fixture = TempFixture::new("crate-visible-glob-reexport");
    fixture.write(
        "src/main.rs",
        "mod api;\nmod facade;\nmod direct;\nmod chain;\nfn main() {}\n",
    );
    fixture.write("src/api.rs", "pub(crate) struct CrateVisible;\n");
    fixture.write("src/facade.rs", "pub(crate) use crate::api::*;\n");
    fixture.write(
        "src/direct.rs",
        "use crate::api::*;\npub fn accept(_: CrateVisible) {}\n",
    );
    fixture.write(
        "src/chain.rs",
        "use crate::facade::*;\npub fn accept(_: CrateVisible) {}\n",
    );
    fixture.write("policy.yaml", "rules: []\n");

    // Act
    let compilation = completed_json(&run_check(&fixture, "json"), 0);
    let source_union = completed_json(
        &run_check_with(
            &fixture,
            "bin",
            "json",
            &["--analysis-profile", "source-union"],
        ),
        0,
    );

    // Assert
    for report in [&compilation, &source_union] {
        assert_eq!(report["analysis_issues"], serde_json::json!([]));
        assert_eq!(report["decision"], "conformant");
        assert!(dependency_exists(
            report,
            "crate::direct",
            "crate::api",
            "import",
            "internal-module"
        ));
        assert!(dependency_exists(
            report,
            "crate::facade",
            "crate::api",
            "re-export",
            "internal-module"
        ));
        assert!(dependency_exists(
            report,
            "crate::chain",
            "crate::facade",
            "import",
            "internal-module"
        ));
    }
}

#[test]
fn should_crate_visible_super_glob_reexport_resolves_in_both_profiles() {
    // Arrange
    let fixture = TempFixture::new("crate-visible-super-glob-reexport");
    fixture.write(
        "src/main.rs",
        "mod api;\npub(crate) use api::*;\nmod facade;\nmod consumer;\nfn main() {}\n",
    );
    fixture.write("src/api.rs", "pub(crate) struct CrateVisible;\n");
    fixture.write("src/facade.rs", "pub(crate) use super::*;\n");
    fixture.write(
        "src/consumer.rs",
        "use crate::facade::*;\npub fn accept(_: CrateVisible) {}\n",
    );
    fixture.write("policy.yaml", "rules: []\n");

    // Act
    let compilation = completed_json(&run_check(&fixture, "json"), 0);
    let source_union = completed_json(
        &run_check_with(
            &fixture,
            "bin",
            "json",
            &["--analysis-profile", "source-union"],
        ),
        0,
    );

    // Assert
    for report in [&compilation, &source_union] {
        assert_eq!(report["analysis_issues"], serde_json::json!([]));
        assert_eq!(report["decision"], "conformant");
        assert!(dependency_exists(
            report,
            "crate",
            "crate::api",
            "re-export",
            "internal-module"
        ));
        assert!(dependency_exists(
            report,
            "crate::facade",
            "crate",
            "re-export",
            "internal-module"
        ));
        assert!(dependency_exists(
            report,
            "crate::consumer",
            "crate::facade",
            "import",
            "internal-module"
        ));
    }
}

#[test]
fn should_glob_import_keeps_unresolved_names_indeterminate_in_source_union() {
    // Arrange
    let fixture = TempFixture::new("glob-unresolved-source-union");
    fixture.write(
        "src/main.rs",
        "mod api;\nmod facade;\nmod consumer;\nfn main() {}\n",
    );
    fixture.write("src/api.rs", "pub struct Public;\n");
    fixture.write("src/facade.rs", "pub use crate::api::*;\n");
    fixture.write(
        "src/consumer.rs",
        "use crate::facade::*;\npub fn accept(_: Missing) {}\n",
    );
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: glob-resolution-sensitive\n    kind: circular\n    severity: error\n    modules: [crate, 'crate::**']\n",
    );

    // Act
    let compilation = completed_json(&run_check(&fixture, "json"), 2);
    let source_union = completed_json(
        &run_check_with(
            &fixture,
            "bin",
            "json",
            &["--analysis-profile", "source-union"],
        ),
        2,
    );

    // Assert
    for report in [&compilation, &source_union] {
        assert_issue(report, "analysis.unsupported-name-resolution");
        let issue = report["analysis_issues"]
            .as_array()
            .expect("issues array")
            .iter()
            .find(|issue| issue["code"] == "analysis.unsupported-name-resolution")
            .expect("glob unresolved issue");
        assert_eq!(
            issue["observation"]["outcome"],
            "unresolved-dependency-candidate"
        );
        assert_eq!(
            issue["observation"]["dependency_kinds"],
            serde_json::json!(["path-reference"])
        );
        assert_eq!(issue["policy_impact"]["outcome"], "possible");
        assert_eq!(
            issue["policy_impact"]["rule_ids"],
            serde_json::json!(["glob-resolution-sensitive"])
        );
        assert_eq!(report["decision"], "indeterminate");
    }
}

#[test]
fn should_glob_reexport_does_not_resolve_private_items() {
    // Arrange
    let fixture = TempFixture::new("glob-private-item");
    fixture.write(
        "src/main.rs",
        "mod api;\nmod facade;\nmod consumer;\nfn main() {}\n",
    );
    fixture.write("src/api.rs", "pub struct Public;\nstruct Hidden;\n");
    fixture.write("src/facade.rs", "pub use crate::api::*;\n");
    fixture.write(
        "src/consumer.rs",
        "use crate::facade::*;\npub fn accept(_: Hidden) {}\n",
    );
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: glob-private-resolution\n    kind: circular\n    severity: error\n    modules: [crate, 'crate::**']\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 2);

    // Assert
    assert_issue(&report, "analysis.unsupported-name-resolution");
    assert_eq!(report["decision"], "indeterminate");
}

#[test]
fn should_source_union_unions_cfg_gated_alias_candidates() {
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
fn should_source_union_includes_conditional_external_dependencies() {
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
fn should_source_union_preserves_conditional_dependency_kind_boundaries() {
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
fn should_supported_compilation_targets_select_their_entry_files() {
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
fn should_conventional_compilation_target_paths_are_supported() {
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
fn should_required_feature_activates_the_selected_target() {
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
fn should_inline_nested_and_declared_external_dependencies_are_observed() {
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
fn should_leading_colon_selects_the_external_crate_namespace() {
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
fn should_declared_item_kinds_support_complete_path_resolution() {
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
fn should_dev_dependency_is_available_to_test_target() {
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
fn should_exact_target_dependency_is_active_for_the_selected_triple() {
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

#[test]
fn should_serde_function_path_observes_explicit_internal_dependency() {
    // Arrange
    let fixture = TempFixture::new("serde-explicit-internal-function-path");
    fixture.write(
        "src/main.rs",
        "mod forbidden;\nstruct Model {\n    #[serde(deserialize_with = \"crate::forbidden::decode\")]\n    value: usize,\n}\nfn main() {}\n",
    );
    fixture.write("src/forbidden.rs", "pub fn decode() -> usize { 1 }\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-serde-helper\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: [crate::forbidden]\n",
    );

    // Act
    let report = completed_json(&run_check(&fixture, "json"), 1);

    // Assert
    let dependency = report["dependencies"]
        .as_array()
        .and_then(|dependencies| {
            dependencies.iter().find(|dependency| {
                dependency["source"] == "crate"
                    && dependency["target"] == "crate::forbidden"
                    && dependency["kind"] == "path-reference"
            })
        })
        .expect("serde helper dependency");
    assert_eq!(dependency["evidence"]["line"], 3);
    assert_eq!(report["violations"][0]["rule_id"], "forbid-serde-helper");
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn should_source_union_does_not_observe_serde_function_path_dependency() {
    // Arrange
    let fixture = TempFixture::new("source-union-serde-function-path");
    fixture.write(
        "src/main.rs",
        "mod forbidden;\nstruct Model {\n    #[serde(deserialize_with = \"crate::forbidden::decode\")]\n    value: usize,\n}\nfn main() {}\n",
    );
    fixture.write("src/forbidden.rs", "pub fn decode() -> usize { 1 }\n");
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-serde-helper\n    kind: forbidden\n    severity: error\n    from: [crate]\n    to: [crate::forbidden]\n",
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
    assert_eq!(report["decision"], "conformant");
    assert!(!dependency_exists(
        &report,
        "crate",
        "crate::forbidden",
        "path-reference",
        "internal-module"
    ));
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}

#[test]
fn should_exact_policy_runtime_item_macro_observes_generated_dependencies() {
    // Arrange
    let crate_path_fixture = runtime_item_macro_fixture(
        "exact-policy-runtime-item-macro",
        exact_runtime_macro_definitions(),
    );
    let invocation = |path: &str| {
        format!(
            "{path}! {{\n    slot = SLOT,\n    agnostic_fn = agnostic,\n    contextual_fn = contextual,\n    agnostic_for_fn = agnostic_for,\n    contextual_for_fn = contextual_for,\n    source_view_resolver = crate::forbidden::resolve,\n    language = \"fixture\",\n    provider = \"fixture-provider\",\n    agnostic_provider = crate::foundation::plugin::policy_only::no_language_agnostic_snapshot_extractor,\n    contextual_provider = crate::foundation::plugin::policy_only::no_contextual_snapshot_materializer\n}}\n",
        )
    };
    let self_path_fixture = runtime_item_macro_fixture(
        "exact-self-runtime-item-macro",
        exact_runtime_macro_definitions(),
    );
    self_path_fixture.write(
        "src/foundation/plugin.rs",
        "pub(crate) mod contextual_snapshot_materializer;\npub(crate) mod language_agnostic_snapshot_extractor;\npub(crate) mod policy_only;\npub(crate) mod provider;\npub(crate) mod runtime;\npub(crate) mod self_runtime;\n",
    );
    self_path_fixture.write(
        "src/foundation/plugin/self_runtime.rs",
        &format!(
            "{}\n{}",
            exact_runtime_macro_definitions(),
            invocation("self::declare_builtin_language_runtime")
        ),
    );
    self_path_fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-generated-runtime-dependency\n    kind: forbidden\n    severity: error\n    from: [crate::foundation::plugin::self_runtime]\n    to: [crate::forbidden]\n",
    );
    let super_path_fixture = runtime_item_macro_fixture(
        "exact-super-runtime-item-macro",
        exact_runtime_macro_definitions(),
    );
    super_path_fixture.write(
        "src/foundation/plugin.rs",
        "pub(crate) mod contextual_snapshot_materializer;\npub(crate) mod language_agnostic_snapshot_extractor;\npub(crate) mod policy_only;\npub(crate) mod provider;\npub(crate) mod runtime;\npub(crate) mod runtime_support;\n",
    );
    super_path_fixture.write(
        "src/foundation/plugin/runtime_support.rs",
        &format!(
            "{}\npub(crate) mod nested;\n",
            exact_runtime_macro_definitions()
        ),
    );
    super_path_fixture.write(
        "src/foundation/plugin/runtime_support/nested.rs",
        &invocation("super::declare_builtin_language_runtime"),
    );
    super_path_fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-generated-runtime-dependency\n    kind: forbidden\n    severity: error\n    from: [crate::foundation::plugin::runtime_support::nested]\n    to: [crate::forbidden]\n",
    );
    let local_path_fixture = runtime_item_macro_fixture(
        "exact-local-runtime-item-macro",
        exact_runtime_macro_definitions(),
    );
    local_path_fixture.write(
        "src/foundation/plugin.rs",
        "pub(crate) mod contextual_snapshot_materializer;\npub(crate) mod language_agnostic_snapshot_extractor;\npub(crate) mod local_runtime;\npub(crate) mod policy_only;\npub(crate) mod provider;\npub(crate) mod runtime;\n",
    );
    local_path_fixture.write(
        "src/foundation/plugin/local_runtime.rs",
        &format!(
            "{}\n{}",
            exact_runtime_macro_definitions(),
            invocation("declare_builtin_language_runtime")
        ),
    );
    local_path_fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-generated-runtime-dependency\n    kind: forbidden\n    severity: error\n    from: [crate::foundation::plugin::local_runtime]\n    to: [crate::forbidden]\n",
    );
    let imported_path_fixture = runtime_item_macro_fixture(
        "exact-imported-runtime-item-macro",
        exact_runtime_macro_definitions(),
    );
    imported_path_fixture.write(
        "src/foundation/plugin.rs",
        "pub(crate) mod contextual_snapshot_materializer;\npub(crate) mod imported_runtime;\npub(crate) mod language_agnostic_snapshot_extractor;\npub(crate) mod policy_only;\npub(crate) mod provider;\npub(crate) mod runtime;\n",
    );
    imported_path_fixture.write(
        "src/foundation/plugin/imported_runtime.rs",
        &format!(
            "use super::policy_only::declare_builtin_language_runtime;\n{}",
            invocation("declare_builtin_language_runtime")
        ),
    );
    imported_path_fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-generated-runtime-dependency\n    kind: forbidden\n    severity: error\n    from: [crate::foundation::plugin::imported_runtime]\n    to: [crate::forbidden]\n",
    );
    let fixtures = [
        (crate_path_fixture, "crate::foundation::plugin::runtime"),
        (self_path_fixture, "crate::foundation::plugin::self_runtime"),
        (
            super_path_fixture,
            "crate::foundation::plugin::runtime_support::nested",
        ),
        (
            local_path_fixture,
            "crate::foundation::plugin::local_runtime",
        ),
        (
            imported_path_fixture,
            "crate::foundation::plugin::imported_runtime",
        ),
    ];

    // Act
    let reports = fixtures
        .iter()
        .map(|(fixture, source)| (completed_json(&run_check(fixture, "json"), 1), *source))
        .collect::<Vec<_>>();

    // Assert
    for (report, source) in reports {
        assert!(dependency_exists(
            &report,
            source,
            "crate::forbidden",
            "path-reference",
            "internal-module"
        ));
        assert!(dependency_exists(
            &report,
            source,
            "crate::foundation::plugin::provider",
            "path-reference",
            "internal-module"
        ));
        if source == "crate::foundation::plugin::runtime" {
            assert!(
                report["dependencies"]
                    .as_array()
                    .is_some_and(|dependencies| {
                        dependencies.iter().any(|dependency| {
                            dependency["source"] == source
                                && dependency["target"] == "crate::forbidden"
                                && dependency["evidence"]["line"] == 1
                                && dependency["evidence"]["column"] == 1
                        })
                    })
            );
        }
        assert_eq!(report["analysis_issues"], serde_json::json!([]));
        assert_eq!(
            report["violations"][0]["rule_id"],
            "forbid-generated-runtime-dependency"
        );
    }
}

#[test]
fn should_source_union_does_not_expand_exact_runtime_item_macro() {
    // Arrange
    let fixture = runtime_item_macro_fixture(
        "source-union-runtime-item-macro",
        exact_runtime_macro_definitions(),
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
    assert_eq!(report["decision"], "conformant");
    assert!(!dependency_exists(
        &report,
        "crate",
        "crate::forbidden",
        "path-reference",
        "internal-module"
    ));
    assert_eq!(report["analysis_issues"], serde_json::json!([]));
}
