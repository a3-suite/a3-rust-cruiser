use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{TokenStream, TokenTree};
use syn::{File, Item};

use super::super::cfg_context::CfgContext;
use super::macro_syntax::{
    is_dependency_neutral_expression_macro, is_dependency_neutral_token_macro,
    is_dependency_observing_macro, is_punct, macro_path_is, metavariable_names,
    normalize_metavariables, split_single_macro_rule,
};
use super::module_index::declared_macro_bindings;

pub(crate) fn local_macro_safety_bindings(
    file: &File,
    cfg_context: &CfgContext,
    declared_items: &BTreeSet<String>,
    no_implicit_prelude: bool,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let macro_bindings = declared_macro_bindings(file, cfg_context);
    let scope = MacroDefinitionScope {
        declared_items,
        macro_bindings: &macro_bindings,
        no_implicit_prelude,
    };
    let mut safe = BTreeSet::new();
    let mut observing = BTreeSet::new();
    for item in &file.items {
        let Item::Macro(item_macro) = item else {
            continue;
        };
        let Some(name) = item_macro.ident.as_ref().map(ToString::to_string) else {
            continue;
        };
        let Some(observes_arguments) = local_macro_safety(item_macro, &scope) else {
            continue;
        };
        safe.insert(name.clone());
        if observes_arguments {
            observing.insert(name);
        }
    }
    (safe, observing)
}

pub(crate) struct MacroDefinitionScope<'a> {
    pub(crate) declared_items: &'a BTreeSet<String>,
    pub(crate) macro_bindings: &'a BTreeSet<String>,
    pub(crate) no_implicit_prelude: bool,
}

pub(super) fn local_macro_safety(
    item: &syn::ItemMacro,
    scope: &MacroDefinitionScope<'_>,
) -> Option<bool> {
    if !item.mac.path.is_ident("macro_rules") || !item.attrs.is_empty() {
        return None;
    }
    let tokens = item.mac.tokens.clone().into_iter().collect::<Vec<_>>();
    let (matcher, transcriber) = split_single_macro_rule(&tokens)?;
    let [TokenTree::Group(matcher_group)] = matcher.as_slice() else {
        return None;
    };
    let [TokenTree::Group(transcriber_group)] = transcriber.as_slice() else {
        return None;
    };
    if let Some(observes_arguments) = exact_validation_macro_safety(
        &matcher_group.stream().into_iter().collect::<Vec<_>>(),
        &transcriber_group.stream().into_iter().collect::<Vec<_>>(),
        scope,
    ) {
        return Some(observes_arguments);
    }
    let mut bindings = BTreeSet::new();
    if !validate_macro_matcher(
        &matcher_group.stream().into_iter().collect::<Vec<_>>(),
        &mut bindings,
    ) {
        return None;
    }
    let mut observes_arguments = false;
    if !validate_macro_transcriber(
        &transcriber_group.stream().into_iter().collect::<Vec<_>>(),
        &bindings,
        scope.macro_bindings,
        &mut observes_arguments,
        true,
    ) {
        return None;
    }
    Some(observes_arguments)
}

fn exact_validation_macro_safety(
    matcher: &[TokenTree],
    transcriber: &[TokenTree],
    scope: &MacroDefinitionScope<'_>,
) -> Option<bool> {
    if !scope_allows_validation_symbols(scope) {
        return None;
    }
    let matcher = exact_macro_matcher(matcher, &["expr", "expr", "literal"])
        .or_else(|| exact_macro_matcher(matcher, &["expr", "expr", "expr", "literal"]))?;
    if matcher.len() == 3 && exact_contract_value_transcriber(transcriber, &matcher) {
        return Some(true);
    }
    if matcher.len() == 4 && exact_shape_transcriber(transcriber, &matcher) {
        return Some(true);
    }
    None
}

fn exact_macro_matcher(tokens: &[TokenTree], fragments: &[&str]) -> Option<Vec<String>> {
    const CANONICAL_NAMES: [&str; 4] = ["argument0", "argument1", "argument2", "argument3"];

    let names = metavariable_names(tokens);
    if names.len() != fragments.len() || names.len() > CANONICAL_NAMES.len() {
        return None;
    }
    let canonical_names = names
        .iter()
        .enumerate()
        .map(|(index, binding)| (binding.clone(), CANONICAL_NAMES[index]))
        .collect::<BTreeMap<_, _>>();
    if canonical_names.len() != names.len() {
        return None;
    }
    let expected = fragments
        .iter()
        .enumerate()
        .map(|(index, fragment)| format!("${}:{}", CANONICAL_NAMES[index], fragment))
        .collect::<Vec<_>>()
        .join(", ")
        .parse::<TokenStream>()
        .expect("canonical validation macro matcher must tokenize");
    let normalized = normalize_metavariables(tokens.iter().cloned().collect(), &canonical_names)?;
    (normalized.to_string() == expected.to_string()).then_some(names)
}

fn exact_contract_value_transcriber(tokens: &[TokenTree], matcher: &[String]) -> bool {
    normalized_transcriber_matches(
        tokens,
        matcher,
        &["actual", "expected", "label"],
        "if $actual != $expected { return Err(format!(\"{} must be {}\", $label, $expected)); }",
    )
}

fn exact_shape_transcriber(tokens: &[TokenTree], matcher: &[String]) -> bool {
    normalized_transcriber_matches(
        tokens,
        matcher,
        &["path", "actual", "expected", "field"],
        "if $actual != $expected { return Err(format!(\"{} {} does not match the authoritative schema\", $path.display(), $field)); }",
    )
}

fn normalized_transcriber_matches(
    tokens: &[TokenTree],
    matcher: &[String],
    canonical_names: &[&'static str],
    expected: &str,
) -> bool {
    let names = matcher
        .iter()
        .zip(canonical_names)
        .map(|(binding, canonical)| (binding.clone(), *canonical))
        .collect::<BTreeMap<_, _>>();
    let expected = expected
        .parse::<TokenStream>()
        .expect("canonical validation macro transcriber must tokenize");
    normalize_metavariables(tokens.iter().cloned().collect(), &names)
        .is_some_and(|normalized| normalized.to_string() == expected.to_string())
}

fn scope_allows_validation_symbols(scope: &MacroDefinitionScope<'_>) -> bool {
    !scope.no_implicit_prelude
        && !scope.declared_items.contains("Err")
        && !scope.macro_bindings.contains("Err")
        && !scope.declared_items.contains("format")
        && !scope.macro_bindings.contains("format")
}

fn validate_macro_matcher(tokens: &[TokenTree], bindings: &mut BTreeSet<String>) -> bool {
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Group(group) => {
                if !validate_macro_matcher(
                    &group.stream().into_iter().collect::<Vec<_>>(),
                    bindings,
                ) {
                    return false;
                }
            }
            TokenTree::Punct(punct) if punct.as_char() == '$' => {
                let Some(TokenTree::Ident(name)) = tokens.get(index + 1) else {
                    return false;
                };
                let Some(TokenTree::Punct(colon)) = tokens.get(index + 2) else {
                    return false;
                };
                let Some(TokenTree::Ident(fragment)) = tokens.get(index + 3) else {
                    return false;
                };
                if colon.as_char() != ':'
                    || !matches!(fragment.to_string().as_str(), "expr" | "literal")
                    || !bindings.insert(name.to_string())
                {
                    return false;
                }
                index += 3;
            }
            TokenTree::Punct(punct) if matches!(punct.as_char(), '*' | '+' | '?') => {
                return false;
            }
            _ => {}
        }
        index += 1;
    }
    true
}

fn validate_macro_transcriber(
    tokens: &[TokenTree],
    bindings: &BTreeSet<String>,
    macro_bindings: &BTreeSet<String>,
    observes_arguments: &mut bool,
    observe_metavariables: bool,
) -> bool {
    let mut index = 0;
    while index < tokens.len() {
        if let Some((next_index, path, nested_tokens)) =
            known_nested_macro(tokens, index, macro_bindings)
        {
            let valid = if macro_path_is(&path, &["cfg"]) {
                validate_cfg_macro_tokens(&nested_tokens)
            } else {
                validate_macro_transcriber(
                    &nested_tokens,
                    bindings,
                    macro_bindings,
                    observes_arguments,
                    observe_metavariables && !is_dependency_neutral_token_macro(&path),
                )
            };
            if !valid {
                return false;
            }
            index = next_index;
            continue;
        }
        match &tokens[index] {
            TokenTree::Group(group) => {
                if !validate_macro_transcriber(
                    &group.stream().into_iter().collect::<Vec<_>>(),
                    bindings,
                    macro_bindings,
                    observes_arguments,
                    observe_metavariables,
                ) {
                    return false;
                }
            }
            TokenTree::Punct(punct) if punct.as_char() == '$' => {
                let Some(TokenTree::Ident(name)) = tokens.get(index + 1) else {
                    return false;
                };
                if !bindings.contains(&name.to_string()) {
                    return false;
                }
                if observe_metavariables {
                    *observes_arguments = true;
                }
                index += 1;
            }
            TokenTree::Ident(identifier) if !is_safe_macro_keyword(identifier) => {
                return false;
            }
            TokenTree::Punct(punct) if matches!(punct.as_char(), '#' | ':' | '\'') => {
                return false;
            }
            _ => {}
        }
        index += 1;
    }
    true
}

fn known_nested_macro(
    tokens: &[TokenTree],
    start: usize,
    macro_bindings: &BTreeSet<String>,
) -> Option<(usize, Vec<String>, Vec<TokenTree>)> {
    let TokenTree::Ident(first) = tokens.get(start)? else {
        return None;
    };
    if macro_bindings.contains(&first.to_string()) {
        return None;
    }
    let mut path = vec![first.to_string()];
    let mut index = start + 1;
    while index + 2 < tokens.len()
        && is_punct(&tokens[index], ':')
        && is_punct(&tokens[index + 1], ':')
    {
        let TokenTree::Ident(segment) = &tokens[index + 2] else {
            return None;
        };
        path.push(segment.to_string());
        index += 3;
    }
    if !is_punct(tokens.get(index)?, '!') {
        return None;
    }
    let TokenTree::Group(arguments) = tokens.get(index + 1)? else {
        return None;
    };
    if !(is_dependency_neutral_token_macro(&path)
        || is_dependency_neutral_expression_macro(&path)
        || is_dependency_observing_macro(&path))
    {
        return None;
    }
    Some((index + 2, path, arguments.stream().into_iter().collect()))
}

fn validate_cfg_macro_tokens(tokens: &[TokenTree]) -> bool {
    tokens.iter().all(|token| match token {
        TokenTree::Group(group) => {
            validate_cfg_macro_tokens(&group.stream().into_iter().collect::<Vec<_>>())
        }
        TokenTree::Punct(punct) => !matches!(punct.as_char(), '#' | ':' | '\'' | '$'),
        TokenTree::Ident(_) | TokenTree::Literal(_) => true,
    })
}

fn is_safe_macro_keyword(identifier: &syn::Ident) -> bool {
    matches!(
        identifier.to_string().as_str(),
        "_" | "as"
            | "async"
            | "await"
            | "break"
            | "continue"
            | "else"
            | "false"
            | "for"
            | "if"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "move"
            | "mut"
            | "return"
            | "true"
            | "unsafe"
            | "while"
    )
}
