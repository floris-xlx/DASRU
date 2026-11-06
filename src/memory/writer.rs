use crate::{
    capability::{CapabilityDecision, FeatureStatus},
    identity::BuildResolution,
};

pub(crate) struct ExperimentalWritePermit(());

impl ExperimentalWritePermit {
    pub(crate) fn authorize(
        experimental_opt_in: bool,
        build: &BuildResolution,
        capability: CapabilityDecision,
    ) -> Result<Self, &'static str> {
        if !experimental_opt_in {
            return Err("experimental memory writing was not explicitly enabled");
        }
        if capability.status != FeatureStatus::Experimental || !capability.is_authorized() {
            return Err("capability prerequisites for experimental memory writing were not met");
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
    use crate::capability::{
        self, Architecture, BuildEvidence, CapabilityEvidence, FeatureId, LayoutEvidence,
        MemoryEvidence, Platform, ProcessEvidence,
    };
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
        let capability = capability::evaluate(
            FeatureId::ExperimentalFpsMarker,
            &CapabilityEvidence {
                platform: Platform::Windows,
                architecture: Architecture::X86_64,
                process: ProcessEvidence::Verified,
                build: BuildEvidence::ExactMatch,
                memory: MemoryEvidence::Readable,
                layout: LayoutEvidence::Safe,
                experimental_opt_in: true,
                ..CapabilityEvidence::default()
            },
        );

        assert!(ExperimentalWritePermit::authorize(false, &build, capability).is_err());
        assert!(
            ExperimentalWritePermit::authorize(true, &BuildResolution::UnknownBuild, capability,)
                .is_err()
        );
        assert!(ExperimentalWritePermit::authorize(true, &build, capability).is_ok());
    }
}
