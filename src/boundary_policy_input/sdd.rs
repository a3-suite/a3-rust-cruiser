use std::path::Path;

use serde::Deserialize;

use crate::boundary_validation::{
    DirectoryStructure, LoadedBoundaryPolicy, LogicalStructure, PreparedSddDesign, Projection,
    Result, SddDesignInput, invalid_sdd, prepare_sdd_design, project_sdd_policy,
};

pub(super) fn prepare(
    logical_structure_path: &Path,
    projection_path: &Path,
    directory_structure_path: &Path,
) -> Result<PreparedSddDesign> {
    let logical: LogicalStructure = read_yaml(logical_structure_path, "logical-structure")?;
    let projection: Projection = read_yaml(projection_path, "projection")?;
    let directories: DirectoryStructure =
        read_yaml(directory_structure_path, "directory-structure")?;

    prepare_sdd_design(SddDesignInput {
        logical,
        projection,
        directories,
    })
}

pub(super) fn project(
    prepared: PreparedSddDesign,
    project_root: &Path,
    target_source: &Path,
) -> Result<LoadedBoundaryPolicy> {
    project_sdd_policy(prepared, project_root, target_source)
}

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &Path, kind: &str) -> Result<T> {
    let content = std::fs::read_to_string(path)?;
    let value: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|error| invalid_sdd(format!("invalid {kind} YAML: {error}")))?;
    validate_discriminator(&value, kind)?;
    serde_yaml::from_value(value)
        .map_err(|error| invalid_sdd(format!("invalid {kind} YAML: {error}")))
}

fn validate_discriminator(value: &serde_yaml::Value, kind: &str) -> Result<()> {
    let version = value
        .get("version")
        .and_then(serde_yaml::Value::as_u64)
        .ok_or_else(|| invalid_sdd(format!("{kind} requires an integer version")))?;
    let name = value
        .get("name")
        .and_then(serde_yaml::Value::as_str)
        .ok_or_else(|| invalid_sdd(format!("{kind} requires a name")))?;
    if name != kind {
        return Err(invalid_sdd(format!(
            "unexpected SDD name for {kind}: {name}"
        )));
    }
    if version != 1 {
        return Err(invalid_sdd(format!(
            "unsupported {kind} version: {version}"
        )));
    }
    Ok(())
}
