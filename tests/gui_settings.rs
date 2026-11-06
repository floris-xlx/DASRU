#![cfg(feature = "desktop-ui")]
use dasru::gui::settings::{Preferences, SettingsStore};
use std::path::PathBuf;
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
