use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::Value;

static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

mod logical_module_boundary_validation_integration_test {
    use super::*;

    mod boundary_policy_input;
    mod cli_outcome;
    mod dependency_graph;
    mod policy_evaluation;
    mod validation_report;
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

fn policy_sensitive_fixture(label: &str) -> TempFixture {
    let fixture = TempFixture::new(label);
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: possible-crate-cycle\n    kind: circular\n    severity: error\n    modules: [crate, 'crate::**']\n",
    );
    fixture
}

fn write_resolved_implementation_attribution(fixture: &TempFixture) {
    fixture.write(
        "resolved-implementation-attribution.sdd.yml",
        &resolved_implementation_attribution(fixture),
    );
}

fn resolved_implementation_attribution(_fixture: &TempFixture) -> String {
    "version: 1\nname: resolved-implementation-attribution\nsource_provenance:\n  producer: owner-resolved-current-family\n  artifacts:\n    - dsl_name: logical-structure\n      version: 2\n      artifact_identity:\n        axes:\n          scope_path: fixture\n      sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n    - dsl_name: projection\n      version: 2\n      artifact_identity:\n        axes:\n          scope_path: fixture\n      sha256: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n    - dsl_name: physical-structure\n      version: 1\n      artifact_identity:\n        axes: {}\n      sha256: cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\nfile_attributions:\n  - observed_file_path: src/main.rs\n    canonical_scope: fixture\n    component_id: application\n    element_id: main\ncomponent_dependencies: []\nscope_dependencies: []\n"
        .to_owned()
}

fn forbidden_fixture(label: &str, with_issue: bool) -> TempFixture {
    let fixture = dependency_fixture(label, "infrastructure");
    if with_issue {
        fixture.write(
            "src/main.rs",
            "mod application;\nmod infrastructure;\nfn main() { custom_macro!(); }\n",
        );
    }
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbidden-application\n    kind: forbidden\n    severity: error\n    from: [crate::application]\n    to: [crate::infrastructure]\n  - id: possible-crate-cycle\n    kind: circular\n    severity: error\n    modules: [crate, 'crate::**']\n",
    );
    fixture
}

fn circular_fixture(label: &str) -> TempFixture {
    let fixture = TempFixture::new(label);
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
    fixture
}

fn ambiguous_candidate_fixture(label: &str) -> TempFixture {
    let fixture = TempFixture::new(label);
    fixture.write(
        "src/main.rs",
        "mod left;\nmod right;\nuse crate::left::*;\nuse crate::right::*;\nstruct Model { #[serde(default = \"default_value\")] value: usize }\nfn main() {}\n",
    );
    fixture.write("src/left.rs", "pub fn default_value() -> usize { 1 }\n");
    fixture.write("src/right.rs", "pub fn default_value() -> usize { 1 }\n");
    fixture
}

fn exact_runtime_macro_definitions() -> &'static str {
    r#"
pub(crate) fn no_language_agnostic_snapshot_extractor() -> Option<crate::foundation::plugin::language_agnostic_snapshot_extractor::LanguageAgnosticSnapshotExtractor> { None }
pub(crate) fn no_contextual_snapshot_materializer() -> Option<crate::foundation::plugin::contextual_snapshot_materializer::ContextualSnapshotMaterializer> { None }

macro_rules! declare_builtin_language_runtime {
    (
        slot = $slot:ident,
        agnostic_fn = $agnostic_fn:ident,
        contextual_fn = $contextual_fn:ident,
        agnostic_for_fn = $agnostic_for_fn:ident,
        contextual_for_fn = $contextual_for_fn:ident,
        source_view_resolver = $source_view_resolver:path,
        language = $language:literal,
        provider = $provider:literal,
        agnostic_provider = $agnostic_provider:path,
        contextual_provider = $contextual_provider:path
    ) => {
        pub(crate) const LANGUAGE: &str = $language;
        pub(crate) const PROVIDER_NAME: &str = $provider;
        pub(crate) const $slot: crate::foundation::plugin::provider::LanguagePluginProviderSlot =
            crate::foundation::plugin::provider::LanguagePluginProviderSlot {
                name: PROVIDER_NAME,
                language: LANGUAGE,
                language_agnostic_snapshot_extractor_for: $agnostic_for_fn,
                contextual_snapshot_materializer_for: $contextual_for_fn,
                source_view_resolver: $source_view_resolver,
            };
        inventory::submit! { $slot }
        pub(crate) fn $agnostic_fn() -> Option<crate::foundation::plugin::language_agnostic_snapshot_extractor::LanguageAgnosticSnapshotExtractor> { $agnostic_provider() }
        pub(crate) fn $contextual_fn() -> Option<crate::foundation::plugin::contextual_snapshot_materializer::ContextualSnapshotMaterializer> { $contextual_provider() }
        fn $agnostic_for_fn(language: &str,) -> Option<crate::foundation::plugin::language_agnostic_snapshot_extractor::LanguageAgnosticSnapshotExtractor> { if language == LANGUAGE { $agnostic_fn() } else { None } }
        fn $contextual_for_fn(language: &str,) -> Option<crate::foundation::plugin::contextual_snapshot_materializer::ContextualSnapshotMaterializer> { if language == LANGUAGE { $contextual_fn() } else { None } }
    };
}

macro_rules! declare_policy_only_language_runtime {
    (
        slot = $slot:ident,
        agnostic_fn = $agnostic_fn:ident,
        contextual_fn = $contextual_fn:ident,
        agnostic_for_fn = $agnostic_for_fn:ident,
        contextual_for_fn = $contextual_for_fn:ident,
        source_view_resolver = $source_view_resolver:path,
        language = $language:literal,
        provider = $provider:literal
    ) => {
        crate::foundation::plugin::policy_only::declare_builtin_language_runtime! {
            slot = $slot,
            agnostic_fn = $agnostic_fn,
            contextual_fn = $contextual_fn,
            agnostic_for_fn = $agnostic_for_fn,
            contextual_for_fn = $contextual_for_fn,
            source_view_resolver = $source_view_resolver,
            language = $language,
            provider = $provider,
            agnostic_provider = crate::foundation::plugin::policy_only::no_language_agnostic_snapshot_extractor,
            contextual_provider = crate::foundation::plugin::policy_only::no_contextual_snapshot_materializer
        }
    };
}

pub(crate) use declare_builtin_language_runtime;
pub(crate) use declare_policy_only_language_runtime;
"#
}

fn runtime_item_macro_fixture(name: &str, definitions: &str) -> TempFixture {
    let fixture = TempFixture::new(name);
    fixture.write_manifest("[dependencies]\ninventory = \"0.3\"\n");
    fixture.write(
        "src/main.rs",
        "mod forbidden;\nmod foundation;\nfn main() {}\n",
    );
    fixture.write("src/forbidden.rs", "pub fn resolve() {}\n");
    fixture.write("src/foundation.rs", "pub(crate) mod plugin;\n");
    fixture.write(
        "src/foundation/plugin.rs",
        "pub(crate) mod contextual_snapshot_materializer;\npub(crate) mod language_agnostic_snapshot_extractor;\npub(crate) mod policy_only;\npub(crate) mod provider;\npub(crate) mod runtime;\n",
    );
    fixture.write(
        "src/foundation/plugin/contextual_snapshot_materializer.rs",
        "pub(crate) struct ContextualSnapshotMaterializer;\n",
    );
    fixture.write(
        "src/foundation/plugin/language_agnostic_snapshot_extractor.rs",
        "pub(crate) struct LanguageAgnosticSnapshotExtractor;\n",
    );
    fixture.write(
        "src/foundation/plugin/provider.rs",
        "pub(crate) struct LanguagePluginProviderSlot;\n",
    );
    fixture.write("src/foundation/plugin/policy_only.rs", definitions);
    fixture.write(
        "src/foundation/plugin/runtime.rs",
        "crate::foundation::plugin::policy_only::declare_policy_only_language_runtime! {\n    slot = SLOT,\n    agnostic_fn = agnostic,\n    contextual_fn = contextual,\n    agnostic_for_fn = agnostic_for,\n    contextual_for_fn = contextual_for,\n    source_view_resolver = crate::forbidden::resolve,\n    language = \"fixture\",\n    provider = \"fixture-provider\"\n}\npub(crate) fn provider_name() -> &'static str { PROVIDER_NAME }\n",
    );
    fixture.write(
        "policy.yaml",
        "rules:\n  - id: forbid-generated-runtime-dependency\n    kind: forbidden\n    severity: error\n    from: [crate::foundation::plugin::runtime]\n    to: [crate::forbidden]\n",
    );
    fixture
}

fn macro_expansion_attribution_fixture(label: &str, component_dependencies: &str) -> TempFixture {
    let fixture = TempFixture::new(label);
    fixture.write(
        "src/main.rs",
        "mod provider;\nmacro_rules! pull { () => { fn pulled() -> crate::provider::Thing { crate::provider::Thing } }; }\npull!();\nfn main() {}\n",
    );
    fixture.write("src/provider.rs", "pub struct Thing;\n");
    let content = resolved_implementation_attribution(&fixture).replacen(
        "component_dependencies: []",
        &format!("  - observed_file_path: src/provider.rs\n    canonical_scope: fixture\n    component_id: provider\n    element_id: provider\ncomponent_dependencies: {component_dependencies}"),
        1,
    );
    fixture.write("resolved-implementation-attribution.sdd.yml", &content);
    fixture
}

fn run_resolved_attribution_check(fixture: &TempFixture, extra_args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_a3-rust-cruiser"));
    command
        .arg("check")
        .arg(fixture.path())
        .arg("--resolved-implementation-attribution")
        .arg(fixture.join("resolved-implementation-attribution.sdd.yml"))
        .arg("--package")
        .arg(fixture.package())
        .arg("--target")
        .arg(fixture.target())
        .args([
            "--target-triple",
            "x86_64-unknown-linux-gnu",
            "--format",
            "json",
        ])
        .args(extra_args)
        .output()
        .expect("binary should run")
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

fn run_check_without_policy(
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
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON report");
    // Report-wide guarantees hold for every completed JSON report, so the contract test maps
    // credit them to each test that reads its report through this helper.
    assert_eq!(report["serialization"], "json");
    assert_eq!(report["ordering"], "canonical");
    assert_eq!(report["decision"], completed_decision(exit_code));
    assert!(matches!(
        report["analysis_context"]["analysis_profile"].as_str(),
        Some("compilation" | "source-union")
    ));
    assert!(report["dependencies"].is_array());
    assert!(report["violations"].is_array());
    assert!(report["analysis_issues"].is_array());
    report
}

fn completed_text(output: &Output, exit_code: i32) -> String {
    assert_eq!(output.status.code(), Some(exit_code));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = String::from_utf8(output.stdout.clone()).expect("text report is UTF-8");
    assert!(payload.contains("serialization: text\n"));
    assert!(payload.contains("ordering: canonical\n"));
    assert!(payload.contains(&format!("decision: {}\n", completed_decision(exit_code))));
    payload
}

// The CLI exit code of a completed check is defined by its decision, so asserting the exit code
// and the reported decision together keeps both sides of that contract observable.
fn completed_decision(exit_code: i32) -> &'static str {
    match exit_code {
        0 => "conformant",
        1 => "non-conformant",
        2 => "indeterminate",
        _ => panic!("exit code {exit_code} is not a completed check"),
    }
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
        .unwrap_or_else(|| panic!("expected issue code {code} in {issues:?}"));
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
    assert!(matches!(
        issue["observation"]["outcome"].as_str(),
        Some(
            "unresolved-dependency-candidate"
                | "dependency-neutral"
                | "analysis-context-unresolved"
                | "component-attribution-unresolved"
        )
    ));
    assert!(issue["observation"]["candidate_targets"].is_array());
    assert!(issue["observation"]["dependency_kinds"].is_array());
    assert!(matches!(
        issue["policy_impact"]["outcome"].as_str(),
        Some("none" | "possible")
    ));
    assert!(issue["policy_impact"]["rule_ids"].is_array());
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
