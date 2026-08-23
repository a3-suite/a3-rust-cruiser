use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use proc_macro2::LineColumn;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, File, Item, ItemMod, ItemUse, UseTree};

use crate::{
    error::{CruiserError, Result},
    model::{
        AnalysisContext, AnalysisIssue, Dependency, DependencyKind, LogicalModule,
        ModuleDeclaration, ObservedDependencyGraph, SourceEvidence, TargetScope,
    },
};

struct ModuleRecord {
    id: String,
    path: PathBuf,
    module_directory: PathBuf,
    ast: File,
    declared_items: BTreeSet<String>,
    use_aliases: BTreeMap<String, UseAlias>,
}

#[derive(Clone)]
struct UseAlias {
    path: Vec<String>,
    absolute_external: bool,
}

struct Observer {
    project_root: PathBuf,
    external_crates: BTreeSet<String>,
    context_dependent_crates: BTreeSet<String>,
    records: BTreeMap<String, ModuleRecord>,
    declarations: Vec<ModuleDeclaration>,
    issues: Vec<AnalysisIssue>,
}

pub fn observe(
    project_root: &Path,
    mut context: AnalysisContext,
) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
    context.features.sort();
    context.features.dedup();
    context.cfg.sort();
    context.cfg.dedup();
    let (entry, target_issues) = target_entry(project_root, &context)?;
    let (external_crates, context_dependent_crates) =
        declared_external_crates(project_root, &context)?;
    let mut observer = Observer {
        project_root: project_root.to_path_buf(),
        external_crates,
        context_dependent_crates,
        records: BTreeMap::new(),
        declarations: Vec::new(),
        issues: target_issues,
    };
    observer.discover_module("crate", &entry)?;

    let all_module_ids: BTreeSet<String> = observer.records.keys().cloned().collect();
    let excluded_scopes: Vec<String> = all_module_ids
        .iter()
        .filter(|id| {
            context
                .exclude
                .iter()
                .any(|pattern| crate::boundary_evaluation::matches_pattern(pattern, id))
        })
        .cloned()
        .collect();
    let selected: BTreeSet<String> = all_module_ids
        .iter()
        .filter(|id| {
            let included = context.include.is_empty()
                || context
                    .include
                    .iter()
                    .any(|pattern| crate::boundary_evaluation::matches_pattern(pattern, id));
            included && !excluded_scopes.contains(id)
        })
        .cloned()
        .collect();

    let mut dependencies = Vec::new();
    let record_ids: Vec<String> = observer.records.keys().cloned().collect();
    for record_id in record_ids {
        if !selected.contains(&record_id) {
            continue;
        }
        let (record_dependencies, record_issues) =
            observer.collect_dependencies(&record_id, &selected, &all_module_ids);
        dependencies.extend(record_dependencies);
        observer.issues.extend(record_issues);
    }
    dependencies.sort_by(|left, right| {
        (
            left.source.as_str(),
            left.target.as_str(),
            left.kind,
            left.evidence.path.as_str(),
            left.evidence.line,
            left.evidence.column,
        )
            .cmp(&(
                right.source.as_str(),
                right.target.as_str(),
                right.kind,
                right.evidence.path.as_str(),
                right.evidence.line,
                right.evidence.column,
            ))
    });
    dependencies.dedup_by(|left, right| {
        left.source == right.source
            && left.target == right.target
            && left.kind == right.kind
            && left.evidence == right.evidence
    });

    let modules = selected
        .iter()
        .cloned()
        .map(|id| LogicalModule { id })
        .collect();
    observer.declarations.sort_by(|left, right| {
        (left.parent.as_str(), left.child.as_str())
            .cmp(&(right.parent.as_str(), right.child.as_str()))
    });
    observer
        .declarations
        .dedup_by(|left, right| left.declaration_id == right.declaration_id);
    observer.issues.sort_by(|left, right| {
        (
            left.code.as_str(),
            left.affected_scope.as_str(),
            left.reason.as_str(),
        )
            .cmp(&(
                right.code.as_str(),
                right.affected_scope.as_str(),
                right.reason.as_str(),
            ))
    });
    observer.issues.dedup_by(|left, right| {
        left.code == right.code
            && left.affected_scope == right.affected_scope
            && left.reason == right.reason
    });

    let graph = ObservedDependencyGraph {
        analysis_context: context,
        modules,
        module_declarations: observer.declarations,
        dependencies,
        excluded_scopes,
    };
    Ok((graph, observer.issues))
}

impl Observer {
    fn discover_module(&mut self, id: &str, path: &Path) -> Result<()> {
        if self.records.contains_key(id) {
            return Ok(());
        }
        let source = fs::read_to_string(path).map_err(|error| {
            CruiserError::Io(std::io::Error::new(
                error.kind(),
                format!("{}: {error}", path.display()),
            ))
        })?;
        let ast = match syn::parse_file(&source) {
            Ok(ast) => ast,
            Err(error) => {
                self.issues.push(AnalysisIssue {
                    code: "analysis.unsupported-syntax".to_owned(),
                    reason: format!("unable to parse {}: {error}", path.display()),
                    affected_scope: id.to_owned(),
                });
                return Ok(());
            }
        };
        let record = ModuleRecord {
            id: id.to_owned(),
            path: path.to_path_buf(),
            module_directory: module_directory(path),
            declared_items: declared_item_names(&ast),
            use_aliases: use_aliases(&ast),
            ast,
        };
        self.records.insert(id.to_owned(), record);
        self.discover_children(id)?;
        Ok(())
    }

    fn discover_children(&mut self, parent_id: &str) -> Result<()> {
        let (parent_path, parent_module_directory, items) = {
            let record = self.records.get(parent_id).ok_or_else(|| {
                CruiserError::InvalidInput(format!("missing module record: {parent_id}"))
            })?;
            (
                record.path.clone(),
                record.module_directory.clone(),
                record.ast.items.clone(),
            )
        };
        for item in items {
            let Item::Mod(module) = item else { continue };
            let child_id = format!("{parent_id}::{}", module.ident);
            let evidence = self.evidence(&parent_path, module.span().start());
            self.declarations.push(ModuleDeclaration {
                declaration_id: format!("decl:{parent_id}:{}", module.ident),
                parent: parent_id.to_owned(),
                child: child_id.clone(),
                evidence,
            });
            if has_path_attribute(&module.attrs) {
                self.issues.push(AnalysisIssue {
                    code: "analysis.unsupported-syntax".to_owned(),
                    reason: "#[path] module declarations are not supported in the MVP".to_owned(),
                    affected_scope: child_id,
                });
                continue;
            }
            if let Some((_, inline_items)) = module.content {
                let inline_file = File {
                    shebang: None,
                    attrs: Vec::new(),
                    items: inline_items,
                };
                self.records.insert(
                    child_id.clone(),
                    ModuleRecord {
                        id: child_id.clone(),
                        path: parent_path.clone(),
                        module_directory: parent_module_directory.join(module.ident.to_string()),
                        declared_items: declared_item_names(&inline_file),
                        use_aliases: use_aliases(&inline_file),
                        ast: inline_file,
                    },
                );
                self.discover_children(&child_id)?;
            } else {
                let child_path =
                    resolve_module_file(&parent_module_directory, &module.ident.to_string())
                        .ok_or_else(|| {
                            CruiserError::Io(std::io::Error::new(
                                std::io::ErrorKind::NotFound,
                                format!("module file not found for {child_id}"),
                            ))
                        })?;
                self.discover_module(&child_id, &child_path)?;
            }
        }
        Ok(())
    }

    fn collect_dependencies(
        &self,
        record_id: &str,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
    ) -> (Vec<Dependency>, Vec<AnalysisIssue>) {
        let Some(record) = self.records.get(record_id) else {
            return (Vec::new(), Vec::new());
        };
        let mut dependencies = Vec::new();
        let mut issues = Vec::new();
        for item in &record.ast.items {
            if let Item::Use(use_item) = item {
                self.collect_use(
                    record,
                    use_item,
                    selected,
                    all_modules,
                    &mut dependencies,
                    &mut issues,
                );
            }
        }
        let mut visitor = PathVisitor {
            paths: Vec::new(),
            macro_spans: Vec::new(),
            cfg_spans: Vec::new(),
            strict_path_spans: Vec::new(),
            use_spans: Vec::new(),
        };
        visitor.visit_file(&record.ast);
        let strict_path_spans = visitor.strict_path_spans.clone();
        let top_level_use_spans: Vec<LineColumn> = record
            .ast
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Use(use_item) => Some(use_item.span().start()),
                _ => None,
            })
            .collect();
        for use_span in visitor.use_spans {
            if !top_level_use_spans.contains(&use_span) {
                issues.push(AnalysisIssue {
                    code: "analysis.unsupported-name-resolution".to_owned(),
                    reason:
                        "block-scoped use declarations are outside the MVP name-resolution boundary"
                            .to_owned(),
                    affected_scope: format!("{}:{}", record.id, use_span.line),
                });
            }
        }
        for macro_span in visitor.macro_spans {
            issues.push(AnalysisIssue {
                code: "analysis.unsupported-macro-expansion".to_owned(),
                reason: "macro expansion is outside the MVP observer boundary".to_owned(),
                affected_scope: format!("{}:{}", record.id, macro_span.line),
            });
        }
        for cfg_span in visitor.cfg_spans {
            issues.push(AnalysisIssue {
                code: "analysis.unsupported-analysis-context".to_owned(),
                reason: "cfg and feature conditions are not evaluated by the MVP observer"
                    .to_owned(),
                affected_scope: format!("{}:{}", record.id, cfg_span.line),
            });
        }
        for path in visitor.paths {
            let strict_single_segment = strict_path_spans.contains(&path.span().start());
            let path_segments = syn_path_segments(&path);
            let Some((target, scope)) = self.resolve_path(
                &record.id,
                &path_segments,
                path.leading_colon.is_some(),
                all_modules,
            ) else {
                let explicit_relative_path = matches!(
                    path_segments.first().map(String::as_str),
                    Some("crate" | "self" | "super")
                );
                if explicit_relative_path || path_segments.len() > 1 || strict_single_segment {
                    issues.push(AnalysisIssue {
                        code: if path_segments
                            .first()
                            .is_some_and(|first| self.context_dependent_crates.contains(first))
                        {
                            "analysis.unsupported-analysis-context"
                        } else if explicit_relative_path {
                            "analysis.unresolved-reference"
                        } else {
                            "analysis.unsupported-name-resolution"
                        }
                        .to_owned(),
                        reason: format!(
                            "unable to resolve path {}",
                            path_to_string(&path_segments)
                        ),
                        affected_scope: record.id.clone(),
                    });
                }
                continue;
            };
            if scope == TargetScope::InternalModule && target == record.id {
                continue;
            }
            if scope == TargetScope::InternalModule && !selected.contains(&target) {
                continue;
            }
            let kind = DependencyKind::PathReference;
            let evidence = self.evidence(&record.path, path.span().start());
            dependencies.push(Dependency {
                candidate_id: candidate_id(&record.id, &target, kind, &evidence),
                source: record.id.clone(),
                target,
                kind,
                target_scope: scope,
                evidence,
            });
        }
        (dependencies, issues)
    }

    fn collect_use(
        &self,
        record: &ModuleRecord,
        use_item: &ItemUse,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
        dependencies: &mut Vec<Dependency>,
        issues: &mut Vec<AnalysisIssue>,
    ) {
        let kind = if matches!(use_item.vis, syn::Visibility::Public(_)) {
            DependencyKind::ReExport
        } else {
            DependencyKind::Import
        };
        let mut paths = Vec::new();
        flatten_use_tree(&use_item.tree, Vec::new(), &mut paths);
        for path in paths {
            let resolved = self
                .resolve_path(
                    &record.id,
                    &path,
                    use_item.leading_colon.is_some(),
                    all_modules,
                )
                .or_else(|| {
                    path.first()
                        .filter(|first| {
                            known_external_crate(first) || self.external_crates.contains(*first)
                        })
                        .map(|first| (first.clone(), TargetScope::ExternalCrate))
                });
            let Some((target, scope)) = resolved else {
                let explicit_relative_path = matches!(
                    path.first().map(String::as_str),
                    Some("crate" | "self" | "super")
                );
                issues.push(AnalysisIssue {
                    code: if path
                        .first()
                        .is_some_and(|first| self.context_dependent_crates.contains(first))
                    {
                        "analysis.unsupported-analysis-context"
                    } else if explicit_relative_path {
                        "analysis.unresolved-reference"
                    } else {
                        "analysis.unsupported-name-resolution"
                    }
                    .to_owned(),
                    reason: format!("unable to resolve use path {}", path_to_string(&path)),
                    affected_scope: record.id.clone(),
                });
                continue;
            };
            if scope == TargetScope::InternalModule && target == record.id {
                continue;
            }
            if scope == TargetScope::InternalModule && !selected.contains(&target) {
                continue;
            }
            let evidence = self.evidence(&record.path, use_item.span().start());
            dependencies.push(Dependency {
                candidate_id: candidate_id(&record.id, &target, kind, &evidence),
                source: record.id.clone(),
                target,
                kind,
                target_scope: scope,
                evidence,
            });
        }
    }

    fn resolve_path(
        &self,
        source_module: &str,
        path: &[String],
        absolute_external: bool,
        all_modules: &BTreeSet<String>,
    ) -> Option<(String, TargetScope)> {
        self.resolve_path_inner(source_module, path, absolute_external, all_modules, 0)
    }

    fn resolve_path_inner(
        &self,
        source_module: &str,
        path: &[String],
        absolute_external: bool,
        all_modules: &BTreeSet<String>,
        alias_depth: usize,
    ) -> Option<(String, TargetScope)> {
        if path.is_empty() {
            return None;
        }
        if absolute_external {
            let first = &path[0];
            return (known_external_crate(first) || self.external_crates.contains(first))
                .then(|| (first.clone(), TargetScope::ExternalCrate));
        }
        if alias_depth > self.records.len() {
            return None;
        }
        if let Some(alias) = self
            .records
            .get(source_module)
            .and_then(|record| record.use_aliases.get(&path[0]))
        {
            let mut expanded = alias.path.clone();
            expanded.extend_from_slice(&path[1..]);
            return self.resolve_path_inner(
                source_module,
                &expanded,
                alias.absolute_external,
                all_modules,
                alias_depth + 1,
            );
        }
        let (base, remainder) = match path[0].as_str() {
            "crate" => ("crate".to_owned(), &path[1..]),
            "self" => (source_module.to_owned(), &path[1..]),
            "super" => {
                let mut current = source_module.split("::").collect::<Vec<_>>();
                let super_count = path
                    .iter()
                    .take_while(|segment| segment.as_str() == "super")
                    .count();
                if current.len() <= super_count {
                    return None;
                }
                current.truncate(current.len() - super_count);
                (current.join("::"), &path[super_count..])
            }
            first => {
                let source_parts = source_module.split("::").collect::<Vec<_>>();
                let candidates = if source_parts.len() > 1 {
                    vec![
                        format!("{source_module}::{first}"),
                        format!("crate::{first}"),
                    ]
                } else {
                    vec![format!("crate::{first}")]
                };
                if let Some(local) = candidates.into_iter().find(|candidate| {
                    all_modules.iter().any(|module| {
                        module == candidate || module.starts_with(&(candidate.clone() + "::"))
                    })
                }) {
                    (local, &path[1..])
                } else {
                    if path.len() == 1
                        || !(known_external_crate(first) || self.external_crates.contains(first))
                    {
                        return None;
                    }
                    return Some((first.to_owned(), TargetScope::ExternalCrate));
                }
            }
        };
        let mut candidate = base;
        if all_modules.contains(&candidate) {
            for (index, segment) in remainder.iter().enumerate() {
                let next = format!("{candidate}::{segment}");
                if all_modules.contains(&next) {
                    candidate = next;
                } else {
                    if segment == "self" && index + 1 == remainder.len() {
                        break;
                    }
                    return self
                        .records
                        .get(&candidate)
                        .is_some_and(|record| record.declared_items.contains(segment))
                        .then_some((candidate, TargetScope::InternalModule));
                }
            }
            return Some((candidate, TargetScope::InternalModule));
        }
        let external = candidate
            .split("::")
            .next()
            .unwrap_or(&candidate)
            .to_owned();
        Some((external, TargetScope::ExternalCrate))
    }

    fn evidence(&self, path: &Path, start: LineColumn) -> SourceEvidence {
        let relative = path
            .strip_prefix(&self.project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let column = fs::read_to_string(path)
            .ok()
            .and_then(|source| {
                source
                    .lines()
                    .nth(start.line.saturating_sub(1))
                    .map(str::to_owned)
            })
            .map(|line| {
                line.char_indices()
                    .take_while(|(offset, _)| *offset < start.column)
                    .count()
                    + 1
            })
            .unwrap_or(start.column + 1);
        SourceEvidence {
            path: relative,
            line: start.line,
            column,
        }
    }
}

fn known_external_crate(name: &str) -> bool {
    matches!(name, "alloc" | "core" | "proc_macro" | "std" | "test")
}

fn known_builtin_attribute(path: &syn::Path) -> bool {
    path.get_ident().is_some_and(|name| {
        matches!(
            name.to_string().as_str(),
            "allow"
                | "cold"
                | "deprecated"
                | "deny"
                | "doc"
                | "export_name"
                | "forbid"
                | "global_allocator"
                | "ignore"
                | "inline"
                | "link"
                | "link_name"
                | "link_section"
                | "macro_export"
                | "macro_use"
                | "must_use"
                | "no_main"
                | "no_mangle"
                | "no_std"
                | "non_exhaustive"
                | "panic_handler"
                | "path"
                | "proc_macro"
                | "proc_macro_attribute"
                | "proc_macro_derive"
                | "repr"
                | "should_panic"
                | "test"
                | "warn"
        )
    })
}

fn declared_external_crates(
    project_root: &Path,
    context: &AnalysisContext,
) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let manifest_path = project_root.join("Cargo.toml");
    let content = fs::read_to_string(manifest_path)?;
    let manifest: toml::Value = content
        .parse()
        .map_err(|error| CruiserError::Parse(format!("invalid Cargo.toml: {error}")))?;
    validate_manifest_dependency_shapes(&manifest)?;
    let mut crates = BTreeSet::new();
    let mut context_dependent = BTreeSet::new();
    collect_dependency_names(
        dependency_table(manifest.get("dependencies"), "dependencies")?,
        &mut crates,
        &mut context_dependent,
    )?;
    if is_dev_compilation_target(&context.compilation_target) {
        collect_dependency_names(
            dependency_table(manifest.get("dev-dependencies"), "dev-dependencies")?,
            &mut crates,
            &mut context_dependent,
        )?;
    }
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
                collect_dependency_names(
                    dependency_table(
                        target.get("dependencies"),
                        &format!("target.{selector}.dependencies"),
                    )?,
                    &mut crates,
                    &mut context_dependent,
                )?;
                if is_dev_compilation_target(&context.compilation_target) {
                    collect_dependency_names(
                        dependency_table(
                            target.get("dev-dependencies"),
                            &format!("target.{selector}.dev-dependencies"),
                        )?,
                        &mut crates,
                        &mut context_dependent,
                    )?;
                }
            } else if selector.starts_with("cfg(") {
                collect_all_dependency_names(target, selector, &mut context_dependent)?;
            }
        }
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

fn collect_dependency_names(
    table: Option<&toml::value::Table>,
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
        if optional {
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
        collect_dependency_names(table, &mut active, &mut conditional)?;
        names.extend(active);
        names.extend(conditional);
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

struct PathVisitor {
    paths: Vec<syn::Path>,
    macro_spans: Vec<LineColumn>,
    cfg_spans: Vec<LineColumn>,
    strict_path_spans: Vec<LineColumn>,
    use_spans: Vec<LineColumn>,
}

impl<'ast> Visit<'ast> for PathVisitor {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.paths.push(path.clone());
        syn::visit::visit_path(self, path);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        // Inline module bodies are observed through their own ModuleRecord.
        for attribute in &module.attrs {
            self.visit_attribute(attribute);
        }
    }

    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        self.strict_path_spans.push(path.path.span().start());
        syn::visit::visit_type_path(self, path);
    }

    fn visit_trait_bound(&mut self, bound: &'ast syn::TraitBound) {
        self.strict_path_spans.push(bound.path.span().start());
        syn::visit::visit_trait_bound(self, bound);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if let Some((_, path, _)) = &item.trait_ {
            self.strict_path_spans.push(path.span().start());
        }
        syn::visit::visit_item_impl(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        self.use_spans.push(item.span().start());
        syn::visit::visit_item_use(self, item);
    }

    fn visit_macro(&mut self, macro_item: &'ast syn::Macro) {
        self.macro_spans.push(macro_item.span().start());
        syn::visit::visit_macro(self, macro_item);
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr") {
            self.cfg_spans.push(attribute.span().start());
        } else if attribute.path().is_ident("derive") || !known_builtin_attribute(attribute.path())
        {
            self.macro_spans.push(attribute.span().start());
        }
        syn::visit::visit_attribute(self, attribute);
    }
}

fn flatten_use_tree(tree: &UseTree, prefix: Vec<String>, paths: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            flatten_use_tree(&path.tree, next, paths);
        }
        UseTree::Name(name) => {
            let mut path = prefix;
            path.push(name.ident.to_string());
            paths.push(path);
        }
        UseTree::Rename(rename) => {
            let mut path = prefix;
            path.push(rename.ident.to_string());
            paths.push(path);
        }
        UseTree::Glob(_) => paths.push(prefix),
        UseTree::Group(group) => {
            for item in &group.items {
                flatten_use_tree(item, prefix.clone(), paths);
            }
        }
    }
}

fn declared_item_names(file: &File) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in &file.items {
        let name = match item {
            Item::Const(item) => Some(item.ident.to_string()),
            Item::Enum(item) => Some(item.ident.to_string()),
            Item::ExternCrate(item) => Some(
                item.rename
                    .as_ref()
                    .map_or(&item.ident, |(_, rename)| rename)
                    .to_string(),
            ),
            Item::Fn(item) => Some(item.sig.ident.to_string()),
            Item::Macro(item) => item.ident.as_ref().map(ToString::to_string),
            Item::Mod(item) => Some(item.ident.to_string()),
            Item::Static(item) => Some(item.ident.to_string()),
            Item::Struct(item) => Some(item.ident.to_string()),
            Item::Trait(item) => Some(item.ident.to_string()),
            Item::TraitAlias(item) => Some(item.ident.to_string()),
            Item::Type(item) => Some(item.ident.to_string()),
            Item::Union(item) => Some(item.ident.to_string()),
            Item::Use(item) => {
                collect_use_bindings(&item.tree, Vec::new(), &mut names);
                None
            }
            _ => None,
        };
        if let Some(name) = name {
            names.insert(name);
        }
    }
    names
}

fn collect_use_bindings(tree: &UseTree, prefix: Vec<String>, names: &mut BTreeSet<String>) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_bindings(&path.tree, next, names);
        }
        UseTree::Name(name) if name.ident == "self" => {
            if let Some(binding) = prefix.last() {
                names.insert(binding.clone());
            }
        }
        UseTree::Name(name) => {
            names.insert(name.ident.to_string());
        }
        UseTree::Rename(rename) => {
            names.insert(rename.rename.to_string());
        }
        UseTree::Glob(_) => {}
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_bindings(item, prefix.clone(), names);
            }
        }
    }
}

fn use_aliases(file: &File) -> BTreeMap<String, UseAlias> {
    let mut aliases = BTreeMap::new();
    for item in &file.items {
        if let Item::Use(item) = item {
            collect_use_aliases(
                &item.tree,
                Vec::new(),
                item.leading_colon.is_some(),
                &mut aliases,
            );
        }
    }
    aliases
}

fn collect_use_aliases(
    tree: &UseTree,
    prefix: Vec<String>,
    absolute_external: bool,
    aliases: &mut BTreeMap<String, UseAlias>,
) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_aliases(&path.tree, next, absolute_external, aliases);
        }
        UseTree::Name(name) if name.ident == "self" => {
            if let Some(binding) = prefix.last() {
                aliases.insert(
                    binding.clone(),
                    UseAlias {
                        path: prefix,
                        absolute_external,
                    },
                );
            }
        }
        UseTree::Name(name) => {
            let mut path = prefix;
            path.push(name.ident.to_string());
            aliases.insert(
                name.ident.to_string(),
                UseAlias {
                    path,
                    absolute_external,
                },
            );
        }
        UseTree::Rename(rename) => {
            let mut path = prefix;
            path.push(rename.ident.to_string());
            aliases.insert(
                rename.rename.to_string(),
                UseAlias {
                    path,
                    absolute_external,
                },
            );
        }
        UseTree::Glob(_) => {}
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_aliases(item, prefix.clone(), absolute_external, aliases);
            }
        }
    }
}

fn has_path_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("path"))
}

fn candidate_id(
    source: &str,
    target: &str,
    kind: DependencyKind,
    evidence: &SourceEvidence,
) -> String {
    format!(
        "dep:{source}:{target}:{}:{}:{}:{}",
        kind.as_str(),
        evidence.path,
        evidence.line,
        evidence.column
    )
}

fn resolve_module_file(directory: &Path, name: &str) -> Option<PathBuf> {
    let flat = directory.join(format!("{name}.rs"));
    if flat.is_file() {
        return Some(flat);
    }
    let nested = directory.join(name).join("mod.rs");
    nested.is_file().then_some(nested)
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

fn target_entry(
    project_root: &Path,
    context: &AnalysisContext,
) -> Result<(PathBuf, Vec<AnalysisIssue>)> {
    let manifest_path = project_root.join("Cargo.toml");
    let content = fs::read_to_string(&manifest_path)?;
    let manifest: toml::Value = content
        .parse()
        .map_err(|error| CruiserError::Parse(format!("invalid Cargo.toml: {error}")))?;
    validate_manifest_target_shapes(&manifest)?;
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
            let explicit = manifest
                .get("lib")
                .map(|value| {
                    value.as_table().ok_or_else(|| {
                        CruiserError::InvalidInput(
                            "Cargo.toml lib target must be a table".to_owned(),
                        )
                    })
                })
                .transpose()?;
            if explicit.is_none() && !package_auto_enabled(&manifest, "autolib")? {
                return Err(CruiserError::InvalidInput(
                    "library target not found: lib".to_owned(),
                ));
            }
            match explicit.and_then(|target| target.get("path")) {
                Some(value) => project_root.join(value.as_str().ok_or_else(|| {
                    CruiserError::InvalidInput("Cargo.toml lib.path must be text".to_owned())
                })?),
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
                &manifest,
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
                &manifest,
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
        Err(CruiserError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("target source file not found: {}", path.display()),
        )))
    }
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
    let explicit = explicit_target(manifest, kind, name)?;
    if let Some(target) = explicit {
        collect_required_feature_issue(target, context, issues)?;
        if let Some(value) = target.get("path") {
            let path = value.as_str().ok_or_else(|| {
                CruiserError::InvalidInput(format!(
                    "Cargo.toml {kind} target {name}.path must be text"
                ))
            })?;
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
) -> Result<Option<&'a toml::value::Table>> {
    let Some(value) = manifest.get(kind) else {
        return Ok(None);
    };
    let targets = value.as_array().ok_or_else(|| {
        CruiserError::InvalidInput(format!("Cargo.toml {kind} targets must be an array"))
    })?;
    for target in targets {
        let target = target.as_table().ok_or_else(|| {
            CruiserError::InvalidInput(format!("Cargo.toml {kind} target must be a table"))
        })?;
        let target_name = target
            .get("name")
            .ok_or_else(|| {
                CruiserError::InvalidInput(format!("Cargo.toml {kind} target.name is required"))
            })?
            .as_str()
            .ok_or_else(|| {
                CruiserError::InvalidInput(format!("Cargo.toml {kind} target.name must be text"))
            })?;
        if target_name == name {
            return Ok(Some(target));
        }
    }
    Ok(None)
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
        issues.push(AnalysisIssue {
            code: "analysis.unsupported-analysis-context".to_owned(),
            reason: format!(
                "target {} requires inactive features: {}",
                context.compilation_target,
                missing.join(", ")
            ),
            affected_scope: context.compilation_target.clone(),
        });
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

fn path_to_string(path: &[String]) -> String {
    path.join("::")
}

fn syn_path_segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::path_to_string;

    #[test]
    fn path_segments_are_joined_with_rust_separator() {
        // Arrange
        let path = vec!["crate".to_owned(), "application".to_owned()];

        // Act
        let value = path_to_string(&path);

        // Assert
        assert_eq!(value, "crate::application");
    }
}
