// [GPT-6] Remote-only controls. Paths are replaced in a temporary test harness.
#[path = "ORIGINAL_MODULE"]
mod original;
#[path = "CANDIDATE_MODULE"]
mod candidate;

use core::mem::{MaybeUninit, size_of};
use wasmer_derive::ValueType;

#[derive(Clone, Copy, ValueType)]
#[repr(C)]
struct Named {
    small: u8,
    wide: u32,
    tail: u16,
}

#[derive(Clone, Copy, ValueType)]
#[repr(C)]
struct Tuple(u8, u32);

#[derive(Clone, Copy, ValueType)]
#[repr(transparent)]
struct Transparent(u64);

#[derive(Clone, Copy, ValueType)]
#[repr(C)]
struct Nested {
    tag: u8,
    inner: Named,
}

#[derive(Clone, Copy, ValueType)]
#[repr(C)]
struct Generic<T: wasmer_types::ValueType + Copy> {
    tag: u8,
    inner: T,
}

#[derive(Clone, Copy, ValueType)]
#[repr(C)]
struct Unit;

fn bytes<T: wasmer_types::ValueType>(value: &T) -> Vec<u8> {
    let mut output = vec![MaybeUninit::new(0xa5); size_of::<T>()];
    value.zero_padding_bytes(&mut output);
    output.into_iter().map(|byte| {
        // SAFETY: every element started initialized. The derived code only
        // replaces padding with initialized zero bytes; integer fields do not
        // deinitialize bytes. These fixtures have no other ValueType impls.
        unsafe { byte.assume_init() }
    }).collect()
}

#[test]
fn named_and_tuple_padding_preserve_nonpadding_markers() {
    let n = Named { small: 1, wide: 2, tail: 3 };
    assert_eq!(size_of::<Named>(), 12);
    assert_eq!(core::mem::offset_of!(Named, wide), 4);
    assert_eq!(bytes(&n), [0xa5, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0, 0]);
    assert_eq!(bytes(&Tuple(1, 2)), [0xa5, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5]);
}

#[test]
fn nested_generic_transparent_and_unit_layout() {
    let n = Nested { tag: 1, inner: Named { small: 2, wide: 3, tail: 4 } };
    assert_eq!(core::mem::offset_of!(Nested, inner), 4);
    assert_eq!(bytes(&n), [0xa5, 0, 0, 0, 0xa5, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0, 0]);
    assert_eq!(bytes(&Generic { tag: 1, inner: 2u32 }), bytes(&Tuple(1, 2)));
    assert_eq!(bytes(&Transparent(1)), [0xa5; 8]);
    assert!(bytes(&Unit).is_empty());
}

#[test]
fn valid_generated_tokens_equal_registry_baseline() {
    for source in [
        "#[repr(C)] struct Named { a: u8, b: u32, c: u16 }",
        "#[repr(C)] struct Tuple(u8, u32);",
        "#[repr(transparent)] struct Transparent(u64);",
        "#[repr(C)] struct Generic<T: Copy> { a: T }",
        "#[repr(C)] struct Unit;",
        "#[repr(C, align(16))] struct Aligned { a: u8 }",
    ] {
        let input = syn::parse_str(source).unwrap();
        assert_eq!(original::impl_value_type(&input).to_string(),
                   candidate::impl_value_type(&input).unwrap().to_string(), "{source}");
    }
}

#[test]
fn invalid_shapes_report_expected_errors() {
    for (source, expected) in [
        ("struct Missing { a: u32 }", "ValueType can only be derived for #[repr(C)] or #[repr(transparent)] structs"),
        ("#[repr(C)] enum Wrong { A }", "ValueType can only be derived for structs"),
        ("#[repr(C)] union Wrong { a: u32 }", "ValueType can only be derived for structs"),
    ] {
        let error = candidate::impl_value_type(&syn::parse_str(source).unwrap()).unwrap_err();
        assert_eq!(error.to_string(), expected);
        assert!(error.into_compile_error().to_string().contains("compile_error"));
    }
    // Former parse_meta().unwrap() panic becomes a normal spanned diagnostic.
    let malformed = syn::parse_str("#[repr(C =)] struct Malformed { a: u8 }").unwrap();
    assert!(candidate::impl_value_type(&malformed).is_err());
}
