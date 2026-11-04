use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::ExecutableIdentity;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BuildRecord {
    pub id: String,
    pub name: String,
    pub sha256: String,
    #[serde(default)]
    pub file_version: Option<String>,
    #[serde(default)]
    pub product_version: Option<String>,
    #[serde(default)]
    pub steam_build_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BuildResolution {
    ExactMatch { build: BuildRecord },
    UnknownBuild,
    InstallMismatch { expected_build_id: String },
    IdentityInconsistent { reason: String },
}

#[derive(Clone, Debug)]
pub struct BuildRegistry {
    builds: Vec<BuildRecord>,
}

#[derive(Deserialize)]
struct RawBuildRegistry {
    builds: Vec<BuildRecord>,
}

#[derive(Debug, Error)]
pub enum BuildRegistryError {
    #[error("invalid build registry TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("build {build_id} has an invalid SHA-256 hash")]
    InvalidHash { build_id: String },
    #[error("build registry contains duplicate build id {0}")]
    DuplicateId(String),
    #[error("build registry contains duplicate SHA-256 hash {0}")]
    DuplicateHash(String),
}

impl BuildRegistry {
    pub fn from_toml(source: &str) -> Result<Self, BuildRegistryError> {
        let RawBuildRegistry { mut builds } = toml::from_str(source)?;
        let mut ids = HashSet::new();
        let mut hashes = HashSet::new();

        for build in &mut builds {
            if !valid_sha256(&build.sha256) {
                return Err(BuildRegistryError::InvalidHash {
                    build_id: build.id.clone(),
                });
            }
            build.sha256.make_ascii_lowercase();
            if !ids.insert(build.id.clone()) {
                return Err(BuildRegistryError::DuplicateId(build.id.clone()));
            }
            if !hashes.insert(build.sha256.clone()) {
                return Err(BuildRegistryError::DuplicateHash(build.sha256.clone()));
            }
        }

        Ok(Self { builds })
    }

    pub fn resolve(
        &self,
        identity: &ExecutableIdentity,
        observed_steam_build_id: Option<&str>,
    ) -> BuildResolution {
        let matched_build = self
            .builds
            .iter()
            .find(|build| build.sha256.eq_ignore_ascii_case(&identity.sha256));

        if let Some(build) = matched_build {
            for (field, observed, registered) in [
                (
                    "file_version",
                    identity.file_version.as_deref(),
                    build.file_version.as_deref(),
                ),
                (
                    "product_version",
                    identity.product_version.as_deref(),
                    build.product_version.as_deref(),
                ),
            ] {
                if let (Some(observed), Some(registered)) = (observed, registered) {
                    if observed != registered {
                        return BuildResolution::IdentityInconsistent {
                            reason: format!(
                                "registered build metadata conflicts with observed {field} {observed}; registry says {registered}"
                            ),
                        };
                    }
                }
            }

            if let (Some(observed), Some(expected)) =
                (observed_steam_build_id, build.steam_build_id.as_deref())
            {
                if observed != expected {
                    return BuildResolution::InstallMismatch {
                        expected_build_id: expected.to_owned(),
                    };
                }
            }

            return BuildResolution::ExactMatch {
                build: build.clone(),
            };
        }

        BuildResolution::UnknownBuild
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
