use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::Deserialize;

use crate::boundary_evaluation::{
    AnalysisIssue, BoundaryPolicy, BoundaryRule, ComponentAttributionModel, LoadedBoundaryPolicy,
    RuleKind, Severity,
};

use super::{CruiserError, Result};

/// Parsed SDD design input shared between the representation adapter and the semantic compiler.
///
/// The adapter owns deserialization; this value contains only neutral identifiers and paths.
pub(crate) struct SddDesignInput {
    pub(crate) logical: LogicalStructure,
    pub(crate) projection: Projection,
    pub(crate) directories: DirectoryStructure,
}

pub(crate) struct PreparedSddDesign {
    nodes: BTreeMap<String, String>,
    mappings: BTreeSet<(String, String)>,
    declared_dependencies: BTreeSet<(String, String)>,
}

#[cfg(test)]
impl PreparedSddDesign {
    pub(crate) fn valid_for_test() -> Self {
        Self {
            nodes: BTreeMap::from([("node".to_owned(), "src/application.rs".to_owned())]),
            mappings: BTreeSet::from([("component".to_owned(), "node".to_owned())]),
            declared_dependencies: BTreeSet::new(),
        }
    }
}

pub(crate) fn prepare_sdd_design(input: SddDesignInput) -> Result<PreparedSddDesign> {
    let SddDesignInput {
        logical,
        projection,
        directories,
    } = input;

    let component_ids = unique_values(
        logical
            .components
            .into_iter()
            .map(|component| component.component_id),
        "logical component ID",
    )?;
    if component_ids.is_empty() {
        return Err(invalid_sdd("logical-structure requires a component"));
    }

    let nodes = unique_pairs(
        directories
            .nodes
            .into_iter()
            .map(|node| (node.node_id, node.path)),
        "directory node ID",
    )?;
    let mut mappings = BTreeSet::new();
    for mapping in projection.mappings {
        if mapping.component_id.is_empty() || mapping.node_id.is_empty() {
            return Err(invalid_sdd("projection mapping IDs must not be empty"));
        }
        mappings.insert((mapping.component_id, mapping.node_id));
    }
    let projected_component_ids = mappings
        .iter()
        .map(|(component_id, _)| component_id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(component_id) = projected_component_ids.difference(&component_ids).next() {
        return Err(invalid_sdd(format!(
            "projection references unknown logical component: {component_id}"
        )));
    }
    if projected_component_ids != component_ids {
        return Err(invalid_sdd(
            "each logical component must have at least one projection",
        ));
    }

    for (_, node_id) in &mappings {
        if !nodes.contains_key(node_id) {
            return Err(invalid_sdd(format!(
                "projection references unknown directory node: {node_id}"
            )));
        }
    }

    let mut declared_dependencies = BTreeSet::new();
    for dependency in logical.dependencies {
        if !component_ids.contains(&dependency.from_component_id) {
            return Err(invalid_sdd(format!(
                "dependency references unknown source component: {}",
                dependency.from_component_id
            )));
        }
        if !component_ids.contains(&dependency.to_component_id) {
            return Err(invalid_sdd(format!(
                "dependency references unknown target component: {}",
                dependency.to_component_id
            )));
        }
        declared_dependencies.insert((dependency.from_component_id, dependency.to_component_id));
    }
    validate_acyclic_dependencies(&component_ids, &declared_dependencies)?;

    Ok(PreparedSddDesign {
        nodes,
        mappings,
        declared_dependencies,
    })
}

pub(crate) fn project_sdd_policy(
    prepared: PreparedSddDesign,
    project_root: &Path,
    target_source: &Path,
) -> Result<LoadedBoundaryPolicy> {
    let PreparedSddDesign {
        nodes,
        mappings,
        declared_dependencies,
    } = prepared;

    let mut component_modules = BTreeMap::<String, BTreeSet<String>>::new();
    let mut module_nodes = BTreeMap::new();
    for (component_id, node_id) in mappings {
        let path = nodes
            .get(&node_id)
            .expect("prepared SDD projection node must exist");
        if project_root.join(path) == target_source {
            return Err(invalid_sdd(format!(
                "component cannot project to the selected Cargo target root: {path}"
            )));
        }
        let Some(module) = rust_module_id(path)? else {
            continue;
        };
        if module_nodes
            .insert(module.clone(), node_id.clone())
            .is_some_and(|existing_node| existing_node != node_id)
        {
            return Err(invalid_sdd(format!(
                "multiple directory nodes project to the same Rust module: {module}"
            )));
        }
        component_modules
            .entry(component_id)
            .or_default()
            .insert(module);
    }
    for modules in component_modules.values_mut() {
        let roots = modules.clone();
        modules.retain(|module| {
            !roots
                .iter()
                .any(|root| root != module && module_is_within(module, root))
        });
    }
    let mut module_components = BTreeMap::<String, BTreeSet<String>>::new();
    for (component_id, modules) in &component_modules {
        for module in modules {
            module_components
                .entry(module.clone())
                .or_default()
                .insert(component_id.clone());
        }
    }

    let mut ambiguous_components = BTreeSet::new();
    let mut analysis_issues = Vec::new();
    for (module, components) in module_components
        .iter()
        .filter(|(_, components)| components.len() > 1)
    {
        let candidate_components = components.iter().cloned().collect::<Vec<_>>();
        ambiguous_components.extend(components.iter().cloned());
        analysis_issues.push(AnalysisIssue::component_attribution_unresolved(
            "analysis.ambiguous-component-attribution",
            format!(
                "Rust module {module} represents multiple logical components: {}",
                candidate_components.join(", ")
            ),
            module.clone(),
            candidate_components,
        ));
    }
    for (ancestor, ancestor_components) in &module_components {
        let descendant_prefix = format!("{ancestor}::");
        for (descendant, descendant_components) in module_components
            .range(descendant_prefix.clone()..)
            .take_while(|(module, _)| module.starts_with(&descendant_prefix))
        {
            let components = ancestor_components
                .iter()
                .chain(descendant_components)
                .cloned()
                .collect::<BTreeSet<_>>();
            if components.len() < 2 {
                continue;
            }
            ambiguous_components.extend(components.iter().cloned());
            let candidate_components = components.iter().cloned().collect::<Vec<_>>();
            analysis_issues.push(AnalysisIssue::component_attribution_unresolved(
                "analysis.ambiguous-component-attribution",
                format!(
                    "Rust module scope {ancestor} overlaps {descendant} for logical components: {}",
                    candidate_components.join(", ")
                ),
                descendant.clone(),
                candidate_components,
            ));
        }
    }

    let mut rules = Vec::new();
    for (source_component, source_modules) in &component_modules {
        for (target_component, target_modules) in &component_modules {
            if source_component != target_component
                && !ambiguous_components.contains(source_component)
                && !ambiguous_components.contains(target_component)
                && !declared_dependencies
                    .contains(&(source_component.clone(), target_component.clone()))
            {
                rules.push(BoundaryRule {
                    id: format!("sdd-forbidden-{:04}", rules.len() + 1),
                    kind: RuleKind::Forbidden,
                    severity: Severity::Error,
                    from: source_modules
                        .iter()
                        .flat_map(|module| module_patterns(module))
                        .collect(),
                    to: target_modules
                        .iter()
                        .flat_map(|module| module_patterns(module))
                        .collect(),
                    modules: Vec::new(),
                    dependency_kinds: Vec::new(),
                });
            }
        }
    }
    Ok(LoadedBoundaryPolicy {
        policy: BoundaryPolicy { rules },
        analysis_issues,
        component_attribution: (!ambiguous_components.is_empty()).then_some(
            ComponentAttributionModel {
                component_modules,
                declared_dependencies,
            },
        ),
    })
}

fn validate_acyclic_dependencies(
    component_ids: &BTreeSet<String>,
    dependencies: &BTreeSet<(String, String)>,
) -> Result<()> {
    let mut incoming = component_ids
        .iter()
        .map(|component| (component.clone(), 0_usize))
        .collect::<BTreeMap<_, _>>();
    let mut outgoing = component_ids
        .iter()
        .map(|component| (component.clone(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for (source, target) in dependencies {
        if source == target {
            return Err(invalid_sdd(format!(
                "logical component dependency graph is circular at: {source}"
            )));
        }
        outgoing
            .get_mut(source)
            .expect("validated source component must exist")
            .push(target.clone());
        *incoming
            .get_mut(target)
            .expect("validated target component must exist") += 1;
    }
    let mut ready = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(component, _)| component.clone())
        .collect::<BTreeSet<_>>();
    let mut visited = 0;
    while let Some(component) = ready.pop_first() {
        visited += 1;
        for target in &outgoing[&component] {
            let count = incoming
                .get_mut(target)
                .expect("validated target component must exist");
            *count -= 1;
            if *count == 0 {
                ready.insert(target.clone());
            }
        }
    }
    if visited != component_ids.len() {
        return Err(invalid_sdd(
            "logical component dependency graph must be acyclic",
        ));
    }
    Ok(())
}

fn unique_values(values: impl IntoIterator<Item = String>, kind: &str) -> Result<BTreeSet<String>> {
    let mut unique = BTreeSet::new();
    for value in values {
        if value.is_empty() || !unique.insert(value.clone()) {
            return Err(invalid_sdd(format!("invalid or duplicate {kind}: {value}")));
        }
    }
    Ok(unique)
}

fn unique_pairs(
    pairs: impl IntoIterator<Item = (String, String)>,
    kind: &str,
) -> Result<BTreeMap<String, String>> {
    let mut unique = BTreeMap::new();
    for (key, value) in pairs {
        if key.is_empty() || value.is_empty() || unique.insert(key.clone(), value).is_some() {
            return Err(invalid_sdd(format!("invalid or duplicate {kind}: {key}")));
        }
    }
    Ok(unique)
}

fn rust_module_id(path: &str) -> Result<Option<String>> {
    if path.starts_with('/')
        || path
            .split('/')
            .any(|segment| segment == "." || segment == "..")
    {
        return Err(invalid_sdd(format!(
            "component path must be a normalized path below src/: {path}"
        )));
    }
    let Some(relative) = path.strip_prefix("src/") else {
        return Ok(None);
    };
    let module_path = relative
        .strip_suffix("/mod.rs")
        .or_else(|| relative.strip_suffix(".rs"))
        .unwrap_or(relative);
    if module_path.is_empty() {
        return Err(invalid_sdd(format!(
            "crate root cannot represent a component: {path}"
        )));
    }
    let segments = module_path.split('/').collect::<Vec<_>>();
    if segments
        .iter()
        .any(|segment| !is_rust_module_segment(segment))
    {
        return Err(invalid_sdd(format!(
            "component path contains an unsupported Rust module segment: {path}"
        )));
    }
    Ok(Some(format!("crate::{}", segments.join("::"))))
}

fn is_rust_module_segment(segment: &str) -> bool {
    let mut characters = segment.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn module_patterns(module: &str) -> Vec<String> {
    vec![module.to_owned(), format!("{module}::**")]
}

fn module_is_within(module: &str, root: &str) -> bool {
    module == root || module.starts_with(&format!("{root}::"))
}

pub(crate) fn invalid_sdd(message: impl Into<String>) -> CruiserError {
    CruiserError::InvalidInput(format!(
        "invalid SDD boundary policy input: {}",
        message.into()
    ))
}

#[derive(Deserialize)]
pub(crate) struct LogicalStructure {
    pub(crate) components: Vec<LogicalComponent>,
    pub(crate) dependencies: Vec<LogicalDependency>,
}

#[derive(Deserialize)]
pub(crate) struct LogicalComponent {
    pub(crate) component_id: String,
}

#[derive(Deserialize)]
pub(crate) struct LogicalDependency {
    pub(crate) from_component_id: String,
    pub(crate) to_component_id: String,
}

#[derive(Deserialize)]
pub(crate) struct Projection {
    pub(crate) mappings: Vec<ProjectionMapping>,
}

#[derive(Deserialize)]
pub(crate) struct ProjectionMapping {
    pub(crate) component_id: String,
    pub(crate) node_id: String,
}

#[derive(Deserialize)]
pub(crate) struct DirectoryStructure {
    pub(crate) nodes: Vec<DirectoryNode>,
}

#[derive(Deserialize)]
pub(crate) struct DirectoryNode {
    pub(crate) node_id: String,
    pub(crate) path: String,
}

#[cfg(test)]
mod tests {
    use super::rust_module_id;

    #[test]
    fn should_map_source_paths_to_logical_modules() {
        // Arrange
        let module_file = "src/application.rs";
        let module_directory = "src/domain/model";
        let module_root = "src/infrastructure/mod.rs";

        // Act
        let mapped_file = rust_module_id(module_file)
            .expect("module file should be valid")
            .expect("module file should map");
        let mapped_directory = rust_module_id(module_directory)
            .expect("module directory should be valid")
            .expect("module directory should map");
        let mapped_root = rust_module_id(module_root)
            .expect("mod.rs should be valid")
            .expect("mod.rs should map");

        // Assert
        assert_eq!(mapped_file, "crate::application");
        assert_eq!(mapped_directory, "crate::domain::model");
        assert_eq!(mapped_root, "crate::infrastructure");
    }

    #[test]
    fn should_exclude_project_sources_outside_the_rust_module_scope() {
        // Arrange
        let path = ".ci/scripts";

        // Act
        let mapped = rust_module_id(path).expect("project-relative path should be valid");

        // Assert
        assert_eq!(mapped, None);
    }
}
