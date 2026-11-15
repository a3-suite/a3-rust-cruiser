//! 依存を生じないと分かっている名前の一覧を集める。走査処理はここを参照して判定し、
//! 名前を追加するときはこのファイルだけを変更する。

pub(super) fn is_builtin_attribute_name(name: &str) -> bool {
    matches!(
        name,
        "allow"
            | "automatically_derived"
            | "cold"
            | "deprecated"
            | "default"
            | "deny"
            | "doc"
            | "expect"
            | "export_name"
            | "forbid"
            | "global_allocator"
            | "ignore"
            | "inline"
            | "link"
            | "link_name"
            | "link_section"
            | "macro_export"
            | "macro_use"
            | "must_use"
            | "no_main"
            | "no_implicit_prelude"
            | "no_mangle"
            | "no_std"
            | "non_exhaustive"
            | "panic_handler"
            | "path"
            | "proc_macro"
            | "proc_macro_attribute"
            | "proc_macro_derive"
            | "repr"
            | "should_panic"
            | "target_feature"
            | "test"
            | "track_caller"
            | "used"
            | "warn"
    )
}

pub(super) fn is_builtin_derive_name(name: &str) -> bool {
    matches!(
        name,
        "Clone" | "Copy" | "Debug" | "Default" | "Eq" | "Hash" | "Ord" | "PartialEq" | "PartialOrd"
    )
}

pub(super) fn is_known_external_derive_crate(name: &str) -> bool {
    matches!(name, "serde")
}

pub(super) fn is_serde_neutral_flag(name: &str) -> bool {
    matches!(
        name,
        "alias"
            | "content"
            | "default"
            | "deny_unknown_fields"
            | "flatten"
            | "other"
            | "rename_all"
            | "skip"
            | "skip_deserializing"
            | "skip_serializing"
            | "transparent"
            | "untagged"
    )
}

pub(super) fn is_serde_neutral_literal_key(name: &str) -> bool {
    matches!(
        name,
        "alias" | "content" | "expecting" | "rename" | "rename_all" | "rename_all_fields" | "tag"
    )
}

pub(super) fn is_serde_path_valued_key(name: &str) -> bool {
    matches!(
        name,
        "default" | "deserialize_with" | "serialize_with" | "skip_serializing_if" | "with"
    )
}

pub(crate) fn known_external_crate(name: &str) -> bool {
    matches!(name, "alloc" | "core" | "proc_macro" | "std" | "test")
}

pub(crate) fn known_language_name(name: &str) -> bool {
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

pub(crate) fn known_prelude_name(name: &str) -> bool {
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
