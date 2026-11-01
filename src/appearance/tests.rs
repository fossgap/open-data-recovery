use super::*;

#[test]
fn missing_unreadable_or_unknown_theme_preferences_use_system_gtk() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("theme");
    assert_eq!(Theme::read(None), Theme::System);
    assert_eq!(Theme::read(Some(&path)), Theme::System);
    assert_eq!(Theme::read(Some(temp.path())), Theme::System);
    for value in ["", "unknown", "CUSTOM", "custom\nsystem"] {
        std::fs::write(&path, value).unwrap();
        assert_eq!(Theme::read(Some(&path)), Theme::System);
    }
    std::fs::write(&path, [0xff]).unwrap();
    assert_eq!(Theme::read(Some(&path)), Theme::System);
}

#[test]
fn theme_preference_is_private_persistent_and_resettable() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("open-data-recovery/theme");
    for theme in [Theme::Custom, Theme::System, Theme::Custom] {
        crate::preferences::write(&path, theme.name()).unwrap();
        assert_eq!(Theme::read(Some(&path)), theme);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn failed_preference_write_preserves_previous_choice_and_never_follows_temporary_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("theme");
    crate::preferences::write(&path, "custom").unwrap();
    let target = temp.path().join("unrelated");
    std::fs::write(&target, "unchanged").unwrap();
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::os::unix::fs::symlink(&target, &temporary).unwrap();
    assert!(crate::preferences::write(&path, "system").is_err());
    assert_eq!(Theme::read(Some(&path)), Theme::Custom);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "unchanged");
}
