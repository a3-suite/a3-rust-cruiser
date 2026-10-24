use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use proc_macro2::LineColumn;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{File, Item, ItemUse};

mod macro_expansion;
mod path_resolution;

use path_resolution::{PathResolver, ResolutionScope};

use super::{
    cargo_project,
    cfg_context::{CfgContext, CfgDecision},
    rust_syntax::{
        ObservedPath, PathVisitor, ScopedUse, UseAlias, UseBinding,
        declared_external_crate_aliases, declared_item_names, declared_macro_bindings,
        declared_self_crate_aliases, flatten_use_tree, glob_imports, has_no_implicit_prelude,
        has_path_attribute, is_crate_visible, item_attributes, local_macro_bindings,
        local_macro_safety_bindings, path_to_string, public_glob_imports, public_item_names,
        public_use_bindings, syn_path_segments, use_aliases,
    },
};

use crate::boundary_validation::{
    AnalysisContext, AnalysisIssue, AnalysisProfile, CruiserError, Dependency, DependencyKind,
    DependencyObserver, LogicalModule, ModuleDeclaration, ObservedDependencyGraph, Result,
    SourceEvidence, TargetScope, matches_pattern,
};

pub struct RustDependencyObserver;

impl DependencyObserver for RustDependencyObserver {
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

struct Observer {
    project_root: PathBuf,
    cfg_context: CfgContext,
    source_lines: BTreeMap<PathBuf, Vec<String>>,
    external_crates: BTreeSet<String>,
    context_dependent_crates: BTreeSet<String>,
    self_crate_aliases: BTreeSet<String>,
    records: BTreeMap<String, ModuleRecord>,
    exported_items: BTreeMap<String, BTreeSet<String>>,
    declarations: Vec<ModuleDeclaration>,
    issues: Vec<AnalysisIssue>,
}

fn observe(
    project_root: &Path,
    mut context: AnalysisContext,
) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
    let project_root = &project_root
        .canonicalize()
        .map_err(|error| CruiserError::io("Cargo project root", error))?;
    context.features.sort();
    context.features.dedup();
    context.cfg.sort();
    context.cfg.dedup();
    let (package_root, manifest) = cargo_project::selected_manifest(project_root, &context)?;
    let cfg_context = CfgContext::resolve(project_root, &context);
    let (entry, target_issues) = cargo_project::target_entry(&package_root, &manifest, &context)?;
    let (mut external_crates, context_dependent_crates) =
        cargo_project::declared_external_crates(&manifest, &context, &cfg_context)?;
    let target_kind = context
        .compilation_target
        .split_once(':')
        .map_or(context.compilation_target.as_str(), |(kind, _)| kind);
    if target_kind != "lib" && cargo_project::has_library_target(&package_root, &manifest)? {
        // A binary/example/test target in the same package links the package
        // library under its crate name. The selected target remains the
        // observation boundary, so treat that library crate as external to
        // this graph rather than inventing a second source root.
        external_crates.insert(context.package.replace('-', "_"));
    }
    let mut observer = Observer {
        project_root: project_root.to_path_buf(),
        cfg_context,
        source_lines: BTreeMap::new(),
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
        macro_expansion::expand_supported_item_macros(
            &mut observer.records,
            &observer.cfg_context,
            &observer.external_crates,
        );
    }
    observer.self_crate_aliases = observer
        .records
        .get("crate")
        .into_iter()
        .flat_map(|record| record.variants.iter())
        .flat_map(|variant| variant.self_crate_aliases.iter().cloned())
        .collect();

    let all_module_ids: BTreeSet<String> = observer.records.keys().cloned().collect();
    let exported_items = path_resolution::build_export_index(
        &observer.records,
        &observer.external_crates,
        &observer.self_crate_aliases,
        &all_module_ids,
    );
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
        .map(|id| {
            let mut source_paths = observer
                .records
                .get(id)
                .into_iter()
                .flat_map(|record| record.variants.iter())
                .map(|variant| observer.relative_path(&variant.path))
                .collect::<Vec<_>>();
            source_paths.sort();
            source_paths.dedup();
            LogicalModule {
                id: id.clone(),
                source_paths,
            }
        })
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
    fn discover_module(&mut self, id: &str, path: &Path) -> Result<()> {
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
        self.collect_top_level_use_dependencies(
            source_id,
            variant,
            selected,
            all_modules,
            dependencies,
            issues,
        );
        let visitor = self.observe_variant(variant, analysis_profile);
        let PathVisitor {
            paths,
            scoped_uses,
            strict_path_spans,
            macro_spans,
            cfg_spans,
            ..
        } = visitor;
        self.collect_scoped_use_dependencies(
            source_id,
            variant,
            &scoped_uses,
            selected,
            all_modules,
            dependencies,
            issues,
        );
        if analysis_profile == AnalysisProfile::Compilation {
            self.append_visitor_issues(source_id, &variant.path, macro_spans, cfg_spans, issues);
        }
        self.collect_observed_path_dependencies(
            source_id,
            variant,
            paths,
            strict_path_spans,
            selected,
            all_modules,
            analysis_profile,
            dependencies,
            issues,
        );
    }

    fn collect_top_level_use_dependencies(
        &self,
        source_id: &str,
        variant: &ModuleVariant,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
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
    }

    fn observe_variant<'a>(
        &'a self,
        variant: &'a ModuleVariant,
        analysis_profile: AnalysisProfile,
    ) -> PathVisitor<'a> {
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
            declared_items: &variant.declared_items,
            observe_attribute_paths: analysis_profile == AnalysisProfile::Compilation,
            paths: Vec::new(),
            scoped_uses: Vec::new(),
            local_bindings: BTreeSet::new(),
            local_item_bindings: BTreeSet::new(),
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
        };
        visitor.visit_file(&variant.ast);
        visitor
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_scoped_use_dependencies(
        &self,
        source_id: &str,
        variant: &ModuleVariant,
        scoped_uses: &[ScopedUse],
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
        dependencies: &mut Vec<Dependency>,
        issues: &mut Vec<AnalysisIssue>,
    ) {
        for scoped_use in scoped_uses {
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
    }

    fn append_visitor_issues(
        &self,
        source_id: &str,
        path: &Path,
        macro_spans: Vec<LineColumn>,
        cfg_spans: Vec<LineColumn>,
        issues: &mut Vec<AnalysisIssue>,
    ) {
        for macro_span in macro_spans {
            issues.push(AnalysisIssue::unresolved_dependency_candidate(
                "analysis.unsupported-macro-expansion",
                "macro expansion is outside the MVP observer boundary",
                format!("{source_id}:{}", macro_span.line),
                source_id.to_owned(),
                self.evidence(path, macro_span),
                Vec::new(),
                all_dependency_kinds(),
            ));
        }
        for cfg_span in cfg_spans {
            issues.push(AnalysisIssue::analysis_context_unresolved(
                "analysis.unsupported-analysis-context",
                "cfg and feature conditions are not evaluated by the MVP observer",
                format!("{source_id}:{}", cfg_span.line),
                Some(source_id.to_owned()),
                Some(self.evidence(path, cfg_span)),
                all_dependency_kinds(),
            ));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_observed_path_dependencies(
        &self,
        source_id: &str,
        variant: &ModuleVariant,
        paths: Vec<ObservedPath>,
        strict_path_spans: Vec<LineColumn>,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
        analysis_profile: AnalysisProfile,
        dependencies: &mut Vec<Dependency>,
        issues: &mut Vec<AnalysisIssue>,
    ) {
        for observed_path in paths {
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
            let resolver = PathResolver::new(
                &self.records,
                &self.external_crates,
                &self.self_crate_aliases,
                &self.exported_items,
            );
            let resolved = resolver.resolve_path(
                source_id,
                variant,
                &observed_scope,
                &path_segments,
                path.leading_colon.is_some(),
                all_modules,
            );
            let may_reference_internal = resolver.may_reference_internal_module(
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
                    let candidate_targets = resolver.candidate_targets_for_path(
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
            let resolver = PathResolver::new(
                &self.records,
                &self.external_crates,
                &self.self_crate_aliases,
                &self.exported_items,
            );
            let mut resolved = resolver.resolve_path(
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
                let candidate_targets = resolver.candidate_targets_for_path(
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

    fn evidence(&self, path: &Path, start: LineColumn) -> SourceEvidence {
        let relative = self.relative_path(path);
        let column = self
            .source_lines
            .get(path)
            .and_then(|lines| lines.get(start.line.saturating_sub(1)))
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

    fn relative_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/")
    }
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
