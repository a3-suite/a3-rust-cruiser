use std::{collections::BTreeSet, process::Command};

use syn::{Attribute, Meta, Token, parse::Parser, punctuated::Punctuated};

use crate::boundary_validation::{AnalysisContext, AnalysisProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CfgDecision {
    Active,
    Inactive,
    Unknown,
}

#[derive(Debug, Clone)]
pub(super) struct CfgContext {
    active: BTreeSet<String>,
    available: bool,
    source_union: bool,
}

impl CfgContext {
    pub(super) fn resolve(context: &AnalysisContext) -> Self {
        let output = Command::new("rustc")
            .args(["--print", "cfg", "--target", &context.target_triple])
            .output();
        let (mut active, available) = match output {
            Ok(output) if output.status.success() => (
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(normalize_cfg)
                    .collect(),
                true,
            ),
            _ => (BTreeSet::new(), false),
        };
        active.extend(context.cfg.iter().map(|value| normalize_cfg(value)));
        active.extend(
            context
                .features
                .iter()
                .map(|feature| format!("feature=\"{feature}\"")),
        );
        Self {
            active,
            available,
            source_union: context.analysis_profile == AnalysisProfile::SourceUnion,
        }
    }

    pub(super) fn evaluate_attributes(&self, attributes: &[Attribute]) -> CfgDecision {
        if self.source_union {
            return CfgDecision::Active;
        }
        let mut decision = CfgDecision::Active;
        for attribute in attributes {
            let current = if attribute.path().is_ident("cfg") {
                attribute
                    .parse_args::<Meta>()
                    .map_or(CfgDecision::Unknown, |meta| self.evaluate_meta(&meta))
            } else if attribute.path().is_ident("cfg_attr") {
                self.evaluate_cfg_attr(attribute)
            } else {
                continue;
            };
            decision = and(decision, current);
            if decision == CfgDecision::Inactive {
                break;
            }
        }
        decision
    }

    pub(super) fn has_active_path_attribute(&self, attribute: &Attribute) -> bool {
        if attribute.path().is_ident("path") {
            return true;
        }
        if !attribute.path().is_ident("cfg_attr") {
            return false;
        }
        let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
        let Ok(arguments) = attribute.parse_args_with(parser) else {
            return false;
        };
        let mut arguments = arguments.iter();
        let Some(predicate) = arguments.next() else {
            return false;
        };
        if self.source_union {
            return arguments.any(contains_path_meta);
        }
        match self.evaluate_meta(predicate) {
            CfgDecision::Inactive => false,
            CfgDecision::Active | CfgDecision::Unknown => arguments.any(contains_path_meta),
        }
    }

    pub(super) fn evaluate_selector(&self, selector: &str) -> CfgDecision {
        let Some(predicate) = selector
            .strip_prefix("cfg(")
            .and_then(|value| value.strip_suffix(')'))
        else {
            return CfgDecision::Unknown;
        };
        syn::parse_str::<Meta>(predicate)
            .map_or(CfgDecision::Unknown, |meta| self.evaluate_meta(&meta))
    }

    fn evaluate_cfg_attr(&self, attribute: &Attribute) -> CfgDecision {
        let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
        let Ok(arguments) = attribute.parse_args_with(parser) else {
            return CfgDecision::Unknown;
        };
        let mut arguments = arguments.iter();
        let Some(predicate) = arguments.next() else {
            return CfgDecision::Unknown;
        };
        match self.evaluate_meta(predicate) {
            CfgDecision::Inactive => CfgDecision::Active,
            CfgDecision::Unknown => CfgDecision::Unknown,
            CfgDecision::Active => arguments
                .filter(|meta| meta.path().is_ident("cfg"))
                .map(|meta| match meta {
                    Meta::List(list) => list
                        .parse_args::<Meta>()
                        .map_or(CfgDecision::Unknown, |meta| self.evaluate_meta(&meta)),
                    _ => CfgDecision::Unknown,
                })
                .fold(CfgDecision::Active, and),
        }
    }

    fn evaluate_meta(&self, meta: &Meta) -> CfgDecision {
        if !self.available {
            return CfgDecision::Unknown;
        }
        match meta {
            Meta::Path(path) => path
                .get_ident()
                .map(|ident| self.contains(&ident.to_string()))
                .unwrap_or(CfgDecision::Unknown),
            Meta::NameValue(value) => {
                let Some(ident) = value.path.get_ident() else {
                    return CfgDecision::Unknown;
                };
                let syn::Expr::Lit(expression) = &value.value else {
                    return CfgDecision::Unknown;
                };
                let syn::Lit::Str(literal) = &expression.lit else {
                    return CfgDecision::Unknown;
                };
                self.contains(&format!("{}=\"{}\"", ident, literal.value()))
            }
            Meta::List(list) if list.path.is_ident("all") || list.path.is_ident("any") => {
                let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
                let Ok(values) = parser.parse2(list.tokens.clone()) else {
                    return CfgDecision::Unknown;
                };
                let decisions = values.iter().map(|value| self.evaluate_meta(value));
                if list.path.is_ident("all") {
                    decisions.fold(CfgDecision::Active, and)
                } else {
                    decisions.fold(CfgDecision::Inactive, or)
                }
            }
            Meta::List(list) if list.path.is_ident("not") => {
                list.parse_args::<Meta>()
                    .map_or(CfgDecision::Unknown, |value| {
                        match self.evaluate_meta(&value) {
                            CfgDecision::Active => CfgDecision::Inactive,
                            CfgDecision::Inactive => CfgDecision::Active,
                            CfgDecision::Unknown => CfgDecision::Unknown,
                        }
                    })
            }
            Meta::List(_) => CfgDecision::Unknown,
        }
    }

    fn contains(&self, value: &str) -> CfgDecision {
        if self.active.contains(&normalize_cfg(value)) {
            CfgDecision::Active
        } else {
            CfgDecision::Inactive
        }
    }
}

fn normalize_cfg(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn contains_path_meta(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("path"),
        Meta::NameValue(value) => value.path.is_ident("path"),
        Meta::List(list) => {
            list.path.is_ident("path")
                || (list.path.is_ident("cfg_attr")
                    && Punctuated::<Meta, Token![,]>::parse_terminated
                        .parse2(list.tokens.clone())
                        .is_ok_and(|nested| nested.iter().any(contains_path_meta)))
        }
    }
}

fn and(left: CfgDecision, right: CfgDecision) -> CfgDecision {
    match (left, right) {
        (CfgDecision::Inactive, _) | (_, CfgDecision::Inactive) => CfgDecision::Inactive,
        (CfgDecision::Unknown, _) | (_, CfgDecision::Unknown) => CfgDecision::Unknown,
        _ => CfgDecision::Active,
    }
}

fn or(left: CfgDecision, right: CfgDecision) -> CfgDecision {
    match (left, right) {
        (CfgDecision::Active, _) | (_, CfgDecision::Active) => CfgDecision::Active,
        (CfgDecision::Unknown, _) | (_, CfgDecision::Unknown) => CfgDecision::Unknown,
        _ => CfgDecision::Inactive,
    }
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::{CfgContext, CfgDecision};

    fn available_context() -> CfgContext {
        CfgContext {
            active: ["unix", "target_os=\"linux\"", "feature=\"cli\""]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            available: true,
            source_union: false,
        }
    }

    #[test]
    fn should_evaluate_compound_cfg_selectors_with_three_states() {
        let context = available_context();

        assert_eq!(
            context.evaluate_selector("cfg(all(unix, target_os = \"linux\"))"),
            CfgDecision::Active
        );
        assert_eq!(
            context.evaluate_selector("cfg(any(windows, feature = \"cli\"))"),
            CfgDecision::Active
        );
        assert_eq!(
            context.evaluate_selector("cfg(not(unix))"),
            CfgDecision::Inactive
        );
        assert_eq!(
            context.evaluate_selector("cfg(all(unix, custom(value)))"),
            CfgDecision::Unknown
        );
        assert_eq!(
            context.evaluate_selector("target_os = \"linux\""),
            CfgDecision::Unknown
        );
    }

    #[test]
    fn should_propagate_unavailable_cfg_context_as_unknown() {
        let context = CfgContext {
            active: Default::default(),
            available: false,
            source_union: false,
        };

        assert_eq!(context.evaluate_selector("cfg(unix)"), CfgDecision::Unknown);
    }

    #[test]
    fn should_apply_cfg_attr_only_when_its_predicate_is_active() {
        let context = available_context();
        let inactive_condition = parse_quote!(#[cfg_attr(windows, cfg(unix))]);
        let active_condition = parse_quote!(#[cfg_attr(unix, cfg(windows))]);

        assert_eq!(
            context.evaluate_attributes(&[inactive_condition]),
            CfgDecision::Active
        );
        assert_eq!(
            context.evaluate_attributes(&[active_condition]),
            CfgDecision::Inactive
        );
    }

    #[test]
    fn should_detect_only_active_cfg_attr_path_attributes() {
        let context = available_context();
        let active_path = parse_quote!(#[cfg_attr(unix, path = "alternate.rs")]);
        let inactive_path = parse_quote!(#[cfg_attr(windows, path = "alternate.rs")]);

        assert!(context.has_active_path_attribute(&active_path));
        assert!(!context.has_active_path_attribute(&inactive_path));
    }

    #[test]
    fn should_treat_all_attributes_as_active_in_source_union() {
        let context = CfgContext {
            source_union: true,
            ..available_context()
        };
        let attribute = parse_quote!(#[cfg(windows)]);

        assert_eq!(
            context.evaluate_attributes(&[attribute]),
            CfgDecision::Active
        );
    }
}
