use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

use serde::Deserialize;

use crate::boundary_validation::{
    BoundaryPolicy, BoundaryPolicyAssessment, CruiserError, LoadedBoundaryPolicy,
    NormalizedSddPolicy, NormalizedSddProvenance, NormalizedSddSourceArtifact, Result,
    ScopedComponentAttribution,
};

const ATTRIBUTION_NAME: &str = "resolved-implementation-attribution";
const ATTRIBUTION_VERSION: u64 = 1;
const PRODUCER: &str = "owner-resolved-current-family";

fn invalid_boundary_policy_input(message: impl Into<String>) -> CruiserError {
    CruiserError::InvalidInput(format!("invalid boundary policy input: {}", message.into()))
}

pub(super) fn prepare(path: &Path) -> Result<String> {
    let content = std::fs::read_to_string(path).map_err(|error| {
        CruiserError::io(
            format!("resolved implementation attribution {}", path.display()),
            error,
        )
    })?;
    parse(&content)?;
    Ok(content)
}

fn parse(content: &str) -> Result<ResolvedImplementationAttributionDto> {
    let attribution: ResolvedImplementationAttributionDto =
        serde_yaml::from_str(content).map_err(|error| {
            invalid_boundary_policy_input(format!("invalid resolved attribution YAML: {error}"))
        })?;
    validate_discriminator(&attribution)?;
    validate_provenance(&attribution.source_provenance)?;
    Ok(attribution)
}

pub(super) fn finalize(content: String, project_root: &Path) -> Result<LoadedBoundaryPolicy> {
    let attribution = parse(&content)?;

    let mut file_attributions = BTreeMap::new();
    for file_attribution in attribution.file_attributions {
        validate_non_empty("canonical scope", &file_attribution.canonical_scope)?;
        validate_non_empty("component ID", &file_attribution.component_id)?;
        validate_non_empty("element ID", &file_attribution.element_id)?;
        validate_observed_file(project_root, &file_attribution.observed_file_path)?;
        let model = ScopedComponentAttribution {
            canonical_scope: file_attribution.canonical_scope,
            component_id: file_attribution.component_id,
            element_id: file_attribution.element_id,
        };
        if file_attributions
            .insert(file_attribution.observed_file_path.clone(), model)
            .is_some()
        {
            return Err(invalid_boundary_policy_input(format!(
                "duplicate normalized attribution file: {}",
                file_attribution.observed_file_path
            )));
        }
    }

    let mut declared_component_dependencies = BTreeSet::new();
    for dependency in attribution.component_dependencies {
        validate_non_empty("component dependency scope", &dependency.canonical_scope)?;
        validate_non_empty("component dependency source", &dependency.from_component_id)?;
        validate_non_empty("component dependency target", &dependency.to_component_id)?;
        if !declared_component_dependencies.insert((
            dependency.canonical_scope,
            dependency.from_component_id,
            dependency.to_component_id,
        )) {
            return Err(invalid_boundary_policy_input(
                "resolved component dependency entries must be unique",
            ));
        }
    }
    let mut declared_scope_dependencies = BTreeSet::new();
    let mut seen_scope_dependencies = BTreeSet::new();
    for dependency in attribution.scope_dependencies {
        validate_non_empty("scope dependency source", &dependency.from_scope)?;
        validate_non_empty("scope dependency target", &dependency.to_scope)?;
        if dependency.contract_refs.is_empty() {
            return Err(invalid_boundary_policy_input(
                "normalized scope dependency requires at least one contract reference",
            ));
        }
        let mut contract_refs = BTreeSet::new();
        for contract_ref in dependency.contract_refs {
            validate_non_empty(
                "scope dependency subject reference",
                &contract_ref.subject_ref,
            )?;
            validate_non_empty("scope dependency contract ID", &contract_ref.contract_id)?;
            if !contract_refs.insert((contract_ref.subject_ref, contract_ref.contract_id)) {
                return Err(invalid_boundary_policy_input(
                    "normalized scope dependency contract references must be unique",
                ));
            }
        }
        let relation = (dependency.from_scope, dependency.to_scope);
        if !seen_scope_dependencies.insert((relation.0.clone(), relation.1.clone(), contract_refs))
        {
            return Err(invalid_boundary_policy_input(
                "resolved scope dependency entries must be unique",
            ));
        }
        declared_scope_dependencies.insert(relation);
    }

    let mut artifacts = attribution
        .source_provenance
        .artifacts
        .into_iter()
        .map(|artifact| NormalizedSddSourceArtifact {
            dsl_name: artifact.dsl_name,
            version: artifact.version,
            artifact_identity_axes: artifact.artifact_identity.axes,
            sha256: artifact.sha256,
        })
        .collect::<Vec<_>>();
    artifacts.sort();
    let provenance = NormalizedSddProvenance {
        producer: attribution.source_provenance.producer,
        artifacts,
    };
    Ok(LoadedBoundaryPolicy {
        policy: BoundaryPolicy { rules: Vec::new() },
        analysis_issues: Vec::new(),
        assessment: BoundaryPolicyAssessment::ResolvedImplementationAttribution(
            NormalizedSddPolicy {
                provenance,
                file_attributions,
                declared_component_dependencies,
                declared_scope_dependencies,
            },
        ),
    })
}

fn validate_discriminator(attribution: &ResolvedImplementationAttributionDto) -> Result<()> {
    if attribution.name != ATTRIBUTION_NAME {
        return Err(invalid_boundary_policy_input(format!(
            "unexpected resolved attribution name: {}",
            attribution.name
        )));
    }
    if attribution.version != ATTRIBUTION_VERSION {
        return Err(invalid_boundary_policy_input(format!(
            "unsupported resolved attribution version: {}",
            attribution.version
        )));
    }
    Ok(())
}

fn validate_provenance(provenance: &SourceProvenanceDto) -> Result<()> {
    if provenance.producer != PRODUCER {
        return Err(invalid_boundary_policy_input(format!(
            "unsupported normalized family producer: {}",
            provenance.producer
        )));
    }
    let expected_versions = BTreeMap::from([
        ("logical-structure", 2),
        ("projection", 2),
        ("physical-structure", 1),
    ]);
    let mut identities = BTreeSet::new();
    let mut counts = BTreeMap::new();
    for artifact in &provenance.artifacts {
        let Some(expected_version) = expected_versions.get(artifact.dsl_name.as_str()) else {
            return Err(invalid_boundary_policy_input(format!(
                "unsupported provenance DSL name: {}",
                artifact.dsl_name
            )));
        };
        if artifact.version != *expected_version {
            return Err(invalid_boundary_policy_input(format!(
                "unsupported provenance version for {}: {}",
                artifact.dsl_name, artifact.version
            )));
        }
        for (key, value) in &artifact.artifact_identity.axes {
            if !is_valid_axis_key(key) || value.is_empty() {
                return Err(invalid_boundary_policy_input(format!(
                    "invalid provenance artifact identity axis for {}: {key}",
                    artifact.dsl_name
                )));
            }
        }
        match artifact.dsl_name.as_str() {
            "logical-structure" | "projection" => {
                if artifact
                    .artifact_identity
                    .axes
                    .get("scope_path")
                    .is_none_or(String::is_empty)
                {
                    return Err(invalid_boundary_policy_input(format!(
                        "scoped provenance artifact requires scope_path: {}",
                        artifact.dsl_name
                    )));
                }
            }
            "physical-structure" if !artifact.artifact_identity.axes.is_empty() => {
                return Err(invalid_boundary_policy_input(
                    "global physical provenance artifact requires empty axes",
                ));
            }
            _ => {}
        }
        if !identities.insert((artifact.dsl_name.as_str(), &artifact.artifact_identity.axes)) {
            return Err(invalid_boundary_policy_input(format!(
                "duplicate provenance artifact identity: {}",
                artifact.dsl_name
            )));
        }
        if artifact.sha256.len() != 64
            || !artifact
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(invalid_boundary_policy_input(format!(
                "invalid lowercase SHA-256 for provenance contract: {}",
                artifact.dsl_name
            )));
        }
        *counts.entry(artifact.dsl_name.as_str()).or_insert(0_usize) += 1;
    }
    if counts.get("logical-structure").copied().unwrap_or_default() < 1
        || counts.get("projection").copied().unwrap_or_default() < 1
        || counts.get("physical-structure").copied() != Some(1)
    {
        return Err(invalid_boundary_policy_input(
            "normalized family provenance requires scoped logical and projection artifacts and exactly one global physical artifact",
        ));
    }
    Ok(())
}

fn is_valid_axis_key(value: &str) -> bool {
    let mut segments = value.split('_');
    let Some(first) = segments.next() else {
        return false;
    };
    let valid_first = first
        .bytes()
        .enumerate()
        .all(|(index, byte)| byte.is_ascii_lowercase() || (index > 0 && byte.is_ascii_digit()));
    !first.is_empty()
        && valid_first
        && segments.all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn validate_observed_file(project_root: &Path, path: &str) -> Result<()> {
    let relative = Path::new(path);
    let bytes = path.as_bytes();
    let has_windows_drive_prefix =
        matches!(bytes, [drive, b':', ..] if drive.is_ascii_alphabetic());
    let violates_owner_path_contract = path.chars().next().is_some_and(char::is_whitespace)
        || path.chars().last().is_some_and(char::is_whitespace)
        || path
            .chars()
            .any(|character| character.is_control() || "\\*?[]{}".contains(character))
        || path.contains("//")
        || path.ends_with('/')
        || path.split('/').any(|segment| matches!(segment, "." | ".."));
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || has_windows_drive_prefix
        || violates_owner_path_contract
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid_boundary_policy_input(format!(
            "observed file path must be normalized below the project root: {path}"
        )));
    }
    let mut current = project_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            unreachable!("validated normal path component")
        };
        current.push(segment);
        let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
            invalid_boundary_policy_input(format!(
                "observed file path is unavailable: {path}: {error}"
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(invalid_boundary_policy_input(format!(
                "observed file path contains a symlink: {path}"
            )));
        }
    }
    if !current.is_file() {
        return Err(invalid_boundary_policy_input(format!(
            "observed file path is not a file: {path}"
        )));
    }
    Ok(())
}

fn validate_non_empty(kind: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(invalid_boundary_policy_input(format!(
            "{kind} must not be empty"
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolvedImplementationAttributionDto {
    version: u64,
    name: String,
    source_provenance: SourceProvenanceDto,
    file_attributions: Vec<FileAttributionDto>,
    component_dependencies: Vec<ComponentDependencyDto>,
    scope_dependencies: Vec<ScopeDependencyDto>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceProvenanceDto {
    producer: String,
    artifacts: Vec<SourceArtifactDto>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceArtifactDto {
    dsl_name: String,
    version: u64,
    artifact_identity: ArtifactIdentityDto,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactIdentityDto {
    axes: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileAttributionDto {
    observed_file_path: String,
    canonical_scope: String,
    component_id: String,
    element_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentDependencyDto {
    canonical_scope: String,
    from_component_id: String,
    to_component_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeDependencyDto {
    from_scope: String,
    to_scope: String,
    contract_refs: Vec<ContractRefDto>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractRefDto {
    subject_ref: String,
    contract_id: String,
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::{finalize, prepare, validate_observed_file};
    use crate::{
        boundary_policy_input::ProjectBoundaryPolicyLoader,
        boundary_validation::{
            AnalysisContext, AnalysisIssue, AnalysisProfile, BoundaryPolicyAssessment,
            BoundaryPolicyInput, Dependency, DependencyKind, DependencyObserver,
            LoadedBoundaryPolicy, LogicalModule, NormalizedSddPolicy, ObservedDependencyGraph,
            Result, SourceEvidence, TargetScope, ValidationDecision, validate,
        },
    };

    static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn should_accept_file_keyed_attributions_without_producer_module_coverage() {
        // Arrange
        let fixture = Fixture::new("file-keyed");
        fixture.write("src/application.rs", "pub fn run() {}\n");
        fixture.write("src/provider.rs", "pub struct Provider;\n");
        fixture.write("src/unused.rs", "pub fn unused() {}\n");
        fixture.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "component_dependencies: []",
                "  - observed_file_path: src/unused.rs\n    canonical_scope: scope-a\n    component_id: unused\n    element_id: unused-file\ncomponent_dependencies: []",
            ),
        );

        // Act
        let result = finalize(
            prepare(&fixture.path("normalized.sdd.yml")).expect("structurally valid"),
            fixture.root(),
        );

        // Assert
        assert_eq!(
            resolved_policy(result.expect("file-keyed normalized policy"))
                .file_attributions
                .len(),
            3
        );
    }

    #[test]
    fn should_derive_all_selected_modules_as_unresolved_from_an_empty_file_table() {
        // Arrange
        let fixture = Fixture::new("empty-file-table");
        fixture.write("src/application.rs", "pub fn run() {}\n");
        fixture.write("src/provider.rs", "pub struct Provider;\n");
        fixture.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "file_attributions:\n  - observed_file_path: src/application.rs\n    canonical_scope: scope-a\n    component_id: application\n    element_id: application-file\n  - observed_file_path: src/provider.rs\n    canonical_scope: scope-b\n    component_id: provider\n    element_id: provider-file",
                "file_attributions: []",
            ),
        );

        // Act
        let result = validate(
            &StaticObserver,
            &ProjectBoundaryPolicyLoader,
            fixture.root(),
            context(),
            &BoundaryPolicyInput::ResolvedImplementationAttribution {
                path: fixture.path("normalized.sdd.yml"),
            },
        )
        .expect("empty file table is a valid transient input");

        // Assert
        assert_eq!(result.decision, ValidationDecision::Indeterminate);
        let coverage = result.normalized_sdd_coverage.expect("derived coverage");
        assert!(coverage.covered_modules.is_empty());
        assert_eq!(coverage.unresolved_modules.len(), 2);
    }

    #[test]
    fn should_validate_normalized_scope_edges_and_return_coverage() {
        // Arrange
        let fixture = Fixture::new("validation-route");
        fixture.write("src/application.rs", "pub fn run() {}\n");
        fixture.write("src/provider.rs", "pub struct Provider;\n");
        fixture.write("normalized.sdd.yml", &valid_envelope());

        // Act
        let result = validate(
            &StaticObserver,
            &ProjectBoundaryPolicyLoader,
            fixture.root(),
            context(),
            &BoundaryPolicyInput::ResolvedImplementationAttribution {
                path: fixture.path("normalized.sdd.yml"),
            },
        )
        .expect("normalized validation");

        // Assert
        assert_eq!(result.decision, ValidationDecision::Conformant);
        assert!(result.violations.is_empty());
        assert_eq!(
            result
                .normalized_sdd_coverage
                .expect("normalized coverage")
                .covered_modules
                .len(),
            2
        );
    }

    #[test]
    fn should_keep_normalized_unresolved_attribution_indeterminate() {
        // Arrange
        let fixture = Fixture::new("unresolved-validation-route");
        fixture.write("src/application.rs", "pub fn run() {}\n");
        fixture.write("src/provider.rs", "pub struct Provider;\n");
        fixture.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "  - observed_file_path: src/provider.rs\n    canonical_scope: scope-b\n    component_id: provider\n    element_id: provider-file\n",
                "",
            ),
        );

        // Act
        let result = validate(
            &StaticObserver,
            &ProjectBoundaryPolicyLoader,
            fixture.root(),
            context(),
            &BoundaryPolicyInput::ResolvedImplementationAttribution {
                path: fixture.path("normalized.sdd.yml"),
            },
        )
        .expect("normalized unresolved validation");

        // Assert
        assert_eq!(result.decision, ValidationDecision::Indeterminate);
        assert_eq!(result.analysis_issues.len(), 1);
    }

    #[test]
    fn should_reject_normalized_undeclared_scope_edge_as_non_conformant() {
        // Arrange
        let fixture = Fixture::new("forbidden-validation-route");
        fixture.write("src/application.rs", "pub fn run() {}\n");
        fixture.write("src/provider.rs", "pub struct Provider;\n");
        fixture.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "scope_dependencies:\n  - from_scope: scope-a\n    to_scope: scope-b\n    contract_refs:\n      - subject_ref: provider\n        contract_id: contract.example",
                "scope_dependencies: []",
            ),
        );

        // Act
        let result = validate(
            &StaticObserver,
            &ProjectBoundaryPolicyLoader,
            fixture.root(),
            context(),
            &BoundaryPolicyInput::ResolvedImplementationAttribution {
                path: fixture.path("normalized.sdd.yml"),
            },
        )
        .expect("normalized forbidden validation");

        // Assert
        assert_eq!(result.decision, ValidationDecision::NonConformant);
        assert_eq!(result.violations.len(), 1);
    }

    #[test]
    fn should_reject_unknown_fields_and_the_legacy_consumer_envelope() {
        // Arrange
        let unknown = Fixture::new("unknown-field");
        unknown.write(
            "normalized.sdd.yml",
            &valid_envelope().replace("version: 1\n", "version: 1\nunknown: true\n"),
        );
        let legacy_envelope = Fixture::new("legacy-consumer-envelope");
        legacy_envelope.write(
            "normalized.sdd.yml",
            &valid_envelope().replace("source_provenance:", "contracts: {}\nsource_provenance:"),
        );

        // Act
        let unknown_result = load_fixture(&unknown);
        let legacy_envelope_result = load_fixture(&legacy_envelope);

        // Assert
        assert!(unknown_result.is_err());
        assert!(legacy_envelope_result.is_err());
    }

    #[test]
    fn should_reject_unsupported_envelope_or_invalid_provenance() {
        // Arrange
        let unsupported = Fixture::new("unsupported-envelope");
        unsupported.write(
            "normalized.sdd.yml",
            &valid_envelope().replacen("version: 1", "version: 2", 1),
        );
        let invalid_producer = Fixture::new("invalid-producer");
        invalid_producer.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "producer: owner-resolved-current-family",
                "producer: untrusted-family",
            ),
        );
        let missing_artifacts = Fixture::new("missing-artifacts");
        missing_artifacts.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "    - dsl_name: physical-structure\n      version: 1\n      artifact_identity:\n        axes: {}\n      sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
                "",
            ),
        );
        let duplicate_identity = Fixture::new("duplicate-provenance-identity");
        duplicate_identity.write(
            "normalized.sdd.yml",
            &valid_envelope().replace("scope_path: scope-b", "scope_path: scope-a"),
        );
        let legacy_family_id = Fixture::new("legacy-family-id");
        legacy_family_id.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "producer: owner-resolved-current-family",
                "producer: owner-resolved-current-family\n  family_id: fixture-family",
            ),
        );
        let missing_file_table = Fixture::new("missing-file-table");
        missing_file_table.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "file_attributions:\n  - observed_file_path: src/application.rs\n    canonical_scope: scope-a\n    component_id: application\n    element_id: application-file\n  - observed_file_path: src/provider.rs\n    canonical_scope: scope-b\n    component_id: provider\n    element_id: provider-file\n",
                "",
            ),
        );

        // Act
        let results = [
            load_fixture(&unsupported),
            load_fixture(&invalid_producer),
            load_fixture(&missing_artifacts),
            load_fixture(&duplicate_identity),
            load_fixture(&legacy_family_id),
            load_fixture(&missing_file_table),
        ];

        // Assert
        assert!(results.into_iter().all(|result| result.is_err()));
    }

    #[test]
    fn should_reject_invalid_provenance_versions_and_axes() {
        // Arrange
        let cases = [
            valid_envelope().replacen(
                "dsl_name: logical-structure\n      version: 2",
                "dsl_name: logical-structure\n      version: 1",
                1,
            ),
            valid_envelope().replacen("          scope_path: scope-a\n", "", 1),
            valid_envelope().replace(
                "artifact_identity:\n        axes: {}",
                "artifact_identity:\n        axes:\n          scope_path: global",
            ),
            valid_envelope().replacen("scope_path: scope-a", "ScopePath: scope-a", 1),
            valid_envelope().replacen("scope_path: scope-a", "1scope_path: scope-a", 1),
            valid_envelope().replacen("scope_path: scope-a", "scope__path: scope-a", 1),
            valid_envelope().replacen("scope_path: scope-a", "scope-path: scope-a", 1),
            valid_envelope().replacen("scope_path: scope-a", "scope_path: ''", 1),
            valid_envelope().replacen(&"a".repeat(64), "abc", 1),
            valid_envelope().replacen(&"a".repeat(64), &"A".repeat(64), 1),
            valid_envelope().replacen(&"a".repeat(64), &"g".repeat(64), 1),
        ];

        let fixtures = cases.into_iter().enumerate().map(|(index, content)| {
            let fixture = Fixture::new(&format!("invalid-provenance-{index}"));
            fixture.write("normalized.sdd.yml", &content);
            fixture
        });

        // Act
        let results = fixtures
            .map(|fixture| load_fixture(&fixture))
            .collect::<Vec<_>>();

        // Assert
        assert!(results.into_iter().all(|result| result.is_err()));
    }

    #[test]
    fn should_reject_duplicate_attribution() {
        // Arrange
        let duplicate = Fixture::new("duplicate-attribution");
        duplicate.write("src/application.rs", "pub fn run() {}\n");
        duplicate.write("src/provider.rs", "pub struct Provider;\n");
        duplicate.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "observed_file_path: src/provider.rs",
                "observed_file_path: src/application.rs",
            ),
        );
        // Act
        let duplicate_result = finalize(
            prepare(&duplicate.path("normalized.sdd.yml")).expect("structurally valid"),
            duplicate.root(),
        );

        // Assert
        assert!(duplicate_result.is_err());
    }

    #[test]
    fn should_reject_duplicate_dependency_entries() {
        // Arrange
        let component = Fixture::new("duplicate-component-dependency");
        let scope = Fixture::new("duplicate-scope-dependency");
        for fixture in [&component, &scope] {
            fixture.write("src/application.rs", "pub fn run() {}\n");
            fixture.write("src/provider.rs", "pub struct Provider;\n");
        }
        component.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "component_dependencies: []",
                "component_dependencies:\n  - canonical_scope: scope-a\n    from_component_id: application\n    to_component_id: provider\n  - canonical_scope: scope-a\n    from_component_id: application\n    to_component_id: provider",
            ),
        );
        let scope_dependency = "  - from_scope: scope-a\n    to_scope: scope-b\n    contract_refs:\n      - subject_ref: provider\n        contract_id: contract.example";
        scope.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                scope_dependency,
                &format!("{scope_dependency}\n{scope_dependency}"),
            ),
        );

        // Act
        let component_result = finalize(
            prepare(&component.path("normalized.sdd.yml")).expect("structurally valid"),
            component.root(),
        );
        let scope_result = finalize(
            prepare(&scope.path("normalized.sdd.yml")).expect("structurally valid"),
            scope.root(),
        );

        // Assert
        assert!(component_result.is_err());
        assert!(scope_result.is_err());
    }

    #[test]
    fn should_reject_empty_required_attribution_values() {
        // Arrange
        let envelope = valid_envelope();
        let cases = [
            envelope.replace("canonical_scope: scope-a", "canonical_scope: ''"),
            envelope.replace("component_id: application", "component_id: ''"),
            envelope.replace("element_id: application-file", "element_id: ''"),
            envelope.replace(
                "component_dependencies: []",
                "component_dependencies:\n  - canonical_scope: ''\n    from_component_id: application\n    to_component_id: provider",
            ),
            envelope.replace(
                "component_dependencies: []",
                "component_dependencies:\n  - canonical_scope: scope-a\n    from_component_id: ''\n    to_component_id: provider",
            ),
            envelope.replace(
                "component_dependencies: []",
                "component_dependencies:\n  - canonical_scope: scope-a\n    from_component_id: application\n    to_component_id: ''",
            ),
            envelope.replace("from_scope: scope-a", "from_scope: ''"),
            envelope.replace("to_scope: scope-b", "to_scope: ''"),
            envelope.replace("subject_ref: provider", "subject_ref: ''"),
            envelope.replace("contract_id: contract.example", "contract_id: ''"),
        ];
        let fixtures = cases.into_iter().enumerate().map(|(index, content)| {
            let fixture = Fixture::new(&format!("empty-required-value-{index}"));
            fixture.write("src/application.rs", "pub fn run() {}\n");
            fixture.write("src/provider.rs", "pub struct Provider;\n");
            fixture.write("normalized.sdd.yml", &content);
            fixture
        });

        // Act
        let results = fixtures
            .map(|fixture| {
                finalize(
                    prepare(&fixture.path("normalized.sdd.yml")).expect("structurally valid"),
                    fixture.root(),
                )
            })
            .collect::<Vec<_>>();

        // Assert
        assert!(results.into_iter().all(|result| result.is_err()));
    }

    #[test]
    fn should_reject_missing_or_root_escape_observed_paths() {
        // Arrange
        let missing = Fixture::new("missing-path");
        missing.write("src/provider.rs", "pub struct Provider;\n");
        missing.write("normalized.sdd.yml", &valid_envelope());
        let escaped = Fixture::new("escaped-path");
        escaped.write("src/provider.rs", "pub struct Provider;\n");
        escaped.write(
            "normalized.sdd.yml",
            &valid_envelope().replace("src/application.rs", "../application.rs"),
        );
        // Act
        let results = [&missing, &escaped].map(|fixture| {
            finalize(
                prepare(&fixture.path("normalized.sdd.yml")).expect("structurally valid"),
                fixture.root(),
            )
        });

        // Assert
        assert!(results.into_iter().all(|result| result.is_err()));
    }

    #[test]
    fn should_reject_observed_paths_outside_the_owner_schema() {
        // Arrange
        let fixture = Fixture::new("invalid-owner-path");
        fixture.write("src/application.rs", "pub fn run() {}\n");
        let invalid_paths = [
            "C:/src/application.rs",
            " src/application.rs",
            "src/application.rs ",
            "src//application.rs",
            "src/./application.rs",
            "src/application?.rs",
            "src\\application.rs",
        ];

        // Act
        let results = invalid_paths.map(|path| validate_observed_file(fixture.root(), path));

        // Assert
        assert!(results.into_iter().all(|result| matches!(
            result,
            Err(error)
                if error
                    .to_string()
                    .contains("observed file path must be normalized below the project root")
        )));
    }

    #[test]
    fn should_consume_owner_selected_nested_attribution_without_inheriting_ownership() {
        // Arrange
        let fixture = Fixture::new("nested-attribution");
        fixture.write("src/application/service.rs", "pub fn run() {}\n");
        fixture.write("src/provider.rs", "pub struct Provider;\n");
        fixture.write(
            "normalized.sdd.yml",
            &valid_envelope().replace("src/application.rs", "src/application/service.rs"),
        );

        // Act
        let loaded = finalize(
            prepare(&fixture.path("normalized.sdd.yml")).expect("structurally valid"),
            fixture.root(),
        )
        .expect("owner-selected nested attribution");

        // Assert
        let normalized = resolved_policy(loaded);
        assert!(
            normalized
                .file_attributions
                .contains_key("src/application/service.rs")
        );
    }

    #[test]
    fn should_keep_contract_refs_trace_only() {
        // Arrange
        let first = Fixture::new("contract-ref-a");
        let second = Fixture::new("contract-ref-b");
        for fixture in [&first, &second] {
            fixture.write("src/application.rs", "pub fn run() {}\n");
            fixture.write("src/provider.rs", "pub struct Provider;\n");
        }
        first.write("normalized.sdd.yml", &valid_envelope());
        second.write(
            "normalized.sdd.yml",
            &valid_envelope().replace("contract.example", "contract.changed"),
        );

        // Act
        let first = resolved_policy(
            finalize(
                prepare(&first.path("normalized.sdd.yml")).expect("first envelope"),
                first.root(),
            )
            .expect("first policy"),
        );
        let second = resolved_policy(
            finalize(
                prepare(&second.path("normalized.sdd.yml")).expect("second envelope"),
                second.root(),
            )
            .expect("second policy"),
        );

        // Assert
        assert_eq!(
            first.declared_scope_dependencies,
            second.declared_scope_dependencies
        );
    }

    #[test]
    fn should_reject_legacy_string_contract_refs() {
        // Arrange
        let fixture = Fixture::new("legacy-contract-ref");
        fixture.write(
            "normalized.sdd.yml",
            &valid_envelope().replace(
                "contract_refs:\n      - subject_ref: provider\n        contract_id: contract.example",
                "contract_refs: [contract.example]",
            ),
        );

        // Act
        let result = load_fixture(&fixture);

        // Assert
        assert!(result.is_err());
    }

    fn load_fixture(fixture: &Fixture) -> Result<LoadedBoundaryPolicy> {
        finalize(
            prepare(&fixture.path("normalized.sdd.yml"))?,
            fixture.root(),
        )
    }

    fn resolved_policy(loaded: LoadedBoundaryPolicy) -> NormalizedSddPolicy {
        match loaded.assessment {
            BoundaryPolicyAssessment::ResolvedImplementationAttribution(policy) => policy,
            _ => panic!("resolved implementation attribution policy"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn should_reject_an_observed_path_with_a_symlink_component() {
        // Arrange
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("symlink");
        fixture.write("real/application.rs", "pub fn run() {}\n");
        fixture.write("src/provider.rs", "pub struct Provider;\n");
        fs::create_dir_all(fixture.path("src")).expect("src directory");
        symlink(
            fixture.path("real/application.rs"),
            fixture.path("src/application.rs"),
        )
        .expect("source symlink");
        fixture.write("normalized.sdd.yml", &valid_envelope());

        // Act
        let result = finalize(
            prepare(&fixture.path("normalized.sdd.yml")).expect("structurally valid"),
            fixture.root(),
        );

        // Assert
        assert!(result.is_err());
    }

    fn valid_envelope() -> String {
        "version: 1\nname: resolved-implementation-attribution\nsource_provenance:\n  producer: owner-resolved-current-family\n  artifacts:\n    - dsl_name: logical-structure\n      version: 2\n      artifact_identity:\n        axes:\n          scope_path: scope-a\n      sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n    - dsl_name: logical-structure\n      version: 2\n      artifact_identity:\n        axes:\n          scope_path: scope-b\n      sha256: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n    - dsl_name: physical-structure\n      version: 1\n      artifact_identity:\n        axes: {}\n      sha256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n    - dsl_name: projection\n      version: 2\n      artifact_identity:\n        axes:\n          scope_path: scope-a\n      sha256: cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\n    - dsl_name: projection\n      version: 2\n      artifact_identity:\n        axes:\n          scope_path: scope-b\n      sha256: dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\nfile_attributions:\n  - observed_file_path: src/application.rs\n    canonical_scope: scope-a\n    component_id: application\n    element_id: application-file\n  - observed_file_path: src/provider.rs\n    canonical_scope: scope-b\n    component_id: provider\n    element_id: provider-file\ncomponent_dependencies: []\nscope_dependencies:\n  - from_scope: scope-a\n    to_scope: scope-b\n    contract_refs:\n      - subject_ref: provider\n        contract_id: contract.example\n"
            .to_owned()
    }

    fn context() -> AnalysisContext {
        AnalysisContext {
            analysis_profile: AnalysisProfile::Compilation,
            package: "fixture".to_owned(),
            compilation_target: "bin:fixture".to_owned(),
            features: vec!["feature-a".to_owned()],
            cfg: vec!["unix".to_owned()],
            target_triple: "x86_64-unknown-linux-gnu".to_owned(),
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "a3-rust-cruiser-normalized-sdd-{}-{label}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("fixture root");
            Self { root }
        }

        fn root(&self) -> &Path {
            &self.root
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }

        fn write(&self, relative: &str, content: &str) {
            let path = self.path(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("fixture parent");
            }
            fs::write(path, content).expect("fixture content");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    struct StaticObserver;

    impl DependencyObserver for StaticObserver {
        fn observe(
            &self,
            _project_root: &Path,
            context: AnalysisContext,
        ) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
            Ok((
                ObservedDependencyGraph {
                    analysis_context: context,
                    modules: vec![
                        LogicalModule {
                            id: "crate::application".to_owned(),
                            source_paths: vec!["src/application.rs".to_owned()],
                        },
                        LogicalModule {
                            id: "crate::provider".to_owned(),
                            source_paths: vec!["src/provider.rs".to_owned()],
                        },
                    ],
                    module_declarations: Vec::new(),
                    dependencies: vec![Dependency {
                        candidate_id: "fixture-dependency".to_owned(),
                        source: "crate::application".to_owned(),
                        target: "crate::provider".to_owned(),
                        kind: DependencyKind::Import,
                        target_scope: TargetScope::InternalModule,
                        evidence: SourceEvidence {
                            path: "src/application.rs".to_owned(),
                            line: 1,
                            column: 1,
                        },
                    }],
                    excluded_scopes: Vec::new(),
                },
                Vec::new(),
            ))
        }
    }
}
