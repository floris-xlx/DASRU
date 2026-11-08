use std::{
    fs, io,
    path::{Path, PathBuf},
};
use thiserror::Error;

const DOOM_THE_DARK_AGES_APP_ID: &str = "3017860";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SteamInstallMetadata {
    pub install_dir: String,
    pub build_id: String,
}

#[derive(Debug, Error)]
pub(crate) enum SteamReadError {
    #[error("{0}")]
    Manifest(String),
    #[error("Steam appmanifest installdir does not contain the inspected executable")]
    InstallationMismatch { metadata: SteamInstallMetadata },
}

pub(crate) fn read_install_metadata(
    executable_path: &Path,
) -> Result<Option<SteamInstallMetadata>, SteamReadError> {
    let Some((common, manifest_path)) = steam_paths(executable_path) else {
        return Ok(None);
    };
    let source = match fs::read_to_string(&manifest_path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(SteamReadError::Manifest(format!(
                "could not read Steam appmanifest: {error}"
            )));
        }
    };
    let metadata = parse_appmanifest(&source).map_err(SteamReadError::Manifest)?;
    let install_root = common.join(&metadata.install_dir);
    let executable_path = fs::canonicalize(executable_path).map_err(|error| {
        SteamReadError::Manifest(format!("could not resolve executable path: {error}"))
    })?;
    let install_root = match fs::canonicalize(install_root) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(SteamReadError::InstallationMismatch { metadata });
        }
        Err(error) => {
            return Err(SteamReadError::Manifest(format!(
                "could not resolve Steam installdir: {error}"
            )));
        }
    };
    if !path_is_within(&executable_path, &install_root) {
        return Err(SteamReadError::InstallationMismatch { metadata });
    }
    Ok(Some(metadata))
}

fn steam_paths(executable_path: &Path) -> Option<(PathBuf, PathBuf)> {
    executable_path.ancestors().find_map(|common| {
        if !common
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("common"))
        {
            return None;
        }
        let steamapps = common.parent()?;
        steamapps
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("steamapps"))
            .then(|| {
                (
                    common.to_owned(),
                    steamapps.join(format!("appmanifest_{DOOM_THE_DARK_AGES_APP_ID}.acf")),
                )
            })
    })
}

#[cfg(windows)]
fn path_is_within(path: &Path, root: &Path) -> bool {
    let normalize = |path: &Path| {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    let path = normalize(path);
    let mut root = normalize(root);
    root.push('\\');
    path.starts_with(&root)
}

#[cfg(not(windows))]
fn path_is_within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

fn parse_appmanifest(source: &str) -> Result<SteamInstallMetadata, String> {
    let tokens = tokenize(source)?;
    if tokens.first() != Some(&Token::Text("AppState".to_owned()))
        || tokens.get(1) != Some(&Token::Open)
    {
        return Err("Steam appmanifest has no AppState object".to_owned());
    }

    let mut app_id = None;
    let mut install_dir = None;
    let mut build_id = None;
    let mut index = 2;
    let mut closed = false;
    while index < tokens.len() {
        match &tokens[index] {
            Token::Close => {
                index += 1;
                closed = true;
                break;
            }
            Token::Text(key) => {
                index += 1;
                match tokens.get(index) {
                    Some(Token::Text(value)) => {
                        let target = if key.eq_ignore_ascii_case("appid") {
                            Some(&mut app_id)
                        } else if key.eq_ignore_ascii_case("installdir") {
                            Some(&mut install_dir)
                        } else if key.eq_ignore_ascii_case("buildid") {
                            Some(&mut build_id)
                        } else {
                            None
                        };
                        if let Some(target) = target
                            && target.replace(value.as_str()).is_some()
                        {
                            return Err(format!("Steam appmanifest repeats {key}"));
                        }
                        index += 1;
                    }
                    Some(Token::Open) => {
                        index = skip_object(&tokens, index)?;
                    }
                    _ => return Err(format!("Steam appmanifest has an invalid {key} value")),
                }
            }
            _ => return Err("Steam appmanifest has invalid top-level structure".to_owned()),
        }
    }
    if !closed {
        return Err("Steam appmanifest has an unterminated AppState object".to_owned());
    }
    if index != tokens.len() {
        return Err("Steam appmanifest has trailing data".to_owned());
    }
    if app_id != Some(DOOM_THE_DARK_AGES_APP_ID) {
        return Err(format!(
            "Steam appmanifest app ID does not match {DOOM_THE_DARK_AGES_APP_ID}"
        ));
    }
    let install_dir =
        install_dir.ok_or_else(|| "Steam appmanifest has no installdir".to_owned())?;
    if install_dir.is_empty()
        || install_dir == "."
        || install_dir == ".."
        || install_dir.contains(['/', '\\', ':', '\0'])
    {
        return Err("Steam appmanifest installdir is not a directory name".to_owned());
    }
    let build_id = build_id.ok_or_else(|| "Steam appmanifest has no build ID".to_owned())?;
    if build_id.is_empty() || !build_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Steam appmanifest build ID is not a decimal integer".to_owned());
    }
    if !build_id.parse::<u64>().is_ok_and(|parsed| parsed > 0) {
        return Err("Steam appmanifest build ID is out of range".to_owned());
    }
    Ok(SteamInstallMetadata {
        install_dir: install_dir.to_owned(),
        build_id: build_id.to_owned(),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Text(String),
    Open,
    Close,
}

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let mut chars = source
        .strip_prefix('\u{feff}')
        .unwrap_or(source)
        .chars()
        .peekable();
    let mut tokens = Vec::new();
    while let Some(character) = chars.next() {
        match character {
            character if character.is_whitespace() => {}
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                for character in chars.by_ref() {
                    if character == '\n' {
                        break;
                    }
                }
            }
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '"' => {
                let mut value = String::new();
                let mut closed = false;
                while let Some(character) = chars.next() {
                    match character {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' => match chars.next() {
                            Some(escaped @ ('\\' | '"')) => value.push(escaped),
                            Some(other) => {
                                value.push('\\');
                                value.push(other);
                            }
                            None => return Err("Steam appmanifest ends in an escape".to_owned()),
                        },
                        other => value.push(other),
                    }
                }
                if !closed {
                    return Err("Steam appmanifest has an unterminated string".to_owned());
                }
                tokens.push(Token::Text(value));
            }
            first => {
                let mut value = String::from(first);
                while chars.peek().is_some_and(|character| {
                    !character.is_whitespace() && !matches!(character, '{' | '}')
                }) {
                    value.push(chars.next().unwrap());
                }
                tokens.push(Token::Text(value));
            }
        }
    }
    Ok(tokens)
}

fn skip_object(tokens: &[Token], open_index: usize) -> Result<usize, String> {
    let mut depth = 1usize;
    let mut index = open_index + 1;
    while let Some(token) = tokens.get(index) {
        match token {
            Token::Open => depth += 1,
            Token::Close => depth -= 1,
            Token::Text(_) => {}
        }
        index += 1;
        if depth == 0 {
            return Ok(index);
        }
    }
    Err("Steam appmanifest has an unterminated object".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
        "AppState"
        {
            "appid" "3017860"
            "installdir" "DOOMTheDarkAges"
            "buildid" "25755602"
            "UserConfig" { "buildid" "99999999" }
        }
    "#;

    #[test]
    fn parses_the_top_level_build_id_for_doom_the_dark_ages() {
        assert_eq!(
            parse_appmanifest(MANIFEST).unwrap(),
            SteamInstallMetadata {
                install_dir: "DOOMTheDarkAges".to_owned(),
                build_id: "25755602".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_mismatched_or_malformed_appmanifests() {
        assert!(parse_appmanifest(&MANIFEST.replace("3017860", "1")).is_err());
        assert!(parse_appmanifest(&MANIFEST.replace("25755602", "not-a-build")).is_err());
        assert!(parse_appmanifest(&MANIFEST.replace("\"buildid\" \"25755602\"", "")).is_err());
        assert!(parse_appmanifest(&format!("\u{feff}{MANIFEST}")).is_ok());
        assert!(parse_appmanifest(&MANIFEST.replace("DOOMTheDarkAges", "../other")).is_err());
        assert!(
            parse_appmanifest(&MANIFEST.replace(
                "\"buildid\" \"25755602\"",
                "\"buildid\" \"1\" \"buildid\" \"2\""
            ))
            .is_err()
        );
    }

    #[test]
    fn locates_the_manifest_in_the_steam_library_containing_the_executable() {
        let root = std::env::temp_dir().join(format!(
            "dasru-steam-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let install = root.join("steamapps/common/DOOMTheDarkAges");
        let manifest = root.join("steamapps/appmanifest_3017860.acf");
        std::fs::create_dir_all(&install).unwrap();
        let executable = install.join("DOOMTheDarkAges.exe");
        std::fs::write(&executable, b"simulated executable").unwrap();
        std::fs::write(&manifest, MANIFEST).unwrap();

        assert_eq!(
            read_install_metadata(&executable).unwrap(),
            Some(SteamInstallMetadata {
                install_dir: "DOOMTheDarkAges".to_owned(),
                build_id: "25755602".to_owned(),
            })
        );

        let wrong_manifest = MANIFEST.replace("DOOMTheDarkAges", "OtherGame");
        std::fs::write(&manifest, wrong_manifest).unwrap();
        assert!(matches!(
            read_install_metadata(&executable),
            Err(SteamReadError::InstallationMismatch { .. })
        ));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_no_build_id_for_non_steam_install_paths() {
        assert_eq!(
            read_install_metadata(Path::new("C:/Games/DOOMTheDarkAges/DOOMTheDarkAges.exe"))
                .unwrap(),
            None
        );
    }
}
