use super::*;
use std::collections::BTreeSet;

fn placeholders(text: &str) -> BTreeSet<&str> {
    let mut rest = text;
    let mut fields = BTreeSet::new();
    while let Some(start) = rest.find('{') {
        let end = rest[start..].find('}').expect("Closed placeholder") + start;
        fields.insert(&rest[start + 1..end]);
        rest = &rest[end + 1..];
    }
    fields
}

#[test]
fn every_language_covers_the_catalog_and_preserves_named_fields() {
    let english: BTreeMap<String, String> = serde_json::from_str(include_str!("en.json")).unwrap();
    assert!(english.len() > 500);
    for (code, json) in CATALOGS {
        let messages: BTreeMap<String, String> = serde_json::from_str(json).unwrap();
        assert_eq!(
            messages.keys().collect::<Vec<_>>(),
            english.keys().collect::<Vec<_>>(),
            "Catalog coverage: {code}"
        );
        for (source, translated) in messages {
            assert!(
                !translated.trim().is_empty(),
                "Empty translation: {code}: {source}"
            );
            assert_eq!(
                placeholders(&source),
                placeholders(&translated),
                "Fields: {code}: {source}"
            );
        }
    }
}

#[test]
fn locale_variants_and_preference_lists_resolve_with_english_fallback() {
    for (input, expected) in [
        ("fr_CA.UTF-8", "fr"),
        ("es-MX", "es"),
        ("pt_BR", "pt"),
        ("pt_PT.UTF-8", "pt"),
        ("zh-Hans-CN", "zh_CN"),
        ("zh_SG.UTF-8", "zh_CN"),
        ("hi_IN", "hi"),
        ("ar_EG", "ar"),
        ("en_GB", "en"),
        ("C.UTF-8", "en"),
    ] {
        assert_eq!(resolve(input), Some(expected));
    }
    assert_eq!(resolve("zh_TW"), None);
    assert_eq!(resolve("invalid"), None);
    assert_eq!(system_language("de:es:fr", "en_US.UTF-8"), "es");
    assert_eq!(system_language("de", "fr_FR.UTF-8"), "fr");
    assert_eq!(system_language("de", "de_DE.UTF-8"), "en");
    assert_eq!(system_language("fr", "C"), "en");
    assert_eq!(system_language("ar", "C.UTF-8"), "ar");
}

#[test]
fn interpolation_reorders_fields_without_reinterpreting_user_data() {
    let values = [
        ("path", "/tmp/{error}<notes>&.txt".into()),
        ("error", "read failed".into()),
    ];
    assert_eq!(
        interpolate("{error}: {path}", &values),
        "read failed: /tmp/{error}<notes>&.txt"
    );
    assert_eq!(
        interpolate("{path} / {path}", &values),
        "/tmp/{error}<notes>&.txt / /tmp/{error}<notes>&.txt"
    );
    assert_eq!(interpolate("a {unknown} b", &values), "a {unknown} b");
    assert_eq!(interpolate("a {unfinished", &values), "a {unfinished");
    assert_eq!(
        tr("Unknown untranslated diagnostic"),
        "Unknown untranslated diagnostic"
    );
}

#[test]
fn language_preference_is_private_persistent_and_resettable() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("open-data-recovery/language");
    assert_eq!(read_preference(Some(&path)), "system");
    write_preference(&path, "fr").unwrap();
    assert_eq!(read_preference(Some(&path)), "fr");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    write_preference(&path, "ar").unwrap();
    assert_eq!(read_preference(Some(&path)), "ar");
    assert!(write_preference(&path, "invalid").is_err());
    assert_eq!(read_preference(Some(&path)), "ar");
    write_preference(&path, "system").unwrap();
    assert_eq!(read_preference(Some(&path)), "system");
    std::fs::write(&path, "unknown-language").unwrap();
    assert_eq!(read_preference(Some(&path)), "system");
}
