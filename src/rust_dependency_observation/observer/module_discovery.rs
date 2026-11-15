use std::{
    fs,
    path::{Path, PathBuf},
};

use syn::spanned::Spanned;
use syn::{File, Item};

use super::super::{
    cfg_context::CfgDecision,
    rust_syntax::{
        declared_external_crate_aliases, declared_item_names, declared_self_crate_aliases,
        glob_imports, has_no_implicit_prelude, has_path_attribute, item_attributes,
        local_macro_bindings, public_glob_imports, public_item_names, public_use_bindings,
        use_aliases,
    },
};
use super::{ModuleRecord, ModuleVariant, Observer, dependency_collection::all_dependency_kinds};
use crate::boundary_validation::{AnalysisIssue, CruiserError, ModuleDeclaration, Result};

impl Observer {
    pub(super) fn discover_module(&mut self, id: &str, path: &Path) -> Result<()> {
        if self.records.contains_key(id) {
            return Ok(());
        }
        let source = fs::read_to_string(path)
            .map_err(|error| CruiserError::io(path.display().to_string(), error))?;
        self.source_lines.insert(
            path.to_path_buf(),
            source.lines().map(str::to_owned).collect(),
        );
        let ast = match syn::parse_file(&source) {
            Ok(ast) => ast,
            Err(error) => {
                let evidence = self.evidence(path, error.span().start());
                self.issues.push(
                    AnalysisIssue::analysis_context_unresolved(
                        "analysis.unsupported-syntax",
                        format!("unable to parse {}: {error}", path.display()),
                        id,
                        Some(id.to_owned()),
                        Some(evidence),
                        all_dependency_kinds(),
                    )
                    .with_descendant_modules(),
                );
                return Ok(());
            }
        };
        let variant = ModuleVariant {
            path: path.to_path_buf(),
            module_directory: module_directory(path),
            declared_items: declared_item_names(&ast, &self.cfg_context),
            public_items: public_item_names(&ast, &self.cfg_context),
            public_use_bindings: public_use_bindings(&ast, &self.cfg_context),
            use_aliases: use_aliases(&ast, &self.cfg_context),
            glob_imports: glob_imports(&ast, &self.cfg_context),
            public_glob_imports: public_glob_imports(&ast, &self.cfg_context),
            local_macro_bindings: local_macro_bindings(&ast, &self.cfg_context),
            external_crate_aliases: declared_external_crate_aliases(&ast, &self.cfg_context),
            self_crate_aliases: declared_self_crate_aliases(&ast, &self.cfg_context),
            no_implicit_prelude: has_no_implicit_prelude(&ast),
            ast,
        };
        let parent_path = variant.path.clone();
        let parent_module_directory = variant.module_directory.clone();
        let items = variant.ast.items.clone();
        self.records.insert(
            id.to_owned(),
            ModuleRecord {
                id: id.to_owned(),
                variants: vec![variant],
            },
        );
        self.discover_child_items(id, &parent_path, &parent_module_directory, items)?;
        Ok(())
    }

    fn discover_child_items(
        &mut self,
        parent_id: &str,
        parent_path: &Path,
        parent_module_directory: &Path,
        items: Vec<Item>,
    ) -> Result<()> {
        for item in items {
            let cfg_decision = self.cfg_context.evaluate_attributes(item_attributes(&item));
            if cfg_decision == CfgDecision::Inactive {
                continue;
            }
            let Item::Mod(module) = item else {
                if cfg_decision == CfgDecision::Unknown {
                    self.issues.push(AnalysisIssue::analysis_context_unresolved(
                        "analysis.unsupported-analysis-context",
                        "unable to evaluate cfg condition for module declaration",
                        parent_id,
                        Some(parent_id.to_owned()),
                        Some(self.evidence(parent_path, item.span().start())),
                        all_dependency_kinds(),
                    ));
                }
                continue;
            };
            let child_id = format!("{parent_id}::{}", module.ident);
            let evidence = self.evidence(parent_path, module.span().start());
            if cfg_decision == CfgDecision::Unknown {
                self.issues.push(AnalysisIssue::analysis_context_unresolved(
                    "analysis.unsupported-analysis-context",
                    "unable to evaluate cfg condition for module declaration",
                    child_id.clone(),
                    Some(child_id.clone()),
                    Some(evidence.clone()),
                    all_dependency_kinds(),
                ));
            }
            self.declarations.push(ModuleDeclaration {
                declaration_id: format!("decl:{parent_id}:{}", module.ident),
                parent: parent_id.to_owned(),
                child: child_id.clone(),
                evidence: evidence.clone(),
            });
            if cfg_decision != CfgDecision::Unknown
                && has_path_attribute(&module.attrs, &self.cfg_context)
            {
                self.issues.push(
                    AnalysisIssue::analysis_context_unresolved(
                        "analysis.unsupported-syntax",
                        "#[path] module declarations are not supported in the MVP",
                        child_id.clone(),
                        Some(child_id.clone()),
                        Some(evidence),
                        all_dependency_kinds(),
                    )
                    .with_descendant_modules(),
                );
                continue;
            }
            if let Some((_, inline_items)) = module.content {
                let inline_file = File {
                    shebang: None,
                    attrs: module
                        .attrs
                        .iter()
                        .filter(|attribute| attribute.path().is_ident("no_implicit_prelude"))
                        .cloned()
                        .collect(),
                    items: inline_items,
                };
                let child_items = inline_file.items.clone();
                let child_module_directory = parent_module_directory.join(module.ident.to_string());
                let variant = ModuleVariant {
                    path: parent_path.to_path_buf(),
                    module_directory: child_module_directory.clone(),
                    declared_items: declared_item_names(&inline_file, &self.cfg_context),
                    public_items: public_item_names(&inline_file, &self.cfg_context),
                    public_use_bindings: public_use_bindings(&inline_file, &self.cfg_context),
                    use_aliases: use_aliases(&inline_file, &self.cfg_context),
                    glob_imports: glob_imports(&inline_file, &self.cfg_context),
                    public_glob_imports: public_glob_imports(&inline_file, &self.cfg_context),
                    local_macro_bindings: local_macro_bindings(&inline_file, &self.cfg_context),
                    external_crate_aliases: declared_external_crate_aliases(
                        &inline_file,
                        &self.cfg_context,
                    ),
                    self_crate_aliases: declared_self_crate_aliases(
                        &inline_file,
                        &self.cfg_context,
                    ),
                    no_implicit_prelude: has_no_implicit_prelude(&inline_file),
                    ast: inline_file,
                };
                if let Some(existing) = self.records.get_mut(&child_id) {
                    existing.variants.push(variant);
                } else {
                    self.records.insert(
                        child_id.clone(),
                        ModuleRecord {
                            id: child_id.clone(),
                            variants: vec![variant],
                        },
                    );
                }
                self.discover_child_items(
                    &child_id,
                    parent_path,
                    &child_module_directory,
                    child_items,
                )?;
            } else {
                let (flat_path, nested_path) =
                    module_file_candidates(parent_module_directory, &module.ident.to_string());
                let child_path =
                    resolve_module_file(parent_module_directory, &module.ident.to_string())
                        .ok_or_else(|| {
                            let evidence = self.evidence(parent_path, module.ident.span().start());
                            CruiserError::InvalidInput(format!(
                                "module file not found for {child_id}; tried {} or {}; declared at {}:{}:{}",
                                flat_path.display(),
                                nested_path.display(),
                                evidence.path,
                                evidence.line,
                                evidence.column,
                            ))
                        })?;
                self.discover_module(&child_id, &child_path)?;
            }
        }
        Ok(())
    }
}

fn resolve_module_file(directory: &Path, name: &str) -> Option<PathBuf> {
    let (flat, nested) = module_file_candidates(directory, name);
    if flat.is_file() {
        return Some(flat);
    }
    nested.is_file().then_some(nested)
}

fn module_file_candidates(directory: &Path, name: &str) -> (PathBuf, PathBuf) {
    (
        directory.join(format!("{name}.rs")),
        directory.join(name).join("mod.rs"),
    )
}

fn module_directory(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or_else(|| Path::new("."));
    let stem = file.file_stem().and_then(|stem| stem.to_str());
    if matches!(stem, Some("mod" | "main" | "lib")) {
        parent.to_path_buf()
    } else {
        parent.join(file.file_stem().unwrap_or_default())
    }
}
