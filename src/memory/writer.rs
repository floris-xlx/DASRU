use crate::identity::BuildResolution;

pub(crate) struct ExperimentalWritePermit(());

impl ExperimentalWritePermit {
    pub(crate) fn authorize(
        experimental_opt_in: bool,
        build: &BuildResolution,
    ) -> Result<Self, &'static str> {
        if !experimental_opt_in {
            return Err("experimental memory writing was not explicitly enabled");
        }
        if !matches!(build, BuildResolution::ExactMatch { .. }) {
            return Err("memory writing requires an exact registered executable build");
        }
        Ok(Self(()))
    }
}

#[cfg(windows)]
pub(crate) mod windows;

#[cfg(test)]
mod tests {
    use crate::identity::{BuildRecord, BuildResolution};

    use super::ExperimentalWritePermit;

    #[test]
    fn experimental_write_requires_opt_in_and_a_registered_build() {
        let build = BuildResolution::ExactMatch {
            build: BuildRecord {
                id: "fixture".to_owned(),
                name: "Synthetic fixture".to_owned(),
                sha256: "a".repeat(64),
                file_version: None,
                product_version: None,
                steam_build_id: None,
            },
        };

        assert!(ExperimentalWritePermit::authorize(false, &build).is_err());
        assert!(ExperimentalWritePermit::authorize(true, &BuildResolution::UnknownBuild).is_err());
        assert!(ExperimentalWritePermit::authorize(true, &build).is_ok());
    }
}
