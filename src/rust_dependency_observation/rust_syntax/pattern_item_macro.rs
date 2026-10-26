use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Delimiter, Group, Ident, Span, TokenStream, TokenTree};

use super::macro_syntax::is_punct;

#[derive(Clone)]
pub(super) struct MacroRules {
    rules: Vec<MacroRule>,
}

#[derive(Clone)]
struct MacroRule {
    matcher: Vec<MatcherNode>,
    transcriber: TokenStream,
    meta_bindings: BTreeSet<String>,
}

#[derive(Clone)]
enum MatcherNode {
    Token(TokenTree),
    Group(Delimiter, Vec<MatcherNode>),
    Fragment(String, Fragment),
    Repetition(Repetition),
}

#[derive(Clone)]
struct Repetition {
    body: Vec<MatcherNode>,
    separator: Option<TokenTree>,
    operator: char,
    bindings: Vec<String>,
}

#[derive(Clone, Copy)]
enum Fragment {
    Ident,
    Literal,
    Meta,
    Path,
}

#[derive(Clone)]
enum Binding {
    Tokens(Fragment, TokenStream),
    Repeated(Vec<Binding>),
}

type Bindings = BTreeMap<String, Binding>;

pub(super) fn macro_rules(tokens: TokenStream) -> Option<MacroRules> {
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let mut rules = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let (
            Some(TokenTree::Group(matcher)),
            Some(equals),
            Some(greater),
            Some(TokenTree::Group(transcriber)),
        ) = (
            tokens.get(index),
            tokens.get(index + 1),
            tokens.get(index + 2),
            tokens.get(index + 3),
        )
        else {
            return None;
        };
        if !is_punct(equals, '=') || !is_punct(greater, '>') {
            return None;
        }
        rules.push(macro_rule(matcher.stream(), transcriber.stream())?);
        index += 4;
        match tokens.get(index) {
            Some(separator) if is_punct(separator, ';') => index += 1,
            Some(_) => return None,
            None => {}
        }
    }
    (!rules.is_empty()).then_some(MacroRules { rules })
}

/// `call_site` replaces the spans of transcriber tokens when the definition
/// lives in another file, so evidence points into the invoking module.
pub(super) fn expand_macro_rules(
    definition: &MacroRules,
    arguments: TokenStream,
    call_site: Option<Span>,
) -> Option<TokenStream> {
    let tokens = arguments.into_iter().collect::<Vec<_>>();
    for rule in &definition.rules {
        let mut bindings = Bindings::new();
        if match_nodes(&rule.matcher, &tokens, &mut bindings) != Some(tokens.len()) {
            // rustc would pick a later arm only after this arm fails. A failure
            // is trusted when it cannot come from the greedy subset matcher.
            if is_exact_matcher(&rule.matcher) || leading_token_mismatch(&rule.matcher, &tokens) {
                continue;
            }
            return None;
        }
        // A bound meta fragment would carry attribute paths and cfg predicates
        // into the generated items, which this limited expansion does not evaluate.
        if rule
            .meta_bindings
            .iter()
            .any(|name| bindings.get(name).is_some_and(has_tokens))
        {
            return None;
        }
        let scope = bindings
            .iter()
            .map(|(name, binding)| (name.as_str(), binding))
            .collect();
        return transcribe(rule.transcriber.clone(), &scope, call_site);
    }
    None
}

fn macro_rule(matcher: TokenStream, transcriber: TokenStream) -> Option<MacroRule> {
    let matcher = parse_matcher(matcher)?;
    let mut bindings = Vec::new();
    collect_matcher_bindings(&matcher, &mut bindings);
    let unique = bindings.iter().cloned().collect::<BTreeSet<_>>();
    if unique.len() != bindings.len() {
        return None;
    }
    let mut meta_bindings = BTreeSet::new();
    collect_meta_bindings(&matcher, &mut meta_bindings);
    Some(MacroRule {
        matcher,
        transcriber,
        meta_bindings,
    })
}

fn parse_matcher(tokens: TokenStream) -> Option<Vec<MatcherNode>> {
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let mut nodes = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Punct(dollar) if dollar.as_char() == '$' => match tokens.get(index + 1)? {
                TokenTree::Ident(name) if name != "crate" => {
                    let (TokenTree::Punct(colon), TokenTree::Ident(fragment)) =
                        (tokens.get(index + 2)?, tokens.get(index + 3)?)
                    else {
                        return None;
                    };
                    if colon.as_char() != ':' {
                        return None;
                    }
                    let fragment = match fragment.to_string().as_str() {
                        "ident" => Fragment::Ident,
                        "literal" => Fragment::Literal,
                        "meta" => Fragment::Meta,
                        "path" => Fragment::Path,
                        _ => return None,
                    };
                    nodes.push(MatcherNode::Fragment(name.to_string(), fragment));
                    index += 4;
                }
                TokenTree::Group(group) if group.delimiter() == Delimiter::Parenthesis => {
                    let body = parse_matcher(group.stream())?;
                    let (separator, operator, consumed) = repetition_suffix(&tokens[index + 2..])?;
                    let mut bindings = Vec::new();
                    collect_matcher_bindings(&body, &mut bindings);
                    nodes.push(MatcherNode::Repetition(Repetition {
                        body,
                        separator,
                        operator,
                        bindings,
                    }));
                    index += 2 + consumed;
                }
                _ => return None,
            },
            TokenTree::Group(group) => {
                nodes.push(MatcherNode::Group(
                    group.delimiter(),
                    parse_matcher(group.stream())?,
                ));
                index += 1;
            }
            token => {
                nodes.push(MatcherNode::Token(token.clone()));
                index += 1;
            }
        }
    }
    Some(nodes)
}

fn repetition_suffix(tokens: &[TokenTree]) -> Option<(Option<TokenTree>, char, usize)> {
    let operator = |token: Option<&TokenTree>| match token {
        Some(TokenTree::Punct(punct)) if matches!(punct.as_char(), '*' | '+' | '?') => {
            Some(punct.as_char())
        }
        _ => None,
    };
    if let Some(found) = operator(tokens.first())
        && operator(tokens.get(1)).is_none()
    {
        return Some((None, found, 1));
    }
    let separator = match tokens.first()? {
        TokenTree::Group(_) => return None,
        TokenTree::Punct(punct) if punct.as_char() == '$' => return None,
        separator => separator.clone(),
    };
    match operator(tokens.get(1))? {
        '?' => None,
        found => Some((Some(separator), found, 2)),
    }
}

fn collect_matcher_bindings(nodes: &[MatcherNode], bindings: &mut Vec<String>) {
    for node in nodes {
        match node {
            MatcherNode::Fragment(name, _) => bindings.push(name.clone()),
            MatcherNode::Group(_, nodes) => collect_matcher_bindings(nodes, bindings),
            MatcherNode::Repetition(repetition) => {
                bindings.extend(repetition.bindings.iter().cloned());
            }
            MatcherNode::Token(_) => {}
        }
    }
}

fn collect_meta_bindings(nodes: &[MatcherNode], bindings: &mut BTreeSet<String>) {
    for node in nodes {
        match node {
            MatcherNode::Fragment(name, Fragment::Meta) => {
                bindings.insert(name.clone());
            }
            MatcherNode::Group(_, nodes) => collect_meta_bindings(nodes, bindings),
            MatcherNode::Repetition(repetition) => {
                collect_meta_bindings(&repetition.body, bindings);
            }
            MatcherNode::Fragment(..) | MatcherNode::Token(_) => {}
        }
    }
}

/// Matchers built only from tokens, groups, and single-token-shaped fragments
/// match exactly as rustc does, so their failure is definitive.
fn is_exact_matcher(nodes: &[MatcherNode]) -> bool {
    nodes.iter().all(|node| match node {
        MatcherNode::Token(_) => true,
        MatcherNode::Group(_, nodes) => is_exact_matcher(nodes),
        MatcherNode::Fragment(_, fragment) => {
            matches!(fragment, Fragment::Ident | Fragment::Literal)
        }
        MatcherNode::Repetition(_) => false,
    })
}

fn leading_token_mismatch(nodes: &[MatcherNode], tokens: &[TokenTree]) -> bool {
    for (index, node) in nodes.iter().enumerate() {
        let MatcherNode::Token(expected) = node else {
            return false;
        };
        if !tokens
            .get(index)
            .is_some_and(|actual| same_token(expected, actual))
        {
            return true;
        }
    }
    false
}

fn has_tokens(binding: &Binding) -> bool {
    match binding {
        Binding::Tokens(..) => true,
        Binding::Repeated(bindings) => bindings.iter().any(has_tokens),
    }
}

fn match_nodes(
    nodes: &[MatcherNode],
    tokens: &[TokenTree],
    bindings: &mut Bindings,
) -> Option<usize> {
    let mut position = 0;
    for node in nodes {
        position += match_node(node, &tokens[position..], bindings)?;
    }
    Some(position)
}

fn match_node(node: &MatcherNode, tokens: &[TokenTree], bindings: &mut Bindings) -> Option<usize> {
    match node {
        MatcherNode::Token(expected) => same_token(expected, tokens.first()?).then_some(1),
        MatcherNode::Group(delimiter, inner) => {
            let TokenTree::Group(group) = tokens.first()? else {
                return None;
            };
            if group.delimiter() != *delimiter {
                return None;
            }
            let group_tokens = group.stream().into_iter().collect::<Vec<_>>();
            (match_nodes(inner, &group_tokens, bindings)? == group_tokens.len()).then_some(1)
        }
        MatcherNode::Fragment(name, fragment) => {
            let length = fragment_length(*fragment, tokens)?;
            bindings.insert(
                name.clone(),
                Binding::Tokens(*fragment, tokens[..length].iter().cloned().collect()),
            );
            Some(length)
        }
        MatcherNode::Repetition(repetition) => match_repetition(repetition, tokens, bindings),
    }
}

/// Repetitions match greedily without backtracking. A compiling invocation has
/// a unique parse, so a greedy success is that parse; inputs that would need
/// backtracking stay unexpanded.
fn match_repetition(
    repetition: &Repetition,
    tokens: &[TokenTree],
    bindings: &mut Bindings,
) -> Option<usize> {
    let mut iterations = Vec::new();
    let mut position = 0;
    while repetition.operator != '?' || iterations.is_empty() {
        let mut start = position;
        if let Some(separator) = &repetition.separator
            && !iterations.is_empty()
        {
            match tokens.get(position) {
                Some(actual) if same_token(separator, actual) => start += 1,
                _ => break,
            }
        }
        let mut iteration = Bindings::new();
        match match_nodes(&repetition.body, &tokens[start..], &mut iteration) {
            Some(length) if length > 0 => {
                position = start + length;
                iterations.push(iteration);
            }
            _ => break,
        }
    }
    if repetition.operator == '+' && iterations.is_empty() {
        return None;
    }
    for name in &repetition.bindings {
        bindings.insert(
            name.clone(),
            Binding::Repeated(
                iterations
                    .iter()
                    .map(|iteration| iteration[name].clone())
                    .collect(),
            ),
        );
    }
    Some(position)
}

fn fragment_length(fragment: Fragment, tokens: &[TokenTree]) -> Option<usize> {
    if let Some(TokenTree::Group(group)) = tokens.first()
        && group.delimiter() == Delimiter::None
    {
        // A forwarded fragment matches only a fragment that accepts its whole
        // content; ident fragments are never forwarded this way.
        let inner = group.stream().into_iter().collect::<Vec<_>>();
        let accepted = !matches!(fragment, Fragment::Ident)
            && fragment_length(fragment, &inner) == Some(inner.len());
        return accepted.then_some(1);
    }
    match fragment {
        Fragment::Ident => match tokens.first()? {
            TokenTree::Ident(ident) if ident != "_" => Some(1),
            _ => None,
        },
        Fragment::Literal => match tokens {
            [TokenTree::Literal(_), ..] => Some(1),
            [TokenTree::Ident(ident), ..] if ident == "true" || ident == "false" => Some(1),
            [TokenTree::Punct(minus), TokenTree::Literal(_), ..] if minus.as_char() == '-' => {
                Some(2)
            }
            _ => None,
        },
        Fragment::Meta => meta_length(tokens),
        Fragment::Path => path_length(tokens),
    }
}

/// Covers paths without generic arguments; a generic path leaves the
/// invocation unexpanded because the following matcher token cannot match `<`.
fn path_length(tokens: &[TokenTree]) -> Option<usize> {
    let is_path_separator = |tokens: Option<&[TokenTree]>| {
        matches!(tokens, Some([TokenTree::Punct(first), TokenTree::Punct(second)])
            if first.as_char() == ':' && second.as_char() == ':')
    };
    let mut length = if is_path_separator(tokens.get(..2)) {
        2
    } else {
        0
    };
    loop {
        let TokenTree::Ident(_) = tokens.get(length)? else {
            return None;
        };
        length += 1;
        if !is_path_separator(tokens.get(length..length + 2)) {
            return Some(length);
        }
        length += 2;
    }
}

/// Covers `path`, `path(...)`, and `path = literal`; other attribute inputs
/// leave the invocation unexpanded.
fn meta_length(tokens: &[TokenTree]) -> Option<usize> {
    let length = path_length(tokens)?;
    match tokens.get(length..) {
        Some([TokenTree::Group(_), ..]) => Some(length + 1),
        Some([TokenTree::Punct(equals), TokenTree::Literal(_), ..]) if equals.as_char() == '=' => {
            Some(length + 2)
        }
        _ => Some(length),
    }
}

fn same_token(expected: &TokenTree, actual: &TokenTree) -> bool {
    match (expected, actual) {
        (TokenTree::Ident(expected), TokenTree::Ident(actual)) => expected == actual,
        (TokenTree::Punct(expected), TokenTree::Punct(actual)) => {
            expected.as_char() == actual.as_char()
        }
        (TokenTree::Literal(expected), TokenTree::Literal(actual)) => {
            expected.to_string() == actual.to_string()
        }
        _ => false,
    }
}

/// Borrowed so each repetition iteration rebinds names without copying the
/// repeated values of every other binding.
type Scope<'a> = BTreeMap<&'a str, &'a Binding>;

fn transcribe(
    tokens: TokenStream,
    bindings: &Scope<'_>,
    call_site: Option<Span>,
) -> Option<TokenStream> {
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let mut output = TokenStream::new();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Punct(dollar) if dollar.as_char() == '$' => match tokens.get(index + 1)? {
                TokenTree::Ident(name) if name == "crate" => {
                    let span = call_site.unwrap_or_else(|| name.span());
                    output.extend([TokenTree::Ident(Ident::new("crate", span))]);
                    index += 2;
                }
                TokenTree::Ident(name) => {
                    let Binding::Tokens(fragment, value) =
                        bindings.get(name.to_string().as_str())?
                    else {
                        return None;
                    };
                    output.extend(forwarded_fragment(*fragment, value.clone()));
                    index += 2;
                }
                TokenTree::Group(group) if group.delimiter() == Delimiter::Parenthesis => {
                    let (separator, operator, consumed) = repetition_suffix(&tokens[index + 2..])?;
                    output.extend(transcribe_repetition(
                        group.stream(),
                        separator.map(|separator| with_call_site(separator, call_site)),
                        operator,
                        bindings,
                        call_site,
                    )?);
                    index += 2 + consumed;
                }
                _ => return None,
            },
            TokenTree::Group(group) => {
                let mut transcribed = Group::new(
                    group.delimiter(),
                    transcribe(group.stream(), bindings, call_site)?,
                );
                transcribed.set_span(call_site.unwrap_or_else(|| group.span()));
                output.extend([TokenTree::Group(transcribed)]);
                index += 1;
            }
            token => {
                output.extend([with_call_site(token.clone(), call_site)]);
                index += 1;
            }
        }
    }
    Some(output)
}

/// rustc forwards every fragment except `ident` as an invisible group, so a
/// nested matcher cannot split it or match it against literal tokens.
fn forwarded_fragment(fragment: Fragment, tokens: TokenStream) -> TokenStream {
    match fragment {
        Fragment::Ident => tokens,
        Fragment::Literal | Fragment::Meta | Fragment::Path => {
            TokenStream::from(TokenTree::Group(Group::new(Delimiter::None, tokens)))
        }
    }
}

fn with_call_site(mut token: TokenTree, call_site: Option<Span>) -> TokenTree {
    if let Some(span) = call_site {
        token.set_span(span);
    }
    token
}

fn transcribe_repetition(
    body: TokenStream,
    separator: Option<TokenTree>,
    operator: char,
    bindings: &Scope<'_>,
    call_site: Option<Span>,
) -> Option<TokenStream> {
    let mut names = BTreeSet::new();
    collect_transcriber_bindings(body.clone(), &mut names);
    let repeated = names
        .iter()
        .filter_map(|name| match bindings.get(name.as_str()) {
            Some(Binding::Repeated(values)) => Some((name, values)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let count = repeated.first()?.1.len();
    if repeated.iter().any(|(_, values)| values.len() != count) || (operator == '?' && count > 1) {
        return None;
    }
    let mut output = TokenStream::new();
    for iteration in 0..count {
        if iteration > 0
            && let Some(separator) = &separator
        {
            output.extend([separator.clone()]);
        }
        let mut scoped = bindings.clone();
        for (name, values) in &repeated {
            scoped.insert(name.as_str(), &values[iteration]);
        }
        output.extend(transcribe(body.clone(), &scoped, call_site)?);
    }
    Some(output)
}

fn collect_transcriber_bindings(tokens: TokenStream, names: &mut BTreeSet<String>) {
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Punct(dollar) if dollar.as_char() == '$' => {
                if let Some(TokenTree::Ident(name)) = tokens.get(index + 1) {
                    names.insert(name.to_string());
                }
            }
            TokenTree::Group(group) => collect_transcriber_bindings(group.stream(), names),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use proc_macro2::{TokenStream, TokenTree};

    use super::{expand_macro_rules, macro_rules};

    fn tokens(source: &str) -> TokenStream {
        source.parse().expect("test tokens must parse")
    }

    fn token_texts(tokens: TokenStream) -> Vec<String> {
        tokens
            .into_iter()
            .flat_map(|token| match token {
                TokenTree::Group(group) => token_texts(group.stream()),
                token => vec![token.to_string()],
            })
            .collect()
    }

    fn expand(definition: &str, arguments: &str) -> Option<Vec<String>> {
        let definition = macro_rules(tokens(definition)).expect("definition must be supported");
        expand_macro_rules(&definition, tokens(arguments), None).map(token_texts)
    }

    #[test]
    fn should_expand_nested_repetitions_with_separators() {
        // Arrange
        let definition = "($($(#[$attr:meta])* $key:ident => $path:literal),+ $(,)?) => { enum Text { $($(#[$attr])* $key),+ } const PATHS: &[&str] = &[$($path),+]; };";

        // Act
        let expanded = expand(definition, "A => \"a\", B => \"b\",");

        // Assert
        assert_eq!(
            expanded,
            Some(token_texts(tokens(
                "enum Text { A, B } const PATHS: &[&str] = &[\"a\", \"b\"];"
            )))
        );
    }

    #[test]
    fn should_select_first_matching_rule_and_bind_paths() {
        // Arrange
        let definition = "(@observer) => { None }; (@observer $observer:path) => { Some($observer) }; ($name:ident $(, observer = $observer:path)?) => { const $name: u8 = 0; };";

        // Act
        let absent = expand(definition, "@observer");
        let present = expand(definition, "@observer super::module::observe");
        let item = expand(definition, "VALUE, observer = crate::observe");

        // Assert
        assert_eq!(absent, Some(token_texts(tokens("None"))));
        assert_eq!(
            present,
            Some(token_texts(tokens("Some(super::module::observe)")))
        );
        assert_eq!(item, Some(token_texts(tokens("const VALUE: u8 = 0;"))));
    }

    #[test]
    fn should_not_fall_through_uncertain_rule_failure() {
        // Arrange
        let definition = "($($first:ident)* last) => { const A: u8 = 0; }; ($($other:ident)*) => { const B: u8 = 0; };";

        // Act
        let expanded = expand(definition, "one last");

        // Assert
        assert!(expanded.is_none());
    }

    #[test]
    fn should_expand_many_repetitions_without_deep_recursion() {
        // Arrange
        let arguments = (0..10_000)
            .map(|index| format!("K{index} => \"k{index}\""))
            .collect::<Vec<_>>()
            .join(", ");

        // Act
        let expanded = expand(
            "($($key:ident => $path:literal),+) => { enum Text { $($key),+ } };",
            &arguments,
        );

        // Assert
        assert_eq!(expanded.map(|texts| texts.len()), Some(20_001));
    }

    #[test]
    fn should_accept_identifier_separator() {
        // Arrange
        let definition = "($($name:ident)and*) => { $(fn $name() {})* };";

        // Act
        let expanded = expand(definition, "first and second");

        // Assert
        assert_eq!(
            expanded,
            Some(token_texts(tokens("fn first() {} fn second() {}")))
        );
    }

    #[test]
    fn should_reject_bound_meta_fragment() {
        // Arrange
        let definition =
            "($($(#[$attr:meta])* $key:ident),+) => { enum Text { $($(#[$attr])* $key),+ } };";

        // Act
        let expanded = expand(definition, "#[cfg(test)] A, B");

        // Assert
        assert!(expanded.is_none());
    }

    #[test]
    fn should_reject_unsupported_fragment_and_duplicate_binding() {
        // Arrange
        let definitions = [
            "($value:expr) => {};",
            "($value:ident $value:ident) => {};",
            "($value:ident) => {}; junk",
        ];

        // Act
        let parsed = definitions.map(|definition| macro_rules(tokens(definition)).is_some());

        // Assert
        assert_eq!(parsed, [false, false, false]);
    }
}
