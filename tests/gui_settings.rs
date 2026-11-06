#![cfg(feature = "desktop-ui")]
use dasru::gui::settings::{Preferences, SettingsStore};
use std::path::PathBuf;
#[test]
fn compact_window_defaults_replace_dashboard_geometry() {
    let p = Preferences::default();
    assert_eq!(p.size, [720.0, 700.0]);
    assert!(p.validate().is_ok());
}
#[test]
fn legacy_wide_window_migrates_without_losing_binding_or_overwriting_on_load() {
    let path = path("legacy-geometry");
    let legacy = Preferences {
        size: [1000.0, 680.0],
        binding: "Ctrl+Shift+F10".into(),
        dark: false,
        ..Default::default()
    };
    let bytes = serde_json::to_vec(&legacy).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let (mut store, p, error) = SettingsStore::load(path.clone());
    assert!(error.is_none());
    assert_eq!(p.size, [720.0, 680.0]);
    assert_eq!(p.binding, legacy.binding);
    assert!(!p.dark);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    store.save(&p).unwrap();
    std::fs::remove_file(path).unwrap();
}
fn path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "dasru-gui-settings-{}-{name}.json",
        std::process::id()
    ))
}
#[test]
fn missing_settings_default_valid_round_trip_and_corrupt_is_preserved() {
    let path = path("roundtrip");
    let _ = std::fs::remove_file(&path);
    let (mut store, preferences, error) = SettingsStore::load(path.clone());
    assert!(error.is_none());
    assert_eq!(preferences, Preferences::default());
    store.save(&preferences).unwrap();
    let (_, loaded, error) = SettingsStore::load(path.clone());
    assert!(error.is_none());
    assert_eq!(loaded, preferences);
    std::fs::write(&path, b"corrupt").unwrap();
    let (mut store, _, error) = SettingsStore::load(path.clone());
    assert!(error.is_some());
    assert!(store.save(&preferences).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt");
    std::fs::remove_file(path).unwrap();
}
#[test]
fn invalid_schema_binding_and_interval_are_rejected() {
    let mut p = Preferences {
        schema_version: 2,
        ..Default::default()
    };
    assert!(p.validate().is_err());
    p.schema_version = 1;
    p.binding = "F9".into();
    assert!(p.validate().is_err());
    p.binding = "Ctrl+Alt+F9".into();
    p.refresh_seconds = 0;
    assert!(p.validate().is_err());
    p.refresh_seconds = 1;
    p.text_scale = f32::NAN;
    assert!(p.validate().is_err());
}

#[test]
fn unchanged_preferences_avoid_republication_without_skipping_conflict_checks() {
    let path = path("unchanged");
    let _ = std::fs::remove_file(&path);
    let (mut store, preferences, _) = SettingsStore::load(path.clone());
    store.save(&preferences).unwrap();
    let original_time =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(946684800);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_time)
        .unwrap();
    store.save(&preferences).unwrap();
    let after = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, b"external change").unwrap();
    let conflict = store.save(&preferences);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        after, original_time,
        "unchanged preferences were unnecessarily republished"
    );
    assert_eq!(conflict, Err("SETTINGS_CHANGED_EXTERNALLY"));
    assert_eq!(bytes, b"external change");
}
#[test]
fn geometry_is_clamped_to_visible_monitor() {
    let mut p = Preferences {
        position: Some([-9000.0, 9000.0]),
        ..Default::default()
    };
    p.clamp_geometry([0.0, 0.0], [1920.0, 1080.0]);
    let [x, y] = p.position.unwrap();
    assert!(x >= 0.0 && y >= 0.0);
    assert!(x + p.size[0] <= 1920.0 && y + p.size[1] <= 1080.0);
}

#[test]
fn native_geometry_keeps_decorations_inside_the_work_area() {
    let mut preferences = Preferences {
        size: [720.0, 1200.0],
        position: Some([5000.0, 1400.0]),
        ..Default::default()
    };
    preferences.clamp_geometry_with_frame([0.0, 0.0], [5120.0, 1392.0], [16.0, 39.0]);
    let [x, y] = preferences.position.unwrap();
    assert!(x >= 0.0 && y >= 0.0);
    assert!(x + preferences.size[0] + 16.0 <= 5120.0);
    assert!(y + preferences.size[1] + 39.0 <= 1392.0);
    assert_eq!(preferences.size, [720.0, 1200.0]);
    preferences.validate().unwrap();
}

#[test]
fn native_geometry_rejects_invalid_frame_evidence() {
    for frame in [[-1.0, 39.0], [16.0, f32::NAN]] {
        let mut preferences = Preferences {
            position: Some([9000.0, 9000.0]),
            ..Default::default()
        };
        preferences.clamp_geometry_with_frame([0.0, 0.0], [1920.0, 1080.0], frame);
        assert_eq!(preferences.position, None);
    }
}

#[test]
fn background_writer_flushes_final_preferences_on_close() {
    let path = path("writer");
    let _ = std::fs::remove_file(&path);
    let (store, mut preferences, _) = SettingsStore::load(path.clone());
    let mut writer = dasru::gui::settings::SettingsWriter::start(store).unwrap();
    writer.save(preferences.clone()).unwrap();
    preferences.dark = false;
    writer.finish(preferences.clone()).unwrap();
    let (_, loaded, error) = SettingsStore::load(path.clone());
    assert!(error.is_none());
    assert_eq!(loaded, preferences);
    std::fs::remove_file(path).unwrap();
}
