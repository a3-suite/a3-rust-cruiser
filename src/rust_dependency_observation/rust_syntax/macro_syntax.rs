use std::collections::BTreeMap;

use proc_macro2::{Group, Span, TokenStream, TokenTree};

pub(super) fn is_dependency_neutral_token_macro(path: &[String]) -> bool {
    macro_identity(path).is_some_and(|name| {
        matches!(
            name,
            "cfg"
                | "column"
                | "concat"
                | "env"
                | "file"
                | "format_description"
                | "include_bytes"
                | "include_str"
                | "line"
                | "macro_rules"
                | "module_path"
                | "option_env"
                | "stringify"
        )
    })
}

pub(super) fn is_dependency_observing_macro(path: &[String]) -> bool {
    [
        &["matches"][..],
        &["json"][..],
        &["serde_json", "json"][..],
        &["inventory", "submit"][..],
        &["inventory", "collect"][..],
        &["require_contract_value"][..],
        &["require_shape"][..],
    ]
    .iter()
    .any(|expected| macro_path_is(path, expected))
}

pub(super) fn is_matches_macro(path: &[String]) -> bool {
    macro_path_is(path, &["matches"])
}

pub(super) fn is_json_macro(path: &[String]) -> bool {
    macro_path_is(path, &["json"]) || macro_path_is(path, &["serde_json", "json"])
}

pub(super) fn macro_path_is(path: &[String], expected: &[&str]) -> bool {
    path.len() == expected.len()
        && path
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual == expected)
}

pub(super) fn is_dependency_neutral_expression_macro(path: &[String]) -> bool {
    macro_identity(path).is_some_and(|name| {
        matches!(
            name,
            "assert"
                | "assert_eq"
                | "assert_ne"
                | "dbg"
                | "debug_assert"
                | "debug_assert_eq"
                | "debug_assert_ne"
                | "eprint"
                | "eprintln"
                | "format"
                | "format_args"
                | "panic"
                | "print"
                | "println"
                | "todo"
                | "unimplemented"
                | "unreachable"
                | "vec"
                | "write"
                | "writeln"
        )
    })
}

fn macro_identity(path: &[String]) -> Option<&str> {
    match path {
        [name] => Some(name),
        [root, name] if matches!(root.as_str(), "alloc" | "core" | "std") => Some(name),
        _ => None,
    }
}

pub(super) fn metavariable_names(tokens: &[TokenTree]) -> Vec<String> {
    tokens
        .windows(2)
        .filter_map(|pair| match pair {
            [TokenTree::Punct(dollar), TokenTree::Ident(binding)] if dollar.as_char() == '$' => {
                Some(binding.to_string())
            }
            _ => None,
        })
        .collect()
}

pub(super) fn split_single_macro_rule(
    tokens: &[TokenTree],
) -> Option<(Vec<TokenTree>, Vec<TokenTree>)> {
    let mut arrow_index = None;
    let mut index = 0;
    while index + 1 < tokens.len() {
        if is_punct(&tokens[index], '=') && is_punct(&tokens[index + 1], '>') {
            if arrow_index.replace(index).is_some() {
                return None;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    let arrow_index = arrow_index?;
    let matcher = tokens[..arrow_index].to_vec();
    let mut transcriber = tokens[arrow_index + 2..].to_vec();
    if matches!(transcriber.last(), Some(TokenTree::Punct(punct)) if punct.as_char() == ';') {
        transcriber.pop();
    }
    if transcriber
        .iter()
        .any(|token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == ';'))
    {
        return None;
    }
    Some((matcher, transcriber))
}

pub(super) fn is_punct(token: &TokenTree, expected: char) -> bool {
    matches!(token, TokenTree::Punct(punct) if punct.as_char() == expected)
}

pub(super) fn normalize_metavariables(
    tokens: TokenStream,
    names: &BTreeMap<String, &'static str>,
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
                let canonical = names.get(&binding.to_string())?;
                output.extend([
                    tokens[index].clone(),
                    TokenTree::Ident(syn::Ident::new(canonical, Span::call_site())),
                ]);
                index += 2;
            }
            TokenTree::Group(group) => {
                let stream = normalize_metavariables(group.stream(), names)?;
                let mut normalized = Group::new(group.delimiter(), stream);
                normalized.set_span(group.span());
                output.extend([TokenTree::Group(normalized)]);
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

#[cfg(test)]
mod tests {
    fn path(segments: &[&str]) -> Vec<String> {
        segments.iter().map(ToString::to_string).collect()
    }

    mod is_dependency_neutral_token_macro {
        use super::super::is_dependency_neutral_token_macro;
        use super::path;

        #[test]
        fn should_classify_standard_token_macros_by_identity() {
            // Arrange
            let unqualified = path(&["stringify"]);
            let standard_qualified = path(&["std", "stringify"]);
            let unrelated_qualified = path(&["application", "stringify"]);
            let expression_macro = path(&["println"]);

            // Act
            let unqualified_result = is_dependency_neutral_token_macro(&unqualified);
            let standard_qualified_result = is_dependency_neutral_token_macro(&standard_qualified);
            let unrelated_qualified_result =
                is_dependency_neutral_token_macro(&unrelated_qualified);
            let expression_macro_result = is_dependency_neutral_token_macro(&expression_macro);

            // Assert
            assert!(unqualified_result);
            assert!(standard_qualified_result);
            assert!(!unrelated_qualified_result);
            assert!(!expression_macro_result);
        }
    }

    mod is_dependency_observing_macro {
        use super::super::is_dependency_observing_macro;
        use super::path;

        #[test]
        fn should_classify_dependency_observing_macros_by_exact_path() {
            // Arrange
            let matches_macro = path(&["matches"]);
            let serde_json_macro = path(&["serde_json", "json"]);
            let nested_matches_macro = path(&["matches", "nested"]);
            let standard_qualified_matches = path(&["std", "matches"]);

            // Act
            let matches_result = is_dependency_observing_macro(&matches_macro);
            let serde_json_result = is_dependency_observing_macro(&serde_json_macro);
            let nested_matches_result = is_dependency_observing_macro(&nested_matches_macro);
            let standard_qualified_result =
                is_dependency_observing_macro(&standard_qualified_matches);

            // Assert
            assert!(matches_result);
            assert!(serde_json_result);
            assert!(!nested_matches_result);
            assert!(!standard_qualified_result);
        }
    }

    mod is_dependency_neutral_expression_macro {
        use super::super::is_dependency_neutral_expression_macro;
        use super::path;

        #[test]
        fn should_classify_expression_macros_with_standard_namespace_aliases() {
            // Arrange
            let unqualified = path(&["assert_eq"]);
            let standard_qualified = path(&["core", "println"]);
            let unrelated_qualified = path(&["application", "println"]);
            let token_macro = path(&["stringify"]);

            // Act
            let unqualified_result = is_dependency_neutral_expression_macro(&unqualified);
            let standard_qualified_result =
                is_dependency_neutral_expression_macro(&standard_qualified);
            let unrelated_qualified_result =
                is_dependency_neutral_expression_macro(&unrelated_qualified);
            let token_macro_result = is_dependency_neutral_expression_macro(&token_macro);

            // Assert
            assert!(unqualified_result);
            assert!(standard_qualified_result);
            assert!(!unrelated_qualified_result);
            assert!(!token_macro_result);
        }
    }
}
