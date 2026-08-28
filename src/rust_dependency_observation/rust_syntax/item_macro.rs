use std::collections::BTreeMap;

use proc_macro2::{Group, Span, TokenStream, TokenTree};
use syn::{File, Item};

use super::macro_syntax::{
    is_punct, metavariable_names, normalize_metavariables, split_single_macro_rule,
};

#[derive(Clone)]
pub(crate) struct SupportedItemMacroDefinition {
    parameters: Vec<MacroParameter>,
    transcriber: TokenStream,
}

#[derive(Clone)]
struct MacroParameter {
    key: &'static str,
    binding: String,
    fragment: &'static str,
}

const BUILTIN_RUNTIME_PARAMETERS: &[(&str, &str)] = &[
    ("slot", "ident"),
    ("agnostic_fn", "ident"),
    ("contextual_fn", "ident"),
    ("agnostic_for_fn", "ident"),
    ("contextual_for_fn", "ident"),
    ("source_view_resolver", "path"),
    ("language", "literal"),
    ("provider", "literal"),
    ("agnostic_provider", "path"),
    ("contextual_provider", "path"),
];

const POLICY_RUNTIME_PARAMETERS: &[(&str, &str)] = &[
    ("slot", "ident"),
    ("agnostic_fn", "ident"),
    ("contextual_fn", "ident"),
    ("agnostic_for_fn", "ident"),
    ("contextual_for_fn", "ident"),
    ("source_view_resolver", "path"),
    ("language", "literal"),
    ("provider", "literal"),
];

pub(crate) fn supported_item_macro_definition(
    item: &syn::ItemMacro,
) -> Option<SupportedItemMacroDefinition> {
    if !item.mac.path.is_ident("macro_rules") || !item.attrs.is_empty() {
        return None;
    }
    let tokens = item.mac.tokens.clone().into_iter().collect::<Vec<_>>();
    let (matcher, transcriber) = split_single_macro_rule(&tokens)?;
    let [TokenTree::Group(matcher)] = matcher.as_slice() else {
        return None;
    };
    let [TokenTree::Group(transcriber)] = transcriber.as_slice() else {
        return None;
    };
    for (expected, expected_transcriber) in [
        (BUILTIN_RUNTIME_PARAMETERS, builtin_runtime_transcriber()),
        (POLICY_RUNTIME_PARAMETERS, policy_runtime_transcriber()),
    ] {
        let Some(parameters) =
            parse_named_macro_matcher(&matcher.stream().into_iter().collect::<Vec<_>>(), expected)
        else {
            continue;
        };
        let canonical_names = parameters
            .iter()
            .zip(expected)
            .map(|(parameter, (key, _))| (parameter.binding.clone(), *key))
            .collect::<BTreeMap<_, _>>();
        let normalized = normalize_metavariables(transcriber.stream(), &canonical_names)?;
        if normalized.to_string() == expected_transcriber.to_string() {
            return Some(SupportedItemMacroDefinition {
                parameters,
                transcriber: transcriber.stream(),
            });
        }
    }
    None
}

pub(crate) fn expand_supported_item_macro(
    definition: &SupportedItemMacroDefinition,
    arguments: TokenStream,
    invocation_span: Span,
) -> Option<Vec<Item>> {
    let values = parse_named_macro_arguments(arguments, &definition.parameters)?;
    let expanded = substitute_metavariables(definition.transcriber.clone(), &values)?;
    let expanded = with_token_stream_span(expanded, invocation_span);
    syn::parse2::<File>(expanded).ok().map(|file| file.items)
}

fn parse_named_macro_matcher(
    tokens: &[TokenTree],
    expected: &[(&'static str, &'static str)],
) -> Option<Vec<MacroParameter>> {
    let bindings = metavariable_names(tokens);
    if bindings.len() != expected.len() {
        return None;
    }
    let canonical_names = bindings
        .iter()
        .zip(expected)
        .map(|(binding, (key, _))| (binding.clone(), *key))
        .collect::<BTreeMap<_, _>>();
    if canonical_names.len() != bindings.len() {
        return None;
    }
    let mut normalized = normalize_metavariables(
        tokens.iter().cloned().collect::<TokenStream>(),
        &canonical_names,
    )?
    .into_iter()
    .collect::<Vec<_>>();
    if normalized.last().is_some_and(|token| is_punct(token, ',')) {
        normalized.pop();
    }
    let expected_tokens = expected
        .iter()
        .map(|(key, fragment)| format!("{key} = ${key}:{fragment}"))
        .collect::<Vec<_>>()
        .join(", ")
        .parse::<TokenStream>()
        .expect("canonical item macro matcher must tokenize")
        .to_string();
    if normalized.into_iter().collect::<TokenStream>().to_string() != expected_tokens {
        return None;
    }
    Some(
        bindings
            .into_iter()
            .zip(expected)
            .map(|(binding, (key, fragment))| MacroParameter {
                key,
                binding,
                fragment,
            })
            .collect(),
    )
}

fn parse_named_macro_arguments(
    tokens: TokenStream,
    parameters: &[MacroParameter],
) -> Option<BTreeMap<String, TokenStream>> {
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let mut values = BTreeMap::new();
    let mut start = 0;
    for parameter in parameters {
        let end = tokens[start..]
            .iter()
            .position(|token| is_punct(token, ','))
            .map_or(tokens.len(), |offset| start + offset);
        let assignment = &tokens[start..end];
        let [TokenTree::Ident(key), TokenTree::Punct(equals), value @ ..] = assignment else {
            return None;
        };
        if key != parameter.key || equals.as_char() != '=' || value.is_empty() {
            return None;
        }
        let value = value.iter().cloned().collect::<TokenStream>();
        let valid = match parameter.fragment {
            "ident" => matches!(assignment, [_, _, TokenTree::Ident(_)]),
            "literal" => matches!(assignment, [_, _, TokenTree::Literal(_)]),
            "path" => syn::parse2::<syn::Path>(value.clone()).is_ok(),
            _ => false,
        };
        if !valid || values.insert(parameter.binding.clone(), value).is_some() {
            return None;
        }
        start = end.saturating_add(1);
    }
    if start < tokens.len() {
        return None;
    }
    Some(values)
}

fn substitute_metavariables(
    tokens: TokenStream,
    values: &BTreeMap<String, TokenStream>,
) -> Option<TokenStream> {
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let mut output = TokenStream::new();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Punct(punct) if punct.as_char() == '$' => {
                let TokenTree::Ident(binding) = tokens.get(index + 1)? else {
                    return None;
                };
                output.extend(values.get(&binding.to_string())?.clone());
                index += 2;
            }
            TokenTree::Group(group) => {
                let stream = substitute_metavariables(group.stream(), values)?;
                let mut substituted = Group::new(group.delimiter(), stream);
                substituted.set_span(group.span());
                output.extend([TokenTree::Group(substituted)]);
                index += 1;
            }
            token => {
                output.extend([token.clone()]);
                index += 1;
            }
        }
    }
    Some(output)
}

fn with_token_stream_span(tokens: TokenStream, span: Span) -> TokenStream {
    tokens
        .into_iter()
        .map(|mut token| {
            if let TokenTree::Group(group) = token {
                let mut group = Group::new(
                    group.delimiter(),
                    with_token_stream_span(group.stream(), span),
                );
                group.set_span(span);
                token = TokenTree::Group(group);
            } else {
                token.set_span(span);
            }
            token
        })
        .collect()
}

fn builtin_runtime_transcriber() -> TokenStream {
    r#"{
        pub(crate) const LANGUAGE: &str = $language;
        pub(crate) const PROVIDER_NAME: &str = $provider;
        pub(crate) const $slot: crate::foundation::plugin::provider::LanguagePluginProviderSlot =
            crate::foundation::plugin::provider::LanguagePluginProviderSlot {
                name: PROVIDER_NAME,
                language: LANGUAGE,
                language_agnostic_snapshot_extractor_for: $agnostic_for_fn,
                contextual_snapshot_materializer_for: $contextual_for_fn,
                source_view_resolver: $source_view_resolver,
            };
        inventory::submit! { $slot }
        pub(crate) fn $agnostic_fn() -> Option<crate::foundation::plugin::language_agnostic_snapshot_extractor::LanguageAgnosticSnapshotExtractor> { $agnostic_provider() }
        pub(crate) fn $contextual_fn() -> Option<crate::foundation::plugin::contextual_snapshot_materializer::ContextualSnapshotMaterializer> { $contextual_provider() }
        fn $agnostic_for_fn(language: &str,) -> Option<crate::foundation::plugin::language_agnostic_snapshot_extractor::LanguageAgnosticSnapshotExtractor> { if language == LANGUAGE { $agnostic_fn() } else { None } }
        fn $contextual_for_fn(language: &str,) -> Option<crate::foundation::plugin::contextual_snapshot_materializer::ContextualSnapshotMaterializer> { if language == LANGUAGE { $contextual_fn() } else { None } }
    }"#
        .parse::<TokenStream>()
        .expect("built-in runtime macro template must parse")
        .into_iter()
        .next()
        .and_then(|token| match token {
            TokenTree::Group(group) => Some(group.stream()),
            _ => None,
        })
        .expect("built-in runtime macro template must be a group")
}

fn policy_runtime_transcriber() -> TokenStream {
    r#"{
        crate::foundation::plugin::policy_only::declare_builtin_language_runtime! {
            slot = $slot,
            agnostic_fn = $agnostic_fn,
            contextual_fn = $contextual_fn,
            agnostic_for_fn = $agnostic_for_fn,
            contextual_for_fn = $contextual_for_fn,
            source_view_resolver = $source_view_resolver,
            language = $language,
            provider = $provider,
            agnostic_provider = crate::foundation::plugin::policy_only::no_language_agnostic_snapshot_extractor,
            contextual_provider = crate::foundation::plugin::policy_only::no_contextual_snapshot_materializer
        }
    }"#
        .parse::<TokenStream>()
        .expect("policy runtime macro template must parse")
        .into_iter()
        .next()
        .and_then(|token| match token {
            TokenTree::Group(group) => Some(group.stream()),
            _ => None,
        })
        .expect("policy runtime macro template must be a group")
}
