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
    serde_yaml::from_str(&content)
        .map_err(|error| invalid_sdd(format!("invalid {kind} YAML: {error}")))
}
