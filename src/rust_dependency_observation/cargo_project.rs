use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use crate::boundary_validation::{
    AnalysisContext, AnalysisIssue, AnalysisProfile, CruiserError, Result,
};

use super::cfg_context::{CfgContext, CfgDecision};

pub(super) fn load_manifest(project_root: &Path) -> Result<toml::Value> {
    let manifest_path = project_root.join("Cargo.toml");
    let content = fs::read_to_string(&manifest_path).map_err(|error| {
        CruiserError::io(format!("Cargo manifest {}", manifest_path.display()), error)
    })?;
    content.parse().map_err(|error| {
        CruiserError::parse(
            format!("invalid Cargo.toml {}", manifest_path.display()),
            error,
        )
    })
}

pub(super) fn declared_external_crates(
    manifest: &toml::Value,
    context: &AnalysisContext,
    cfg_context: &CfgContext,
) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    validate_manifest_dependency_shapes(manifest)?;
    let active_features = active_features(manifest, &context.features)?;
    let mut crates = BTreeSet::new();
    let mut context_dependent = BTreeSet::new();
    let mut source_union_candidates = BTreeSet::new();
    collect_dependency_names(
        dependency_table(manifest.get("dependencies"), "dependencies")?,
        Some(&active_features),
        &mut crates,
        &mut context_dependent,
    )?;
    if is_dev_compilation_target(&context.compilation_target) {
        collect_dependency_names(
            dependency_table(manifest.get("dev-dependencies"), "dev-dependencies")?,
            Some(&active_features),
            &mut crates,
            &mut context_dependent,
        )?;
    }
    source_union_candidates.extend(context_dependent.iter().cloned());
    let targets = match manifest.get("target") {
        Some(value) => Some(value.as_table().ok_or_else(|| {
            CruiserError::InvalidInput("Cargo.toml target must be a table".to_owned())
        })?),
        None => None,
    };
    if let Some(targets) = targets {
        for (selector, target) in targets {
            let target = target.as_table().ok_or_else(|| {
                CruiserError::InvalidInput(format!("Cargo.toml target.{selector} must be a table"))
            })?;
            if selector == &context.target_triple {
                let previous_context_dependent = context_dependent.clone();
                collect_dependency_names(
                    dependency_table(
                        target.get("dependencies"),
                        &format!("target.{selector}.dependencies"),
                    )?,
                    Some(&active_features),
                    &mut crates,
                    &mut context_dependent,
                )?;
                if is_dev_compilation_target(&context.compilation_target) {
                    collect_dependency_names(
                        dependency_table(
                            target.get("dev-dependencies"),
                            &format!("target.{selector}.dev-dependencies"),
                        )?,
                        Some(&active_features),
                        &mut crates,
                        &mut context_dependent,
                    )?;
                }
                source_union_candidates.extend(
                    context_dependent
                        .difference(&previous_context_dependent)
                        .cloned(),
                );
            } else if selector.starts_with("cfg(") {
                match cfg_context.evaluate_selector(selector) {
                    CfgDecision::Active => collect_runtime_dependency_names(
                        target,
                        selector,
                        is_dev_compilation_target(&context.compilation_target),
                        Some(&active_features),
                        &mut crates,
                    )?,
                    CfgDecision::Inactive => {}
                    CfgDecision::Unknown => {
                        collect_all_dependency_names(target, selector, &mut context_dependent)?;
                    }
                }
                collect_runtime_dependency_names(
                    target,
                    selector,
                    is_dev_compilation_target(&context.compilation_target),
                    None,
                    &mut source_union_candidates,
                )?;
            }
        }
    }
    if context.analysis_profile == AnalysisProfile::SourceUnion {
        crates.extend(source_union_candidates);
    }
    context_dependent.retain(|name| !crates.contains(name));
    Ok((crates, context_dependent))
}

fn is_dev_compilation_target(target: &str) -> bool {
    matches!(
        target.split_once(':').map_or(target, |(kind, _)| kind),
        "example" | "test" | "bench"
    )
}

fn active_features(manifest: &toml::Value, requested: &[String]) -> Result<BTreeSet<String>> {
    let feature_table = match manifest.get("features") {
        Some(value) => Some(value.as_table().ok_or_else(|| {
            CruiserError::InvalidInput("Cargo.toml features must be a table".to_owned())
        })?),
        None => None,
    };
    let mut active = requested.iter().cloned().collect::<BTreeSet<_>>();
    if feature_table.is_some_and(|features| features.contains_key("default")) {
        active.insert("default".to_owned());
    }
    let mut pending = active.iter().cloned().collect::<Vec<_>>();
    while let Some(feature) = pending.pop() {
        let Some(entries) = feature_table.and_then(|features| features.get(&feature)) else {
            continue;
        };
        let entries = entries.as_array().ok_or_else(|| {
            CruiserError::InvalidInput(format!("Cargo.toml feature {feature} must be an array"))
        })?;
        for entry in entries {
            let entry = entry.as_str().ok_or_else(|| {
                CruiserError::InvalidInput(format!(
                    "Cargo.toml feature {feature} entries must be text"
                ))
            })?;
            let local = entry
                .strip_prefix("dep:")
                .or_else(|| {
                    entry
                        .split_once('/')
                        .map(|(name, _)| name.trim_end_matches('?'))
                })
                .unwrap_or(entry);
            if active.insert(entry.to_owned()) {
                pending.push(entry.to_owned());
            }
            active.insert(local.to_owned());
            active.insert(local.replace('-', "_"));
        }
    }
    Ok(active)
}

fn collect_dependency_names(
    table: Option<&toml::value::Table>,
    active_features: Option<&BTreeSet<String>>,
    active: &mut BTreeSet<String>,
    context_dependent: &mut BTreeSet<String>,
) -> Result<()> {
    let Some(table) = table else { return Ok(()) };
    for (name, declaration) in table {
        if !(declaration.is_str() || declaration.is_table()) {
            return Err(CruiserError::InvalidInput(format!(
                "dependency {name} must be a version string or table"
            )));
        }
        let normalized = name.replace('-', "_");
        let optional = match declaration
            .as_table()
            .and_then(|value| value.get("optional"))
        {
            Some(value) => value.as_bool().ok_or_else(|| {
                CruiserError::InvalidInput(format!("dependency {name}.optional must be a boolean"))
            })?,
            None => false,
        };
        let feature_active = active_features.is_some_and(|features| {
            features.contains(name)
                || features.contains(&normalized)
                || features.contains(&format!("dep:{name}"))
                || features.contains(&format!("dep:{normalized}"))
        });
        if optional && !feature_active {
            context_dependent.insert(normalized);
        } else {
            active.insert(normalized);
        }
    }
    Ok(())
}

fn collect_all_dependency_names(
    target: &toml::value::Table,
    selector: &str,
    names: &mut BTreeSet<String>,
) -> Result<()> {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let table = dependency_table(target.get(section), &format!("target.{selector}.{section}"))?;
        let mut active = BTreeSet::new();
        let mut conditional = BTreeSet::new();
        collect_dependency_names(table, None, &mut active, &mut conditional)?;
        names.extend(active);
        names.extend(conditional);
    }
    Ok(())
}

fn collect_runtime_dependency_names(
    target: &toml::value::Table,
    selector: &str,
    include_dev_dependencies: bool,
    active_features: Option<&BTreeSet<String>>,
    names: &mut BTreeSet<String>,
) -> Result<()> {
    let sections = if include_dev_dependencies {
        [Some("dependencies"), Some("dev-dependencies")]
    } else {
        [Some("dependencies"), None]
    };
    for section in sections.into_iter().flatten() {
        let table = dependency_table(target.get(section), &format!("target.{selector}.{section}"))?;
        let mut active = BTreeSet::new();
        let mut conditional = BTreeSet::new();
        collect_dependency_names(table, active_features, &mut active, &mut conditional)?;
        names.extend(active);
        if active_features.is_none() {
            names.extend(conditional);
        }
    }
    Ok(())
}

fn dependency_table<'a>(
    value: Option<&'a toml::Value>,
    name: &str,
) -> Result<Option<&'a toml::value::Table>> {
    value
        .map(|value| {
            value.as_table().ok_or_else(|| {
                CruiserError::InvalidInput(format!("Cargo.toml {name} must be a table"))
            })
        })
        .transpose()
}

fn validate_manifest_dependency_shapes(manifest: &toml::Value) -> Result<()> {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let mut active = BTreeSet::new();
        let mut conditional = BTreeSet::new();
        collect_dependency_names(
            dependency_table(manifest.get(section), section)?,
            None,
            &mut active,
            &mut conditional,
        )?;
    }
    let Some(targets) = manifest.get("target") else {
        return Ok(());
    };
    let targets = targets.as_table().ok_or_else(|| {
        CruiserError::InvalidInput("Cargo.toml target must be a table".to_owned())
    })?;
    for (selector, target) in targets {
        let target = target.as_table().ok_or_else(|| {
            CruiserError::InvalidInput(format!("Cargo.toml target.{selector} must be a table"))
        })?;
        let mut names = BTreeSet::new();
        collect_all_dependency_names(target, selector, &mut names)?;
    }
    Ok(())
}

pub(super) fn target_entry(
    project_root: &Path,
    manifest: &toml::Value,
    context: &AnalysisContext,
) -> Result<(PathBuf, Vec<AnalysisIssue>)> {
    validate_manifest_target_shapes(manifest)?;
    let manifest_package = manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            CruiserError::InvalidInput("Cargo.toml package.name is missing".to_owned())
        })?;
    if manifest_package != context.package {
        return Err(CruiserError::InvalidInput(format!(
            "requested package {} does not match Cargo package {manifest_package}",
            context.package
        )));
    }
    let target = context.compilation_target.as_str();
    let (target_kind, target_name) = target.split_once(':').unwrap_or((target, ""));
    let mut issues = Vec::new();
    let path = match target_kind {
        "lib" => {
            let explicit = manifest.get("lib").and_then(toml::Value::as_table);
            if explicit.is_none() && !package_auto_enabled(manifest, "autolib")? {
                return Err(CruiserError::InvalidInput(
                    "library target not found: lib".to_owned(),
                ));
            }
            match explicit
                .and_then(|target| target.get("path"))
                .and_then(toml::Value::as_str)
            {
                Some(path) => project_root.join(path),
                None => project_root.join("src/lib.rs"),
            }
        }
        "bin" => {
            if target_name.is_empty() {
                return Err(CruiserError::InvalidInput(
                    "bin target requires a name".to_owned(),
                ));
            }
            resolve_named_target(
                project_root,
                manifest,
                "bin",
                target_name,
                manifest_package,
                context,
                &mut issues,
            )?
        }
        "example" | "test" | "bench" => {
            if target_name.is_empty() {
                return Err(CruiserError::InvalidInput(format!(
                    "{target_kind} target requires a name"
                )));
            }
            resolve_named_target(
                project_root,
                manifest,
                target_kind,
                target_name,
                manifest_package,
                context,
                &mut issues,
            )?
        }
        _ => {
            return Err(CruiserError::InvalidInput(format!(
                "unsupported compilation target for MVP: {target}"
            )));
        }
    };
    if path.is_file() {
        Ok((path, issues))
    } else {
        Err(CruiserError::InvalidInput(format!(
            "target source file not found: {}",
            path.display()
        )))
    }
}

pub(super) fn has_library_target(project_root: &Path, manifest: &toml::Value) -> Result<bool> {
    let path = match manifest.get("lib").and_then(toml::Value::as_table) {
        Some(target) => match target.get("path").and_then(toml::Value::as_str) {
            Some(path) => project_root.join(path),
            None => project_root.join("src/lib.rs"),
        },
        None if package_auto_enabled(manifest, "autolib")? => project_root.join("src/lib.rs"),
        None => return Ok(false),
    };
    Ok(path.is_file())
}

fn resolve_named_target(
    project_root: &Path,
    manifest: &toml::Value,
    kind: &str,
    name: &str,
    package: &str,
    context: &AnalysisContext,
    issues: &mut Vec<AnalysisIssue>,
) -> Result<PathBuf> {
    let explicit = explicit_target(manifest, kind, name);
    if let Some(target) = explicit {
        collect_required_feature_issue(target, context, issues)?;
        if let Some(path) = target.get("path").and_then(toml::Value::as_str) {
            return Ok(project_root.join(path));
        }
    } else if !package_auto_enabled(manifest, auto_target_key(kind))? {
        return Err(CruiserError::InvalidInput(format!(
            "{kind} target not found: {kind}:{name}"
        )));
    }

    let candidates = conventional_target_paths(project_root, kind, name, package);
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .or_else(|| explicit.is_some().then(|| candidates[0].clone()))
        .ok_or_else(|| {
            CruiserError::InvalidInput(format!("{kind} target not found: {kind}:{name}"))
        })
}

fn explicit_target<'a>(
    manifest: &'a toml::Value,
    kind: &str,
    name: &str,
) -> Option<&'a toml::value::Table> {
    manifest
        .get(kind)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
        .find(|target| {
            target
                .get("name")
                .and_then(toml::Value::as_str)
                .is_some_and(|target_name| target_name == name)
        })
}

fn collect_required_feature_issue(
    target: &toml::value::Table,
    context: &AnalysisContext,
    issues: &mut Vec<AnalysisIssue>,
) -> Result<()> {
    let required_features = match target.get("required-features") {
        Some(value) => Some(value.as_array().ok_or_else(|| {
            CruiserError::InvalidInput("required-features must be an array".to_owned())
        })?),
        None => None,
    };
    let mut missing = Vec::new();
    for feature in required_features.into_iter().flatten() {
        let feature = feature.as_str().ok_or_else(|| {
            CruiserError::InvalidInput("required-features entries must be text".to_owned())
        })?;
        if !context.features.iter().any(|active| active == feature) {
            missing.push(feature);
        }
    }
    if !missing.is_empty() {
        issues.push(AnalysisIssue::analysis_context_unresolved(
            "analysis.unsupported-analysis-context",
            format!(
                "target {} requires inactive features: {}",
                context.compilation_target,
                missing.join(", ")
            ),
            context.compilation_target.clone(),
            None,
            None,
            vec![
                crate::boundary_validation::DependencyKind::Import,
                crate::boundary_validation::DependencyKind::ReExport,
                crate::boundary_validation::DependencyKind::PathReference,
            ],
        ));
    }
    Ok(())
}

fn package_auto_enabled(manifest: &toml::Value, key: &str) -> Result<bool> {
    match manifest.get("package").and_then(|package| package.get(key)) {
        Some(value) => value.as_bool().ok_or_else(|| {
            CruiserError::InvalidInput(format!("Cargo.toml package.{key} must be a boolean"))
        }),
        None => Ok(true),
    }
}

fn validate_manifest_target_shapes(manifest: &toml::Value) -> Result<()> {
    for key in [
        "autolib",
        "autobins",
        "autoexamples",
        "autotests",
        "autobenches",
    ] {
        package_auto_enabled(manifest, key)?;
    }
    if let Some(lib) = manifest.get("lib") {
        let lib = lib.as_table().ok_or_else(|| {
            CruiserError::InvalidInput("Cargo.toml lib target must be a table".to_owned())
        })?;
        if lib.get("path").is_some_and(|path| !path.is_str()) {
            return Err(CruiserError::InvalidInput(
                "Cargo.toml lib.path must be text".to_owned(),
            ));
        }
    }
    for kind in ["bin", "example", "test", "bench"] {
        let Some(value) = manifest.get(kind) else {
            continue;
        };
        let targets = value.as_array().ok_or_else(|| {
            CruiserError::InvalidInput(format!("Cargo.toml {kind} targets must be an array"))
        })?;
        for target in targets {
            let target = target.as_table().ok_or_else(|| {
                CruiserError::InvalidInput(format!("Cargo.toml {kind} target must be a table"))
            })?;
            if !target.get("name").is_some_and(toml::Value::is_str) {
                return Err(CruiserError::InvalidInput(format!(
                    "Cargo.toml {kind} target.name must be text"
                )));
            }
            if target.get("path").is_some_and(|path| !path.is_str()) {
                return Err(CruiserError::InvalidInput(format!(
                    "Cargo.toml {kind} target.path must be text"
                )));
            }
            if let Some(required) = target.get("required-features") {
                let required = required.as_array().ok_or_else(|| {
                    CruiserError::InvalidInput("required-features must be an array".to_owned())
                })?;
                if required.iter().any(|feature| !feature.is_str()) {
                    return Err(CruiserError::InvalidInput(
                        "required-features entries must be text".to_owned(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn auto_target_key(kind: &str) -> &str {
    match kind {
        "bin" => "autobins",
        "example" => "autoexamples",
        "test" => "autotests",
        "bench" => "autobenches",
        _ => "",
    }
}

fn conventional_target_paths(
    project_root: &Path,
    kind: &str,
    name: &str,
    package: &str,
) -> Vec<PathBuf> {
    match kind {
        "bin" if name == package => vec![
            project_root.join("src/main.rs"),
            project_root.join(format!("src/bin/{name}.rs")),
            project_root.join(format!("src/bin/{name}/main.rs")),
        ],
        "bin" => vec![
            project_root.join(format!("src/bin/{name}.rs")),
            project_root.join(format!("src/bin/{name}/main.rs")),
        ],
        "example" => vec![
            project_root.join(format!("examples/{name}.rs")),
            project_root.join(format!("examples/{name}/main.rs")),
        ],
        "test" => vec![project_root.join(format!("tests/{name}.rs"))],
        "bench" => vec![project_root.join(format!("benches/{name}.rs"))],
        _ => Vec::new(),
    }
}
