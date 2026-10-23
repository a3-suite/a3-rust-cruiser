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
const CONTRACT_SUBJECT: &str = include_str!(
    "../../sdd/dsl/specs/contract-core/subjects/logical-module-boundary-validation/subject.sdd.yml"
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
const PHYSICAL_STRUCTURE: &str =
    include_str!("../../sdd/dsl/designs/global/physical-structure.sdd.yml");

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
    let subject = parse_yaml(CONTRACT_SUBJECT);
    let route = parse_yaml(CONTRACT_ROUTE);
    let logical = parse_yaml(LOGICAL_STRUCTURE);
    let projection = parse_yaml(PROJECTION);
    let physical = parse_yaml(PHYSICAL_STRUCTURE);
    let contracts = sequence(&subject, "contracts");
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
    let mapping_pairs = projection_mappings
        .iter()
        .map(|mapping| {
            (
                string(mapping, "component_id").to_owned(),
                string(mapping, "element_id").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        projection_mappings.len(),
        mapping_pairs.iter().cloned().collect::<BTreeSet<_>>().len(),
        "projection mapping pairs must be unique"
    );
    let mut component_elements = BTreeMap::<String, Vec<String>>::new();
    let mut element_owners = BTreeMap::<String, BTreeSet<String>>::new();
    for (component_id, element_id) in &mapping_pairs {
        component_elements
            .entry(component_id.clone())
            .or_default()
            .push(element_id.clone());
        element_owners
            .entry(element_id.clone())
            .or_default()
            .insert(component_id.clone());
    }
    let elements = sequence(&physical, "elements");
    let element_paths = elements
        .iter()
        .map(|element| {
            (
                string(element, "element_id").to_owned(),
                string(element, "path").to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        elements.len(),
        element_paths.len(),
        "physical element IDs must be unique"
    );
    assert_eq!(
        component_ids,
        component_elements.keys().cloned().collect::<BTreeSet<_>>()
    );
    assert_eq!(
        element_paths.keys().cloned().collect::<BTreeSet<_>>(),
        element_owners.keys().cloned().collect::<BTreeSet<_>>(),
        "every physical element must be owned by the current projection family"
    );
    for (element_id, owners) in &element_owners {
        assert_eq!(
            owners.len(),
            1,
            "physical element {element_id} must have exactly one projection owner"
        );
    }
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
        let contract_id = string(route, "contract_id");
        assert!(
            contract_ids.contains(contract_id),
            "route {route_id} references unknown contract {contract_id}"
        );
        let mut hop_ids = BTreeSet::new();
        for hop in sequence(route, "hops") {
            let component_id = scalar(hop, "hop");
            assert!(
                component_ids.contains(component_id),
                "route {route_id} references unknown component {component_id}"
            );
            let element_ids = component_elements
                .get(component_id)
                .unwrap_or_else(|| panic!("component {component_id} has no projection"));
            for element_id in element_ids {
                let path = element_paths.get(element_id).unwrap_or_else(|| {
                    panic!("projection references unknown element {element_id}")
                });
                let projected_path =
                    fs::canonicalize(source_root.join(path)).unwrap_or_else(|error| {
                        panic!("cannot resolve projection path {path}: {error}")
                    });
                assert!(
                    projected_path.starts_with(&source_root),
                    "projection path escapes the project root: {path}"
                );
            }
            assert!(hop_ids.insert(component_id));
        }
        assert!(!hop_ids.is_empty(), "route {route_id} has no hops");
    }
}

fn assert_cli_help(vocabulary: &YamlValue, command: &YamlValue, output: &Output) {
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout.clone()).expect("help is UTF-8");
    let usage = sequence(&command["command"], "usage");
    assert_eq!(usage.len(), 1, "CLI help prints a single Usage line");
    assert_eq!(
        usage_line(&stdout),
        format!("Usage: {}", scalar(&usage[0], "usage"))
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

fn string<'a>(value: &'a YamlValue, key: &str) -> &'a str {
    scalar(&value[key], key)
}

fn scalar<'a>(value: &'a YamlValue, context: &str) -> &'a str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{context} should be a YAML string"))
}
