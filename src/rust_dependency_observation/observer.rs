use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use proc_macro2::LineColumn;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{File, Item, ItemUse};

use super::{
    cargo_project,
    cfg_context::{CfgContext, CfgDecision},
    rust_syntax::{
        PathVisitor, SupportedItemMacroDefinition, UseAlias, UseBinding,
        declared_external_crate_aliases, declared_item_names, declared_macro_bindings,
        declared_self_crate_aliases, expand_supported_item_macro, flatten_use_tree, glob_imports,
        has_no_implicit_prelude, has_path_attribute, is_crate_visible, item_attributes,
        local_macro_bindings, local_macro_safety_bindings, path_to_string, public_glob_imports,
        public_item_names, public_use_bindings, supported_item_macro_definition, syn_path_segments,
        use_aliases,
    },
};

use crate::boundary_validation::{
    AnalysisContext, AnalysisIssue, AnalysisProfile, CruiserError, Dependency, DependencyKind,
    DependencyObserver, LogicalModule, ModuleDeclaration, ObservedDependencyGraph, Result,
    SourceEvidence, TargetScope, matches_pattern,
};

pub struct RustDependencyObserver;

impl DependencyObserver for RustDependencyObserver {
    fn target_source(&self, project_root: &Path, context: &AnalysisContext) -> Result<PathBuf> {
        let manifest = cargo_project::load_manifest(project_root)?;
        cargo_project::target_entry(project_root, &manifest, context).map(|(path, _)| path)
    }

    fn observe(
        &self,
        project_root: &Path,
        context: AnalysisContext,
    ) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
        observe(project_root, context)
    }
}

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

struct PathResolutionContext<'a> {
    source_module: &'a str,
    source_variant: &'a ModuleVariant,
    aliases: &'a BTreeMap<String, Vec<UseAlias>>,
    glob_imports: &'a [UseAlias],
    all_modules: &'a BTreeSet<String>,
    exported_items: &'a BTreeMap<String, BTreeSet<String>>,
}

struct ResolutionScope<'a> {
    aliases: &'a BTreeMap<String, Vec<UseAlias>>,
    glob_imports: &'a [UseAlias],
}

struct Observer {
    project_root: PathBuf,
    cfg_context: CfgContext,
    external_crates: BTreeSet<String>,
    context_dependent_crates: BTreeSet<String>,
    self_crate_aliases: BTreeSet<String>,
    records: BTreeMap<String, ModuleRecord>,
    exported_items: BTreeMap<String, BTreeSet<String>>,
    declarations: Vec<ModuleDeclaration>,
    issues: Vec<AnalysisIssue>,
}

#[derive(Clone)]
struct RegisteredItemMacro {
    module_id: String,
    definition: SupportedItemMacroDefinition,
}

struct ItemMacroRegistry {
    definitions: BTreeMap<(String, String), Vec<RegisteredItemMacro>>,
    exports: BTreeSet<(String, String)>,
}

fn observe(
    project_root: &Path,
    mut context: AnalysisContext,
) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
    context.features.sort();
    context.features.dedup();
    context.cfg.sort();
    context.cfg.dedup();
    let manifest = cargo_project::load_manifest(project_root)?;
    let cfg_context = CfgContext::resolve(&context);
    let (entry, target_issues) = cargo_project::target_entry(project_root, &manifest, &context)?;
    let (mut external_crates, context_dependent_crates) =
        cargo_project::declared_external_crates(&manifest, &context, &cfg_context)?;
    let target_kind = context
        .compilation_target
        .split_once(':')
        .map_or(context.compilation_target.as_str(), |(kind, _)| kind);
    if target_kind != "lib" && cargo_project::has_library_target(project_root, &manifest)? {
        // A binary/example/test target in the same package links the package
        // library under its crate name. The selected target remains the
        // observation boundary, so treat that library crate as external to
        // this graph rather than inventing a second source root.
        external_crates.insert(context.package.replace('-', "_"));
    }
    let mut observer = Observer {
        project_root: project_root.to_path_buf(),
        cfg_context,
        external_crates,
        context_dependent_crates,
        self_crate_aliases: BTreeSet::new(),
        records: BTreeMap::new(),
        exported_items: BTreeMap::new(),
        declarations: Vec::new(),
        issues: target_issues,
    };
    observer.discover_module("crate", &entry)?;
    if context.analysis_profile == AnalysisProfile::Compilation {
        observer.expand_supported_item_macros();
    }
    observer.self_crate_aliases = observer
        .records
        .get("crate")
        .into_iter()
        .flat_map(|record| record.variants.iter())
        .flat_map(|variant| variant.self_crate_aliases.iter().cloned())
        .collect();

    let all_module_ids: BTreeSet<String> = observer.records.keys().cloned().collect();
    let exported_items = observer.build_export_index(&all_module_ids);
    observer.exported_items = exported_items;
    let excluded_scopes: Vec<String> = all_module_ids
        .iter()
        .filter(|id| {
            context
                .exclude
                .iter()
                .any(|pattern| matches_pattern(pattern, id))
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
                    .any(|pattern| matches_pattern(pattern, id));
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
        let (record_dependencies, record_issues) = observer.collect_dependencies(
            &record_id,
            &selected,
            &all_module_ids,
            context.analysis_profile,
        );
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
    observer.issues.sort_by(AnalysisIssue::canonical_cmp);
    observer.issues.dedup();

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
    fn expand_supported_item_macros(&mut self) {
        let registry = ItemMacroRegistry::collect(&self.records, &self.cfg_context);
        let external_crates = self.external_crates.clone();
        for (module_id, record) in &mut self.records {
            for variant in &mut record.variants {
                let items = std::mem::take(&mut variant.ast.items);
                variant.ast.items =
                    expand_module_items(module_id, variant, items, &registry, &external_crates);
                refresh_variant_indexes(variant, &self.cfg_context);
            }
        }
    }

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
                let evidence = self.evidence(path, error.span().start());
                self.issues.push(AnalysisIssue::analysis_context_unresolved(
                    "analysis.unsupported-syntax",
                    format!("unable to parse {}: {error}", path.display()),
                    id,
                    Some(id.to_owned()),
                    Some(evidence),
                    all_dependency_kinds(),
                ));
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
                self.issues.push(AnalysisIssue::analysis_context_unresolved(
                    "analysis.unsupported-syntax",
                    "#[path] module declarations are not supported in the MVP",
                    child_id.clone(),
                    Some(child_id.clone()),
                    Some(evidence),
                    all_dependency_kinds(),
                ));
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
                let child_path =
                    resolve_module_file(parent_module_directory, &module.ident.to_string())
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
        analysis_profile: AnalysisProfile,
    ) -> (Vec<Dependency>, Vec<AnalysisIssue>) {
        let Some(record) = self.records.get(record_id) else {
            return (Vec::new(), Vec::new());
        };
        let mut dependencies = Vec::new();
        let mut issues = Vec::new();
        for variant in &record.variants {
            self.collect_variant_dependencies(
                &record.id,
                variant,
                selected,
                all_modules,
                analysis_profile,
                &mut dependencies,
                &mut issues,
            );
        }
        (dependencies, issues)
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_variant_dependencies(
        &self,
        source_id: &str,
        variant: &ModuleVariant,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
        analysis_profile: AnalysisProfile,
        dependencies: &mut Vec<Dependency>,
        issues: &mut Vec<AnalysisIssue>,
    ) {
        let top_level_scope = ResolutionScope {
            aliases: &variant.use_aliases,
            glob_imports: &variant.glob_imports,
        };
        for item in &variant.ast.items {
            if let Item::Use(use_item) = item
                && self.cfg_context.evaluate_attributes(&use_item.attrs) != CfgDecision::Inactive
            {
                let (use_dependencies, use_issues) = self.collect_use(
                    source_id,
                    variant,
                    use_item,
                    &top_level_scope,
                    selected,
                    all_modules,
                );
                dependencies.extend(use_dependencies);
                issues.extend(use_issues);
            }
        }
        let macro_bindings = declared_macro_bindings(&variant.ast, &self.cfg_context);
        let (safe_local_macro_bindings, observing_local_macro_bindings) =
            local_macro_safety_bindings(
                &variant.ast,
                &self.cfg_context,
                &variant.declared_items,
                variant.no_implicit_prelude,
            );
        let mut visitor = PathVisitor {
            cfg_context: &self.cfg_context,
            external_crates: &self.external_crates,
            observe_attribute_paths: analysis_profile == AnalysisProfile::Compilation,
            paths: Vec::new(),
            scoped_uses: Vec::new(),
            local_bindings: BTreeSet::new(),
            aliases: variant.use_aliases.clone(),
            glob_imports: variant.glob_imports.clone(),
            macro_bindings,
            local_macro_bindings: variant.local_macro_bindings.clone(),
            safe_local_macro_bindings,
            observing_local_macro_bindings,
            inactive_expr_cfg: false,
            in_expr: false,
            in_item_macro: false,
            cfg_guarded_macro: false,
            no_implicit_prelude: variant.no_implicit_prelude,
            macro_spans: Vec::new(),
            cfg_spans: Vec::new(),
            strict_path_spans: Vec::new(),
            use_spans: Vec::new(),
        };
        visitor.visit_file(&variant.ast);
        for scoped_use in &visitor.scoped_uses {
            let scoped_scope = ResolutionScope {
                aliases: &scoped_use.aliases,
                glob_imports: &scoped_use.glob_imports,
            };
            let (use_dependencies, use_issues) = self.collect_use(
                source_id,
                variant,
                &scoped_use.item,
                &scoped_scope,
                selected,
                all_modules,
            );
            dependencies.extend(use_dependencies);
            issues.extend(use_issues);
        }
        let strict_path_spans = visitor.strict_path_spans.clone();
        let top_level_use_spans: Vec<LineColumn> = variant
            .ast
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Use(use_item) => Some(use_item.span().start()),
                _ => None,
            })
            .collect();
        let _block_scoped_use_spans = visitor
            .use_spans
            .iter()
            .filter(|span| !top_level_use_spans.contains(span))
            .collect::<Vec<_>>();
        if analysis_profile == AnalysisProfile::Compilation {
            for macro_span in visitor.macro_spans {
                issues.push(AnalysisIssue::unresolved_dependency_candidate(
                    "analysis.unsupported-macro-expansion",
                    "macro expansion is outside the MVP observer boundary",
                    format!("{source_id}:{}", macro_span.line),
                    source_id.to_owned(),
                    self.evidence(&variant.path, macro_span),
                    Vec::new(),
                    all_dependency_kinds(),
                ));
            }
            for cfg_span in visitor.cfg_spans {
                issues.push(AnalysisIssue::analysis_context_unresolved(
                    "analysis.unsupported-analysis-context",
                    "cfg and feature conditions are not evaluated by the MVP observer",
                    format!("{source_id}:{}", cfg_span.line),
                    Some(source_id.to_owned()),
                    Some(self.evidence(&variant.path, cfg_span)),
                    all_dependency_kinds(),
                ));
            }
        }
        for observed_path in visitor.paths {
            let evidence_span = observed_path.evidence_span;
            let requires_complete_resolution = observed_path.requires_complete_resolution;
            let path = observed_path.path;
            let strict_single_segment = strict_path_spans.contains(&path.span().start());
            let path_segments = syn_path_segments(&path);
            if path_segments
                .first()
                .is_some_and(|first| observed_path.local_bindings.contains(first))
            {
                if requires_complete_resolution && analysis_profile == AnalysisProfile::Compilation
                {
                    issues.push(AnalysisIssue::unresolved_dependency_candidate(
                        "analysis.unsupported-name-resolution",
                        format!(
                            "unable to resolve attribute path {} because its root is shadowed",
                            path_to_string(&path_segments)
                        ),
                        source_id,
                        source_id.to_owned(),
                        self.evidence(&variant.path, evidence_span),
                        Vec::new(),
                        vec![DependencyKind::PathReference],
                    ));
                }
                continue;
            }
            let observed_scope = ResolutionScope {
                aliases: &observed_path.aliases,
                glob_imports: &observed_path.glob_imports,
            };
            let resolved = self.resolve_path(
                source_id,
                variant,
                &observed_scope,
                &path_segments,
                path.leading_colon.is_some(),
                all_modules,
            );
            let may_reference_internal = self.may_reference_internal_module(
                source_id,
                &observed_path.aliases,
                &observed_path.glob_imports,
                &path_segments,
                all_modules,
                0,
            );
            if resolved.is_empty()
                && path_segments.first().is_some_and(|first| {
                    known_language_name(first)
                        || (!variant.no_implicit_prelude && known_prelude_name(first))
                })
                && (!requires_complete_resolution || !may_reference_internal)
            {
                continue;
            }
            if resolved.is_empty() {
                let explicit_relative_path = matches!(
                    path_segments.first().map(String::as_str),
                    Some("crate" | "self" | "super")
                );
                let requires_complete_resolution = analysis_profile == AnalysisProfile::Compilation
                    && (requires_complete_resolution
                        || path_segments.len() > 1
                        || strict_single_segment);
                if explicit_relative_path || requires_complete_resolution || may_reference_internal
                {
                    let code = if path_segments
                        .first()
                        .is_some_and(|first| self.context_dependent_crates.contains(first))
                    {
                        "analysis.unsupported-analysis-context"
                    } else if explicit_relative_path {
                        "analysis.unresolved-reference"
                    } else {
                        "analysis.unsupported-name-resolution"
                    };
                    let reason =
                        format!("unable to resolve path {}", path_to_string(&path_segments));
                    let evidence = self.evidence(&variant.path, evidence_span);
                    let candidate_targets = self.candidate_targets_for_path(
                        source_id,
                        variant,
                        &observed_scope,
                        &path_segments,
                        path.leading_colon.is_some(),
                        all_modules,
                    );
                    if code == "analysis.unsupported-analysis-context" {
                        issues.push(AnalysisIssue::analysis_context_unresolved(
                            code,
                            reason,
                            source_id,
                            Some(source_id.to_owned()),
                            Some(evidence),
                            vec![DependencyKind::PathReference],
                        ));
                    } else {
                        issues.push(AnalysisIssue::unresolved_dependency_candidate(
                            code,
                            reason,
                            source_id,
                            source_id.to_owned(),
                            evidence,
                            candidate_targets,
                            vec![DependencyKind::PathReference],
                        ));
                    }
                }
                continue;
            }
            let declared_item_shadow = requires_complete_resolution
                && path_segments.len() > 1
                && path.leading_colon.is_none()
                && path_segments.first().is_some_and(|first| {
                    !matches!(first.as_str(), "crate" | "self" | "super")
                        && !observed_path.aliases.contains_key(first)
                        && variant.declared_items.contains(first)
                })
                && resolved.iter().all(|(target, scope)| {
                    *scope == TargetScope::InternalModule && target == source_id
                });
            let mut distinct_resolutions = resolved.clone();
            distinct_resolutions.sort_by(|left, right| {
                left.0.cmp(&right.0).then_with(|| {
                    let rank = |scope| match scope {
                        TargetScope::InternalModule => 0,
                        TargetScope::ExternalCrate => 1,
                    };
                    rank(left.1).cmp(&rank(right.1))
                })
            });
            distinct_resolutions.dedup();
            if requires_complete_resolution
                && analysis_profile == AnalysisProfile::Compilation
                && (declared_item_shadow || distinct_resolutions.len() > 1)
            {
                let candidate_targets = distinct_resolutions
                    .iter()
                    .filter_map(|(target, scope)| {
                        (*scope == TargetScope::InternalModule && target != source_id)
                            .then_some(target.clone())
                    })
                    .collect();
                issues.push(AnalysisIssue::unresolved_dependency_candidate(
                    "analysis.unsupported-name-resolution",
                    format!(
                        "unable to prove a unique resolution for attribute path {}",
                        path_to_string(&path_segments)
                    ),
                    source_id,
                    source_id.to_owned(),
                    self.evidence(&variant.path, evidence_span),
                    candidate_targets,
                    vec![DependencyKind::PathReference],
                ));
                continue;
            }
            let kind = DependencyKind::PathReference;
            let evidence = self.evidence(&variant.path, evidence_span);
            for (target, scope) in resolved {
                if scope == TargetScope::InternalModule && target == source_id {
                    continue;
                }
                if scope == TargetScope::InternalModule && !selected.contains(&target) {
                    continue;
                }
                dependencies.push(Dependency {
                    candidate_id: candidate_id(source_id, &target, kind, &evidence),
                    source: source_id.to_owned(),
                    target,
                    kind,
                    target_scope: scope,
                    evidence: evidence.clone(),
                });
            }
        }
    }

    fn may_reference_internal_module(
        &self,
        source_module: &str,
        aliases: &BTreeMap<String, Vec<UseAlias>>,
        glob_imports: &[UseAlias],
        path: &[String],
        all_modules: &BTreeSet<String>,
        alias_depth: usize,
    ) -> bool {
        let Some(first) = path.first() else {
            return false;
        };
        if matches!(first.as_str(), "crate" | "self" | "super") {
            return true;
        }
        if alias_depth <= self.records.len()
            && let Some(candidates) = aliases.get(first)
        {
            return candidates.iter().any(|alias| {
                let mut expanded = alias.path.clone();
                expanded.extend_from_slice(&path[1..]);
                !alias.absolute_external
                    && self.may_reference_internal_module(
                        source_module,
                        aliases,
                        glob_imports,
                        &expanded,
                        all_modules,
                        alias_depth + 1,
                    )
            });
        }
        if !glob_imports.is_empty() {
            return true;
        }
        let local = format!("{source_module}::{first}");
        let crate_level = format!("crate::{first}");
        all_modules.iter().any(|module| {
            module == &local
                || module.starts_with(&(local.clone() + "::"))
                || module == &crate_level
                || module.starts_with(&(crate_level.clone() + "::"))
        })
    }

    fn collect_use(
        &self,
        source_id: &str,
        source_variant: &ModuleVariant,
        use_item: &ItemUse,
        resolution_scope: &ResolutionScope<'_>,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
    ) -> (Vec<Dependency>, Vec<AnalysisIssue>) {
        let mut dependencies = Vec::new();
        let mut issues = Vec::new();
        let kind = if is_crate_visible(&use_item.vis) {
            DependencyKind::ReExport
        } else {
            DependencyKind::Import
        };
        let mut paths = Vec::new();
        flatten_use_tree(&use_item.tree, Vec::new(), &mut paths);
        for path in paths {
            if path.len() == 1 && source_variant.local_macro_bindings.contains(&path[0]) {
                continue;
            }
            let mut resolved = self.resolve_path(
                source_id,
                source_variant,
                resolution_scope,
                &path,
                use_item.leading_colon.is_some(),
                all_modules,
            );
            if resolved.is_empty()
                && let Some(first) = path.first().filter(|first| {
                    known_external_crate(first) || self.external_crates.contains(*first)
                })
            {
                resolved.push((first.clone(), TargetScope::ExternalCrate));
            }
            if resolved.is_empty() {
                let explicit_relative_path = matches!(
                    path.first().map(String::as_str),
                    Some("crate" | "self" | "super")
                );
                let code = if path
                    .first()
                    .is_some_and(|first| self.context_dependent_crates.contains(first))
                {
                    "analysis.unsupported-analysis-context"
                } else if explicit_relative_path {
                    "analysis.unresolved-reference"
                } else {
                    "analysis.unsupported-name-resolution"
                };
                let reason = format!("unable to resolve use path {}", path_to_string(&path));
                let evidence = self.evidence(&source_variant.path, use_item.span().start());
                let candidate_targets = self.candidate_targets_for_path(
                    source_id,
                    source_variant,
                    resolution_scope,
                    &path,
                    use_item.leading_colon.is_some(),
                    all_modules,
                );
                if code == "analysis.unsupported-analysis-context" {
                    issues.push(AnalysisIssue::analysis_context_unresolved(
                        code,
                        reason,
                        source_id,
                        Some(source_id.to_owned()),
                        Some(evidence),
                        vec![kind],
                    ));
                } else {
                    issues.push(AnalysisIssue::unresolved_dependency_candidate(
                        code,
                        reason,
                        source_id,
                        source_id.to_owned(),
                        evidence,
                        candidate_targets,
                        vec![kind],
                    ));
                }
                continue;
            }
            let evidence = self.evidence(&source_variant.path, use_item.span().start());
            for (target, scope) in resolved {
                if scope == TargetScope::InternalModule && target == source_id {
                    continue;
                }
                if scope == TargetScope::InternalModule && !selected.contains(&target) {
                    continue;
                }
                dependencies.push(Dependency {
                    candidate_id: candidate_id(source_id, &target, kind, &evidence),
                    source: source_id.to_owned(),
                    target,
                    kind,
                    target_scope: scope,
                    evidence: evidence.clone(),
                });
            }
        }
        (dependencies, issues)
    }

    fn candidate_targets_for_path(
        &self,
        source_id: &str,
        source_variant: &ModuleVariant,
        resolution_scope: &ResolutionScope<'_>,
        path: &[String],
        leading_colon: bool,
        all_modules: &BTreeSet<String>,
    ) -> Vec<String> {
        let mut candidates = BTreeSet::new();
        for end in (1..path.len()).rev() {
            for (target, scope) in self.resolve_path(
                source_id,
                source_variant,
                resolution_scope,
                &path[..end],
                leading_colon,
                all_modules,
            ) {
                if scope == TargetScope::InternalModule && target != source_id {
                    candidates.insert(target);
                }
            }
            if !candidates.is_empty() {
                break;
            }
        }
        if candidates.is_empty() {
            for glob in resolution_scope.glob_imports {
                for (target, target_scope) in self.resolve_path(
                    source_id,
                    source_variant,
                    &ResolutionScope {
                        aliases: resolution_scope.aliases,
                        glob_imports: &[],
                    },
                    &glob.path,
                    glob.absolute_external,
                    all_modules,
                ) {
                    if target_scope == TargetScope::InternalModule && target != source_id {
                        candidates.insert(target);
                    }
                }
            }
        }
        candidates.into_iter().collect()
    }

    fn resolve_path(
        &self,
        source_module: &str,
        source_variant: &ModuleVariant,
        scope: &ResolutionScope<'_>,
        path: &[String],
        absolute_external: bool,
        all_modules: &BTreeSet<String>,
    ) -> Vec<(String, TargetScope)> {
        self.resolve_path_with_exports(
            source_module,
            source_variant,
            scope.aliases,
            scope.glob_imports,
            path,
            absolute_external,
            all_modules,
            &self.exported_items,
        )
    }

    // Export-index construction uses the same explicit context with a partially built index.
    #[allow(clippy::too_many_arguments)]
    fn resolve_path_with_exports(
        &self,
        source_module: &str,
        source_variant: &ModuleVariant,
        aliases: &BTreeMap<String, Vec<UseAlias>>,
        glob_imports: &[UseAlias],
        path: &[String],
        absolute_external: bool,
        all_modules: &BTreeSet<String>,
        exported_items: &BTreeMap<String, BTreeSet<String>>,
    ) -> Vec<(String, TargetScope)> {
        let context = PathResolutionContext {
            source_module,
            source_variant,
            aliases,
            glob_imports,
            all_modules,
            exported_items,
        };
        self.resolve_path_inner(&context, path, absolute_external, 0)
    }

    fn resolve_path_inner(
        &self,
        context: &PathResolutionContext<'_>,
        path: &[String],
        absolute_external: bool,
        alias_depth: usize,
    ) -> Vec<(String, TargetScope)> {
        if path.is_empty() {
            return Vec::new();
        }
        if absolute_external {
            let first = &path[0];
            return if known_external_crate(first) || self.external_crates.contains(first) {
                vec![(first.clone(), TargetScope::ExternalCrate)]
            } else {
                Vec::new()
            };
        }
        if alias_depth > self.records.len() {
            return Vec::new();
        }
        if self.self_crate_aliases.contains(&path[0])
            || context.source_variant.self_crate_aliases.contains(&path[0])
        {
            let mut crate_path = vec!["crate".to_owned()];
            crate_path.extend_from_slice(&path[1..]);
            return self.resolve_path_inner(context, &crate_path, false, alias_depth + 1);
        }
        if let Some(candidates) = context.aliases.get(&path[0]) {
            return candidates
                .iter()
                .flat_map(|alias| {
                    let mut expanded = alias.path.clone();
                    expanded.extend_from_slice(&path[1..]);
                    self.resolve_path_inner(
                        context,
                        &expanded,
                        alias.absolute_external,
                        alias_depth + 1,
                    )
                })
                .collect();
        }
        let (base, remainder) = match path[0].as_str() {
            "crate" => ("crate".to_owned(), &path[1..]),
            "self" => (context.source_module.to_owned(), &path[1..]),
            "super" => {
                let mut current = context.source_module.split("::").collect::<Vec<_>>();
                let super_count = path
                    .iter()
                    .take_while(|segment| segment.as_str() == "super")
                    .count();
                if current.len() <= super_count {
                    return Vec::new();
                }
                current.truncate(current.len() - super_count);
                (current.join("::"), &path[super_count..])
            }
            first => {
                let source_parts = context.source_module.split("::").collect::<Vec<_>>();
                let candidates = if source_parts.len() > 1 {
                    vec![
                        format!("{}::{first}", context.source_module),
                        format!("crate::{first}"),
                    ]
                } else {
                    vec![format!("crate::{first}")]
                };
                if let Some(local) = candidates.into_iter().find(|candidate| {
                    context.all_modules.iter().any(|module| {
                        module == candidate || module.starts_with(&(candidate.clone() + "::"))
                    })
                }) {
                    (local, &path[1..])
                } else if path.len() > 1
                    && context
                        .source_variant
                        .external_crate_aliases
                        .contains(first)
                {
                    return vec![(first.to_owned(), TargetScope::ExternalCrate)];
                } else if first == "Self" || context.source_variant.declared_items.contains(first) {
                    return vec![(
                        context.source_module.to_owned(),
                        TargetScope::InternalModule,
                    )];
                } else if !context.glob_imports.is_empty()
                    && let Some(resolved) = self.resolve_glob_path(context, path, alias_depth)
                {
                    return resolved;
                } else {
                    let is_external_crate = known_external_crate(first)
                        || self.external_crates.contains(first)
                        || context
                            .source_variant
                            .external_crate_aliases
                            .contains(first);
                    if path.len() == 1 || !is_external_crate {
                        return Vec::new();
                    }
                    return vec![(first.to_owned(), TargetScope::ExternalCrate)];
                }
            }
        };
        let mut candidate = base;
        if context.all_modules.contains(&candidate) {
            for (index, segment) in remainder.iter().enumerate() {
                let next = format!("{candidate}::{segment}");
                if context.all_modules.contains(&next) {
                    candidate = next;
                } else {
                    if segment == "self" && index + 1 == remainder.len() {
                        break;
                    }
                    let declared_item = self.records.get(&candidate).is_some_and(|record| {
                        record
                            .variants
                            .iter()
                            .any(|variant| variant.declared_items.contains(segment))
                    }) || context
                        .exported_items
                        .get(&candidate)
                        .is_some_and(|items| items.contains(segment));
                    return if declared_item || alias_depth > 0 {
                        vec![(candidate, TargetScope::InternalModule)]
                    } else {
                        Vec::new()
                    };
                }
            }
            return vec![(candidate, TargetScope::InternalModule)];
        }
        let external = candidate
            .split("::")
            .next()
            .unwrap_or(&candidate)
            .to_owned();
        vec![(external, TargetScope::ExternalCrate)]
    }

    fn resolve_glob_path(
        &self,
        context: &PathResolutionContext<'_>,
        path: &[String],
        alias_depth: usize,
    ) -> Option<Vec<(String, TargetScope)>> {
        if alias_depth >= self.records.len() {
            return None;
        }
        let mut resolved = Vec::new();
        let direct_context = PathResolutionContext {
            source_module: context.source_module,
            source_variant: context.source_variant,
            aliases: context.aliases,
            glob_imports: &[],
            all_modules: context.all_modules,
            exported_items: context.exported_items,
        };
        for glob in context.glob_imports {
            let mut expanded = glob.path.clone();
            expanded.extend_from_slice(path);
            resolved.extend(self.resolve_path_inner(
                &direct_context,
                &expanded,
                glob.absolute_external,
                0,
            ));
        }
        resolved.sort_by(|left, right| {
            left.0.cmp(&right.0).then_with(|| {
                let rank = |scope| match scope {
                    TargetScope::InternalModule => 0,
                    TargetScope::ExternalCrate => 1,
                };
                rank(left.1).cmp(&rank(right.1))
            })
        });
        resolved.dedup();
        (!resolved.is_empty()).then_some(resolved)
    }

    fn build_export_index(
        &self,
        all_modules: &BTreeSet<String>,
    ) -> BTreeMap<String, BTreeSet<String>> {
        let mut exported_items = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        for module_id in all_modules {
            self.collect_exported_items(module_id, all_modules, &mut exported_items, &mut visiting);
        }
        exported_items
    }

    fn collect_exported_items(
        &self,
        module_id: &str,
        all_modules: &BTreeSet<String>,
        exported_items: &mut BTreeMap<String, BTreeSet<String>>,
        visiting: &mut BTreeSet<String>,
    ) -> BTreeSet<String> {
        if let Some(items) = exported_items.get(module_id) {
            return items.clone();
        }
        if !visiting.insert(module_id.to_owned()) {
            return BTreeSet::new();
        }
        let mut items = BTreeSet::new();
        if let Some(record) = self.records.get(module_id) {
            for variant in &record.variants {
                items.extend(variant.public_items.iter().cloned());
                for binding in &variant.public_use_bindings {
                    if self.public_use_binding_is_visible(
                        module_id,
                        variant,
                        binding,
                        all_modules,
                        exported_items,
                        visiting,
                    ) {
                        items.insert(binding.name.clone());
                    }
                }
                let glob_imports = variant.public_glob_imports.clone();
                for glob in glob_imports {
                    let targets = self.resolve_path_with_exports(
                        module_id,
                        variant,
                        &variant.use_aliases,
                        &variant.glob_imports,
                        &glob.path,
                        glob.absolute_external,
                        all_modules,
                        exported_items,
                    );
                    for (target, scope) in targets {
                        if scope == TargetScope::InternalModule && target != module_id {
                            items.extend(self.collect_exported_items(
                                &target,
                                all_modules,
                                exported_items,
                                visiting,
                            ));
                        }
                    }
                }
            }
        }
        visiting.remove(module_id);
        exported_items.insert(module_id.to_owned(), items.clone());
        items
    }

    fn public_use_binding_is_visible(
        &self,
        module_id: &str,
        variant: &ModuleVariant,
        binding: &UseBinding,
        all_modules: &BTreeSet<String>,
        exported_items: &mut BTreeMap<String, BTreeSet<String>>,
        visiting: &mut BTreeSet<String>,
    ) -> bool {
        let Some(name) = binding.path.last() else {
            return false;
        };
        if binding.path.len() == 1 {
            return variant.public_items.contains(name);
        }
        let parent_path = &binding.path[..binding.path.len() - 1];
        let targets = self.resolve_path_with_exports(
            module_id,
            variant,
            &variant.use_aliases,
            &variant.glob_imports,
            parent_path,
            binding.absolute_external,
            all_modules,
            exported_items,
        );
        targets.into_iter().any(|(target, scope)| {
            if scope != TargetScope::InternalModule {
                return true;
            }
            if target == module_id && variant.public_items.contains(name) {
                return true;
            }
            if all_modules.contains(&format!("{target}::{name}")) {
                return true;
            }
            self.collect_exported_items(&target, all_modules, exported_items, visiting)
                .contains(name)
        })
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

impl ItemMacroRegistry {
    fn collect(records: &BTreeMap<String, ModuleRecord>, cfg_context: &CfgContext) -> Self {
        let mut definitions: BTreeMap<(String, String), Vec<RegisteredItemMacro>> = BTreeMap::new();
        for (module_id, record) in records {
            for variant in &record.variants {
                for item in &variant.ast.items {
                    let Item::Macro(item_macro) = item else {
                        continue;
                    };
                    let Some(name) = item_macro.ident.as_ref().map(ToString::to_string) else {
                        continue;
                    };
                    let Some(definition) = supported_item_macro_definition(item_macro, cfg_context)
                    else {
                        continue;
                    };
                    definitions
                        .entry((module_id.clone(), name))
                        .or_default()
                        .push(RegisteredItemMacro {
                            module_id: module_id.clone(),
                            definition,
                        });
                }
            }
        }
        let mut exports = BTreeSet::new();
        for (module_id, record) in records {
            for variant in &record.variants {
                for binding in &variant.public_use_bindings {
                    if binding.path.len() == 1
                        && definitions.contains_key(&(module_id.clone(), binding.path[0].clone()))
                    {
                        exports.insert((module_id.clone(), binding.name.clone()));
                    }
                }
            }
        }
        Self {
            definitions,
            exports,
        }
    }

    fn resolve(
        &self,
        current_module: &str,
        variant: &ModuleVariant,
        path: &syn::Path,
    ) -> Option<&RegisteredItemMacro> {
        if path.leading_colon.is_some() {
            return None;
        }
        let segments = syn_path_segments(path);
        let name = segments.last()?.clone();
        let mut candidates = Vec::new();
        if segments.len() == 1 {
            if self
                .definitions
                .contains_key(&(current_module.to_owned(), name.clone()))
            {
                candidates.push((current_module.to_owned(), name.clone()));
            }
            for alias in variant.use_aliases.get(&name).into_iter().flatten() {
                if alias.absolute_external || alias.path.len() < 2 {
                    continue;
                }
                let macro_name = alias.path.last()?.clone();
                let target_module =
                    resolve_macro_module(current_module, &alias.path[..alias.path.len() - 1])?;
                if self
                    .exports
                    .contains(&(target_module.clone(), macro_name.clone()))
                    && self
                        .definitions
                        .contains_key(&(target_module.clone(), macro_name.clone()))
                {
                    candidates.push((target_module, macro_name));
                }
            }
        } else {
            let target_module =
                resolve_macro_module(current_module, &segments[..segments.len() - 1])?;
            if target_module != current_module
                && !self
                    .exports
                    .contains(&(target_module.clone(), name.clone()))
            {
                return None;
            }
            candidates.push((target_module, name));
        }
        let [candidate] = candidates.as_slice() else {
            return None;
        };
        let definitions = self.definitions.get(candidate)?;
        (definitions.len() == 1).then_some(&definitions[0])
    }
}

fn resolve_macro_module(current_module: &str, path: &[String]) -> Option<String> {
    if path.is_empty() {
        return Some(current_module.to_owned());
    }
    let mut module = current_module
        .split("::")
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut index = 0;
    match path.first().map(String::as_str) {
        Some("crate") => {
            module.truncate(1);
            index = 1;
        }
        Some("self") => index = 1,
        Some("super") => {
            while path.get(index).is_some_and(|segment| segment == "super") {
                if module.len() == 1 {
                    return None;
                }
                module.pop();
                index += 1;
            }
        }
        _ => return None,
    }
    module.extend(path[index..].iter().cloned());
    Some(module.join("::"))
}

fn expand_module_items(
    module_id: &str,
    variant: &ModuleVariant,
    items: Vec<Item>,
    registry: &ItemMacroRegistry,
    external_crates: &BTreeSet<String>,
) -> Vec<Item> {
    let mut expanded = Vec::new();
    for item in items {
        let Item::Macro(invocation) = &item else {
            expanded.push(item);
            continue;
        };
        if invocation.mac.path.is_ident("macro_rules")
            || !invocation.attrs.is_empty()
            || invocation.ident.is_some()
        {
            expanded.push(item);
            continue;
        }
        let mut stack = Vec::new();
        if let Some(generated) = expand_item_macro_invocation(
            module_id,
            variant,
            invocation,
            registry,
            external_crates,
            invocation.span().start(),
            &mut stack,
        ) {
            expanded.extend(generated);
        } else {
            expanded.push(item);
        }
    }
    expanded
}

fn expand_item_macro_invocation(
    module_id: &str,
    variant: &ModuleVariant,
    invocation: &syn::ItemMacro,
    registry: &ItemMacroRegistry,
    external_crates: &BTreeSet<String>,
    _outer_span: LineColumn,
    stack: &mut Vec<(String, String)>,
) -> Option<Vec<Item>> {
    let resolved = registry.resolve(module_id, variant, &invocation.mac.path)?;
    let name = invocation.mac.path.segments.last()?.ident.to_string();
    let identity = (resolved.module_id.clone(), name);
    if stack.contains(&identity) {
        return None;
    }
    stack.push(identity);
    let generated = expand_supported_item_macro(
        &resolved.definition,
        invocation.mac.tokens.clone(),
        invocation.span(),
    )?;
    let generated_names = generated
        .iter()
        .filter_map(generated_item_name)
        .collect::<BTreeSet<_>>();
    let mut fully_expanded = Vec::new();
    for item in generated {
        if matches!(item, Item::Mod(_) | Item::Use(_))
            || item_attributes(&item).iter().any(|attribute| {
                attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr")
            })
        {
            return None;
        }
        if let Item::Macro(nested) = &item {
            let path = syn_path_segments(&nested.mac.path);
            if path == ["inventory", "submit"] {
                let submitted = nested.mac.tokens.clone().into_iter().collect::<Vec<_>>();
                let [proc_macro2::TokenTree::Ident(slot)] = submitted.as_slice() else {
                    return None;
                };
                if !external_crates.contains("inventory")
                    || !generated_names.contains(&slot.to_string())
                {
                    return None;
                }
                fully_expanded.push(item);
                continue;
            }
            let nested_items = expand_item_macro_invocation(
                module_id,
                variant,
                nested,
                registry,
                external_crates,
                _outer_span,
                stack,
            )?;
            fully_expanded.extend(nested_items);
        } else {
            fully_expanded.push(item);
        }
    }
    stack.pop();
    Some(fully_expanded)
}

fn generated_item_name(item: &Item) -> Option<String> {
    match item {
        Item::Const(item) => Some(item.ident.to_string()),
        Item::Enum(item) => Some(item.ident.to_string()),
        Item::Fn(item) => Some(item.sig.ident.to_string()),
        Item::Static(item) => Some(item.ident.to_string()),
        Item::Struct(item) => Some(item.ident.to_string()),
        Item::Trait(item) => Some(item.ident.to_string()),
        Item::TraitAlias(item) => Some(item.ident.to_string()),
        Item::Type(item) => Some(item.ident.to_string()),
        Item::Union(item) => Some(item.ident.to_string()),
        _ => None,
    }
}

fn refresh_variant_indexes(variant: &mut ModuleVariant, cfg_context: &CfgContext) {
    variant.declared_items = declared_item_names(&variant.ast, cfg_context);
    variant.public_items = public_item_names(&variant.ast, cfg_context);
    variant.public_use_bindings = public_use_bindings(&variant.ast, cfg_context);
    variant.use_aliases = use_aliases(&variant.ast, cfg_context);
    variant.glob_imports = glob_imports(&variant.ast, cfg_context);
    variant.public_glob_imports = public_glob_imports(&variant.ast, cfg_context);
    variant.local_macro_bindings = local_macro_bindings(&variant.ast, cfg_context);
    variant.external_crate_aliases = declared_external_crate_aliases(&variant.ast, cfg_context);
    variant.self_crate_aliases = declared_self_crate_aliases(&variant.ast, cfg_context);
    variant.no_implicit_prelude = has_no_implicit_prelude(&variant.ast);
}

fn all_dependency_kinds() -> Vec<DependencyKind> {
    vec![
        DependencyKind::Import,
        DependencyKind::ReExport,
        DependencyKind::PathReference,
    ]
}

fn known_external_crate(name: &str) -> bool {
    matches!(name, "alloc" | "core" | "proc_macro" | "std" | "test")
}

fn known_language_name(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "f32"
            | "f64"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "str"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
    )
}

fn known_prelude_name(name: &str) -> bool {
    matches!(
        name,
        "Box"
            | "AsRef"
            | "Clone"
            | "Copy"
            | "Default"
            | "DoubleEndedIterator"
            | "Drop"
            | "Eq"
            | "Err"
            | "ExactSizeIterator"
            | "Fn"
            | "FnMut"
            | "FnOnce"
            | "From"
            | "Into"
            | "IntoIterator"
            | "Iterator"
            | "None"
            | "Ok"
            | "Option"
            | "Ord"
            | "PartialEq"
            | "PartialOrd"
            | "Result"
            | "Send"
            | "Sized"
            | "Some"
            | "String"
            | "Sync"
            | "ToOwned"
            | "ToString"
            | "TryFrom"
            | "TryInto"
            | "Unpin"
            | "Vec"
    )
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
