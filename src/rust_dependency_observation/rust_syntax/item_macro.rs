use proc_macro2::{Span, TokenStream};
use syn::{Expr, File, Item};

use super::pattern_item_macro::{MacroRules, expand_macro_rules, macro_rules};

#[derive(Clone)]
pub(crate) struct SupportedItemMacroDefinition(MacroRules);

pub(crate) fn supported_item_macro_definition(
    item: &syn::ItemMacro,
) -> Option<SupportedItemMacroDefinition> {
    if !item.mac.path.is_ident("macro_rules") || !item.attrs.is_empty() {
        return None;
    }
    macro_rules(item.mac.tokens.clone()).map(SupportedItemMacroDefinition)
}

pub(crate) fn expand_supported_item_macro(
    definition: &SupportedItemMacroDefinition,
    arguments: TokenStream,
    call_site: Option<Span>,
) -> Option<Vec<Item>> {
    let file =
        syn::parse2::<File>(expand_macro_rules(&definition.0, arguments, call_site)?).ok()?;
    file.attrs.is_empty().then_some(file.items)
}

pub(crate) fn expand_supported_expression_macro(
    definition: &SupportedItemMacroDefinition,
    arguments: TokenStream,
    call_site: Option<Span>,
) -> Option<Expr> {
    syn::parse2::<Expr>(expand_macro_rules(&definition.0, arguments, call_site)?).ok()
}
