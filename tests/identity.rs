use std::path::PathBuf;

use dasru::{
    identity::{BuildRegistry, BuildResolution, ExecutableIdentity},
    process::is_game_executable_name,
};

const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn fixture_registry() -> BuildRegistry {
    BuildRegistry::from_toml(
        r#"
[[builds]]
id = "tda-1"
name = "Known build"
sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
file_version = "1.2.3.4"
product_version = "1.2.3.4"
steam_build_id = "12345"
"#,
    )
    .expect("fixture registry parses")
}

fn identity(sha256: &str, file_version: Option<&str>) -> ExecutableIdentity {
    ExecutableIdentity {
        path: PathBuf::from("DOOMTheDarkAges.exe"),
        sha256: sha256.to_owned(),
        size: 3,
        file_version: file_version.map(str::to_owned),
        product_version: Some("1.2.3.4".to_owned()),
    }
}

#[test]
fn hashes_the_entire_executable_file() {
    let path = std::env::temp_dir().join(format!("dasru-hash-{}.bin", std::process::id()));
    std::fs::write(&path, b"abc").expect("fixture is written");

    let observed = ExecutableIdentity::inspect(&path).expect("identity is read");

    assert_eq!(observed.sha256, HASH);
    assert_eq!(observed.size, 3);
    let _ = std::fs::remove_file(path);
}

#[test]
fn recognizes_a_registered_build_only_by_sha256() {
    let result = fixture_registry().resolve(&identity(HASH, Some("1.2.3.4")), Some("12345"));

    assert!(matches!(result, BuildResolution::ExactMatch { .. }));
}

#[test]
fn reports_unknown_hash_even_when_steam_metadata_is_absent() {
    let result = fixture_registry().resolve(&identity(&"0".repeat(64), None), None);

    assert_eq!(result, BuildResolution::UnknownBuild);
}

#[test]
fn reports_install_metadata_conflicting_with_the_executable_hash() {
    let result = fixture_registry().resolve(&identity(&"0".repeat(64), None), Some("12345"));

    assert_eq!(
        result,
        BuildResolution::InstallMismatch {
            expected_build_id: "12345".to_owned()
        }
    );
}

#[test]
fn rejects_conflicting_version_metadata_for_a_registered_hash() {
    let result = fixture_registry().resolve(&identity(HASH, Some("9.9.9.9")), None);

    assert!(matches!(
        result,
        BuildResolution::IdentityInconsistent { .. }
    ));
}

#[test]
fn rejects_duplicate_registered_hashes() {
    let source = format!(
        "[[builds]]\nid = 'one'\nname = 'One'\nsha256 = '{HASH}'\n\
         [[builds]]\nid = 'two'\nname = 'Two'\nsha256 = '{HASH}'\n"
    );

    assert!(BuildRegistry::from_toml(&source).is_err());
}

#[test]
fn rejects_malformed_registered_hashes() {
    let source = "[[builds]]\nid = 'bad'\nname = 'Bad'\nsha256 = '1234'\n";

    assert!(BuildRegistry::from_toml(source).is_err());
}

#[test]
fn process_name_is_only_an_exact_case_insensitive_candidate_filter() {
    assert!(is_game_executable_name("DOOMTheDarkAges.exe"));
    assert!(is_game_executable_name("doomthedarkages.EXE"));
    assert!(!is_game_executable_name("DOOMTheDarkAges.exe.bak"));
    assert!(!is_game_executable_name("other.exe"));
}
