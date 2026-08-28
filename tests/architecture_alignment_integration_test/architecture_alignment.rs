// integration_id: logical-module-boundary-architecture-alignment

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    process::{Command, Output},
};

use serde_yaml::Value as YamlValue;

const CLI_OPTION_VOCABULARY: &str =
    include_str!("../../sdd/dsl/specs/cli/cli-option-vocabulary.sdd.yml");
const CLI_COMMAND_MANIFEST: &str =
    include_str!("../../sdd/dsl/specs/cli/cli-command-manifest.sdd.yml");
const CONTRACT_CLAUSES: &str = include_str!(
    "../../sdd/dsl/specs/contract-core/subjects/logical-module-boundary-validation/clauses.sdd.yml"
);
const CONTRACT_ROUTE: &str = include_str!(
    "../../sdd/dsl/designs/scopes/logical-module-boundary-validation/contract-route.sdd.yml"
);
const LOGICAL_STRUCTURE: &str = include_str!(
    "../../sdd/dsl/designs/scopes/logical-module-boundary-validation/logical-structure.sdd.yml"
);
const PROJECTION: &str = include_str!(
    "../../sdd/dsl/designs/scopes/logical-module-boundary-validation/projection.sdd.yml"
);
const DIRECTORY_STRUCTURE: &str =
    include_str!("../../sdd/dsl/designs/global/directory-structure.sdd.yml");

#[test]
fn should_cli_help_is_a_projection_of_sdd_command_definitions() {
    // Arrange
    let vocabulary = parse_yaml(CLI_OPTION_VOCABULARY);
    let manifest = parse_yaml(CLI_COMMAND_MANIFEST);
    let cli_root =
        fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sdd/dsl/specs/cli"))
            .expect("CLI specification root should be canonicalizable");
    let commands = sequence(&manifest, "commands");
    assert!(!commands.is_empty(), "CLI manifest must declare a command");
    let command_ids = commands
        .iter()
        .map(|command| string(command, "id"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        commands.len(),
        command_ids.len(),
        "CLI command IDs must be unique"
    );

    let command_inputs = commands
        .iter()
        .map(|entry| {
            let command_id = string(entry, "id").to_owned();
            let relative_path = scalar(&entry["path"], "command path");
            let command_path = fs::canonicalize(cli_root.join(relative_path))
                .unwrap_or_else(|error| panic!("cannot resolve {relative_path}: {error}"));
            assert!(
                command_path.starts_with(&cli_root),
                "command path escapes the CLI specification root: {relative_path}"
            );
            let command_source = fs::read_to_string(&command_path)
                .unwrap_or_else(|error| panic!("cannot read {relative_path}: {error}"));
            let command = parse_yaml(&command_source);
            assert_eq!(string(&command["command"], "id"), command_id);
            let invocation = sequence(&command["command"], "path")
                .iter()
                .map(|token| scalar(token, "command path token").to_owned())
                .collect::<Vec<_>>();
            assert!(!invocation.is_empty(), "command {command_id} has no path");
            let args = invocation
                .into_iter()
                .skip(1)
                .chain(std::iter::once("--help".to_owned()))
                .collect::<Vec<_>>();
            (command, args)
        })
        .collect::<Vec<_>>();

    // Act
    let outputs = command_inputs
        .iter()
        .map(|(_, args)| {
            Command::new(env!("CARGO_BIN_EXE_a3-rust-cruiser"))
                .args(args)
                .output()
                .expect("binary should run")
        })
        .collect::<Vec<_>>();

    // Assert
    for ((command, _), output) in command_inputs.iter().zip(outputs.iter()) {
        assert_cli_help(&vocabulary, command, output);
    }
}

#[test]
fn should_contract_routes_resolve_to_declared_projected_components() {
    // Arrange
    let clauses = parse_yaml(CONTRACT_CLAUSES);
    let route = parse_yaml(CONTRACT_ROUTE);
    let logical = parse_yaml(LOGICAL_STRUCTURE);
    let projection = parse_yaml(PROJECTION);
    let directories = parse_yaml(DIRECTORY_STRUCTURE);
    let contracts = sequence(&clauses, "contracts");
    let contract_ids = contracts
        .iter()
        .map(|contract| string(contract, "id").to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        contracts.len(),
        contract_ids.len(),
        "contract IDs must be unique"
    );
    let components = sequence(&logical, "components");
    let component_ids = components
        .iter()
        .map(|component| string(component, "component_id").to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        components.len(),
        component_ids.len(),
        "component IDs must be unique"
    );
    let projection_mappings = sequence(&projection, "mappings");
    let mappings = projection_mappings
        .iter()
        .map(|mapping| {
            (
                string(mapping, "component_id").to_owned(),
                string(mapping, "node_id").to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        projection_mappings.len(),
        mappings.len(),
        "projection component IDs must be unique"
    );
    let directory_nodes = sequence(&directories, "nodes");
    let nodes = directory_nodes
        .iter()
        .map(|node| {
            (
                string(node, "node_id").to_owned(),
                string(node, "path").to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        directory_nodes.len(),
        nodes.len(),
        "directory node IDs must be unique"
    );
    assert_eq!(
        component_ids,
        mappings.keys().cloned().collect::<BTreeSet<_>>()
    );
    let source_root = fs::canonicalize(env!("CARGO_MANIFEST_DIR"))
        .expect("project root should be canonicalizable");
    let mut route_ids = BTreeSet::new();

    // Act
    let routes = sequence(&route, "routes");

    // Assert
    assert!(!routes.is_empty(), "contract route must declare a route");
    for route in routes {
        let route_id = string(route, "route_id");
        assert!(route_ids.insert(route_id), "duplicate route id: {route_id}");
        let contract_id = string(&route["contract_ref"], "contract_id");
        assert!(
            contract_ids.contains(contract_id),
            "route {route_id} references unknown contract {contract_id}"
        );
        let mut hop_ids = BTreeSet::new();
        for hop in sequence(route, "hops") {
            let component_id = string(hop, "component_id");
            assert!(
                component_ids.contains(component_id),
                "route {route_id} references unknown component {component_id}"
            );
            let node_id = mappings
                .get(component_id)
                .unwrap_or_else(|| panic!("component {component_id} has no projection"));
            let path = nodes
                .get(node_id)
                .unwrap_or_else(|| panic!("projection references unknown node {node_id}"));
            let projected_path = fs::canonicalize(source_root.join(path))
                .unwrap_or_else(|error| panic!("cannot resolve projection path {path}: {error}"));
            assert!(
                projected_path.starts_with(&source_root),
                "projection path escapes the project root: {path}"
            );
            assert!(hop_ids.insert(component_id));
        }
        assert!(!hop_ids.is_empty(), "route {route_id} has no hops");
        for edge in optional_sequence(route, "edge_semantics") {
            let from = string(edge, "from_component_id");
            let to = string(edge, "to_component_id");
            assert!(
                hop_ids.contains(from),
                "edge source is not a route hop: {from}"
            );
            assert!(hop_ids.contains(to), "edge target is not a route hop: {to}");
        }
    }
}

fn assert_cli_help(vocabulary: &YamlValue, command: &YamlValue, output: &Output) {
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout.clone()).expect("help is UTF-8");
    assert_eq!(
        usage_line(&stdout),
        format!("Usage: {}", string(&command["command"]["help"], "usage"))
    );
    assert_eq!(
        option_tokens(&stdout),
        expected_option_tokens(vocabulary, command),
        "CLI help option surface differs from SDD command definition"
    );
}

fn expected_option_tokens(vocabulary: &YamlValue, command: &YamlValue) -> BTreeSet<String> {
    let options = sequence(vocabulary, "options");
    let definitions = options
        .iter()
        .map(|option| (string(option, "id"), option))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        options.len(),
        definitions.len(),
        "CLI option IDs must be unique"
    );
    let ids = sequence(&command["command"], "option_groups")
        .iter()
        .flat_map(|group| sequence(group, "options"))
        .map(|option| scalar(option, "option id"));
    let mut tokens = BTreeSet::new();
    for id in ids {
        let option = definitions
            .get(id)
            .unwrap_or_else(|| panic!("command references unknown option {id}"));
        for key in ["long", "short"] {
            if let Some(token) = option[key].as_str() {
                tokens.insert(token.to_owned());
            }
        }
    }
    tokens
}

fn option_tokens(help: &str) -> BTreeSet<String> {
    help.split_whitespace()
        .map(|token| {
            token.trim_matches(|character| matches!(character, ',' | '(' | ')' | '[' | ']'))
        })
        .filter(|token| token.starts_with("--") || (token.starts_with('-') && token.len() == 2))
        .map(str::to_owned)
        .collect()
}

fn usage_line(help: &str) -> &str {
    help.lines()
        .find(|line| line.starts_with("Usage: "))
        .expect("help should contain a Usage line")
}

fn parse_yaml(source: &str) -> YamlValue {
    serde_yaml::from_str(source).expect("SDD fixture should be valid YAML")
}

fn sequence<'a>(value: &'a YamlValue, key: &str) -> &'a [YamlValue] {
    value[key]
        .as_sequence()
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("{key} should be a YAML sequence"))
}

fn optional_sequence<'a>(value: &'a YamlValue, key: &str) -> &'a [YamlValue] {
    value[key].as_sequence().map_or(&[], Vec::as_slice)
}

fn string<'a>(value: &'a YamlValue, key: &str) -> &'a str {
    scalar(&value[key], key)
}

fn scalar<'a>(value: &'a YamlValue, context: &str) -> &'a str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{context} should be a YAML string"))
}
