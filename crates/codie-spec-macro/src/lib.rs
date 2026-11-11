//! Attribute macro used to tag tests with a specification requirement.

use proc_macro::TokenStream;
use syn::{Item, LitStr, parse_macro_input};

/// Tags an item with a requirement identifier such as `AGENT-001`.
///
/// The attribute is erased at compile time and exists so `cargo xtask
/// spec-check` can prove requirement coverage. Use it below `#[test]`:
///
/// ```ignore
/// use codie_testkit::spec;
///
/// #[test]
/// #[spec("AGENT-001")]
/// fn agent_end_is_always_last() {
///     // ...
/// }
/// ```
///
/// Identifiers must match `^[A-Z][A-Z0-9_]{1,8}-\d{3}$`; malformed values fail
/// compilation:
///
/// ```compile_fail
/// use codie_spec_macro::spec;
///
/// #[spec("msg-001")]
/// fn malformed() {}
/// ```
#[proc_macro_attribute]
pub fn spec(attr: TokenStream, item: TokenStream) -> TokenStream {
    let id = parse_macro_input!(attr as LitStr);
    if let Err(message) = validate(id.value().as_str()) {
        return syn::Error::new(id.span(), message)
            .to_compile_error()
            .into();
    }

    match syn::parse::<Item>(item) {
        Ok(item) => quote::quote!(#item).into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Validates a requirement id against `^[A-Z][A-Z0-9_]{1,8}-\d{3}$`.
///
/// Returns a human-readable compiler diagnostic on failure. `spec` attaches the
/// message to the literal's span.
fn validate(value: &str) -> Result<(), &'static str> {
    let Some((area, number)) = value.split_once('-') else {
        return Err("specification id must look like AREA-001");
    };

    let mut characters = area.chars();
    let starts_correctly = characters
        .next()
        .is_some_and(|character| character.is_ascii_uppercase());
    let area_is_valid = starts_correctly
        && (2..=9).contains(&area.len())
        && characters.all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        });
    if !area_is_valid {
        return Err(
            "specification area must be 2-9 characters: an uppercase ASCII letter followed by A-Z, 0-9, or _",
        );
    }

    if number.len() != 3 || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("specification number must be exactly three digits");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn accepts_canonical_ids() {
        for id in [
            "MSG-001",
            "AGENT-999",
            "PROV-010",
            "A_1-001",
            "ABCDEFGHI-123",
        ] {
            assert!(validate(id).is_ok(), "{id} should be accepted");
        }
    }

    #[test]
    fn rejects_malformed_ids() {
        for id in [
            "",
            "MSG001",
            "MSG-1",
            "MSG-0001",
            "msg-001",
            "M-001",
            "TOOLONGAREA-001",
            "1MSG-001",
            "MSG-001-EXTRA",
            "MSG 001",
        ] {
            assert!(validate(id).is_err(), "{id} should be rejected");
        }
    }

    #[test]
    fn error_messages_name_the_malformed_part() {
        assert!(validate("nope").unwrap_err().contains("AREA-001"));
        assert!(validate("msg-001").unwrap_err().contains("area"));
        assert!(validate("MSG-1").unwrap_err().contains("three digits"));
    }
}
