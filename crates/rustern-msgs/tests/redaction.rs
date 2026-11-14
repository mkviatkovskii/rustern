//! FR-017: `Debug` of every generated type with a secret field (cookie, certificate,
//! block proof, secret key) prints `<redacted>` for that field, never its bytes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code

use rustern_msgs::SECRET_FIELDS;

/// Every `<secret field>: ` in `debug` is followed by `<redacted>`; returns how many.
fn redacted_fields(debug: &str) -> usize {
    let mut found = 0;
    for field in SECRET_FIELDS {
        let pat = format!("{field}: ");
        for (i, _) in debug.match_indices(&pat) {
            let rest = &debug[i + pat.len()..];
            assert!(
                rest.starts_with("<redacted>"),
                "{field} not redacted in {debug}"
            );
            found += 1;
        }
    }
    found
}

#[test]
fn every_secret_field_is_redacted() {
    let mut types = 0;
    macro_rules! check {
        ($ty:ident) => {{
            let debug = format!("{:?}", rustern_msgs::$ty::default());
            assert!(
                redacted_fields(&debug) > 0,
                "{}: no redacted field in {debug}",
                stringify!($ty)
            );
            types += 1;
        }};
    }
    rustern_msgs::for_each_secret_type!(check);
    assert!(types >= 10, "only {types} types with secrets");
}

#[test]
fn secret_bytes_never_printed() {
    let cookie = rustern_msgs::wire::Secret([0xA5u8; 8]);
    let debug = format!("{cookie:?}");
    assert_eq!(debug, "<redacted>");
    assert!(!debug.contains("165"));
}
