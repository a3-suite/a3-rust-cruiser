use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use syn::File;

mod builtin_names;
mod dependency_collection;
mod macro_expansion;
mod module_discovery;
mod observation;
mod path_resolution;
mod source_evidence;

use super::{
    cfg_context::CfgContext,
    rust_syntax::{UseAlias, UseBinding},
};
use crate::boundary_validation::{AnalysisIssue, ModuleDeclaration};

pub struct RustDependencyObserver;

struct ModuleRecord {
    id: String,
    variants: Vec<ModuleVariant>,
}

struct ModuleVariant {
    path: PathBuf,
    module_directory: PathBuf,
    ast: File,
    declared_items: BTreeSet<String>,
    public_items: BTreeSet<String>,
    public_use_bindings: Vec<UseBinding>,
    use_aliases: BTreeMap<String, Vec<UseAlias>>,
    glob_imports: Vec<UseAlias>,
    public_glob_imports: Vec<UseAlias>,
    local_macro_bindings: BTreeSet<String>,
    external_crate_aliases: BTreeSet<String>,
    self_crate_aliases: BTreeSet<String>,
    no_implicit_prelude: bool,
}

struct Observer {
    project_root: PathBuf,
    cfg_context: CfgContext,
    source_lines: BTreeMap<PathBuf, Vec<String>>,
    external_crates: BTreeSet<String>,
    context_dependent_crates: BTreeSet<String>,
    self_crate_aliases: BTreeSet<String>,
    crate_macro_rules_names: BTreeSet<String>,
    records: BTreeMap<String, ModuleRecord>,
    exported_items: BTreeMap<String, BTreeSet<String>>,
    declarations: Vec<ModuleDeclaration>,
    issues: Vec<AnalysisIssue>,
}
