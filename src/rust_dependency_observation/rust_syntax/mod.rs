mod item_macro;
mod macro_safety;
mod macro_syntax;
mod module_index;
mod visitor;

pub(super) use item_macro::{
    SupportedItemMacroDefinition, expand_supported_item_macro, supported_item_macro_definition,
};
pub(super) use macro_safety::local_macro_safety_bindings;
pub(super) use module_index::{
    UseAlias, UseBinding, declared_external_crate_aliases, declared_item_names,
    declared_macro_bindings, declared_self_crate_aliases, flatten_use_tree, glob_imports,
    has_no_implicit_prelude, has_path_attribute, is_crate_visible, item_attributes,
    local_macro_bindings, path_to_string, public_glob_imports, public_item_names,
    public_use_bindings, syn_path_segments, use_aliases,
};
pub(super) use visitor::PathVisitor;

#[cfg(test)]
mod tests {
    use super::path_to_string;

    #[test]
    fn should_join_path_segments_with_the_rust_separator() {
        // Arrange
        let path = vec!["crate".to_owned(), "application".to_owned()];

        // Act
        let value = path_to_string(&path);

        // Assert
        assert_eq!(value, "crate::application");
    }
}
