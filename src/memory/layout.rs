use super::FPS_SIGNATURE;

const STRING_STARTS: [(&str, usize); 3] = [
    ("fps_label", 0),
    ("frametime_format", 8),
    ("frame_label", 16),
];
const MAX_STRING_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StringBoundary {
    pub(super) name: &'static str,
    pub(super) start_offset: usize,
    pub(super) terminator_offset: usize,
    pub(super) capacity_bytes: usize,
    pub(super) capacity_basis: &'static str,
    pub(super) original_value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PatchDiff {
    pub(super) offset: usize,
    pub(super) before: u8,
    pub(super) after: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PatchPlan {
    pub(super) patched: Vec<u8>,
    pub(super) diff: Vec<PatchDiff>,
    pub(super) write_plan: Vec<PatchDiff>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PatchError {
    InvalidBoundary,
    ContainsNul,
    DoesNotFit,
}

pub(super) fn parse_boundaries(
    bytes: &[u8],
    signature_index: usize,
) -> Result<Vec<StringBoundary>, &'static str> {
    let signature_end = signature_index
        .checked_add(FPS_SIGNATURE.len())
        .ok_or("signature range overflows")?;
    if bytes.get(signature_index..signature_end) != Some(FPS_SIGNATURE) {
        return Err("original bytes no longer contain the expected FPS signature");
    }

    STRING_STARTS
        .iter()
        .enumerate()
        .map(|(index, (name, offset))| {
            let start = signature_index
                .checked_add(*offset)
                .ok_or("string start overflows")?;
            if start >= bytes.len() {
                return Err("string start is outside the readable context");
            }
            let (search_end, capacity_basis) = match STRING_STARTS.get(index + 1) {
                Some((_, next_offset)) => (
                    signature_index + next_offset,
                    "bounded_by_next_string_start",
                ),
                None => (
                    bytes.len().min(start.saturating_add(MAX_STRING_BYTES)),
                    "bounded_by_observed_terminator",
                ),
            };
            let search_end = search_end.min(bytes.len());
            let terminator = bytes[start..search_end]
                .iter()
                .position(|byte| *byte == 0)
                .map(|offset| start + offset)
                .ok_or("string terminator was not found within its safe boundary")?;
            let capacity_bytes = match STRING_STARTS.get(index + 1) {
                Some((_, next_offset)) => next_offset - offset,
                None => terminator - start + 1,
            };
            if terminator >= start + capacity_bytes {
                return Err("string terminator lies beyond the available slot");
            }
            Ok(StringBoundary {
                name,
                start_offset: *offset,
                terminator_offset: terminator - signature_index,
                capacity_bytes,
                capacity_basis,
                original_value: String::from_utf8_lossy(&bytes[start..terminator]).into_owned(),
            })
        })
        .collect()
}

pub(super) fn plan_patch(
    original: &[u8],
    start: usize,
    capacity: usize,
    replacement: &[u8],
) -> Result<PatchPlan, PatchError> {
    let end = start
        .checked_add(capacity)
        .ok_or(PatchError::InvalidBoundary)?;
    if capacity == 0 || end > original.len() {
        return Err(PatchError::InvalidBoundary);
    }
    if replacement.contains(&0) {
        return Err(PatchError::ContainsNul);
    }
    if replacement.len() >= capacity {
        return Err(PatchError::DoesNotFit);
    }

    let mut patched = original.to_vec();
    patched[start..end].fill(0);
    patched[start..start + replacement.len()].copy_from_slice(replacement);
    let diff: Vec<_> = original[start..end]
        .iter()
        .zip(&patched[start..end])
        .enumerate()
        .filter_map(|(offset, (before, after))| {
            (before != after).then_some(PatchDiff {
                offset: start + offset,
                before: *before,
                after: *after,
            })
        })
        .collect();
    let write_plan = if let (Some(first), Some(last)) = (diff.first(), diff.last()) {
        let terminator = start + replacement.len();
        let write_start = first.offset.min(terminator);
        let write_end = (last.offset + 1).max(terminator + 1);
        original[write_start..write_end]
            .iter()
            .zip(&patched[write_start..write_end])
            .enumerate()
            .map(|(offset, (before, after))| PatchDiff {
                offset: write_start + offset,
                before: *before,
                after: *after,
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(PatchPlan {
        patched,
        diff,
        write_plan,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{FPS_MARKER_REPLACEMENT, FPS_SIGNATURE};
    use super::{PatchError, parse_boundaries, plan_patch};

    #[test]
    fn reports_fps_string_terminators_and_conservative_capacities() {
        let bytes = b"%i FPS\0\0%.2fms\0\0Frame : %u\0next";
        let strings = parse_boundaries(bytes, 0).unwrap();

        assert_eq!(strings.len(), 3);
        assert_eq!(strings[0].start_offset, 0);
        assert_eq!(strings[0].terminator_offset, 6);
        assert_eq!(strings[0].capacity_bytes, 8);
        assert_eq!(strings[1].start_offset, 8);
        assert_eq!(strings[1].terminator_offset, 14);
        assert_eq!(strings[1].capacity_bytes, 8);
        assert_eq!(strings[2].start_offset, 16);
        assert_eq!(strings[2].terminator_offset, 26);
        assert_eq!(strings[2].capacity_bytes, 11);
    }

    #[test]
    fn dry_run_patch_changes_only_its_bounded_slot() {
        let original = b"%i FPS\0\0%.2fms\0\0Frame : %u\0";
        let plan = plan_patch(original, 0, 8, b"FPS").unwrap();

        assert_eq!(&plan.patched[..8], b"FPS\0\0\0\0\0");
        assert_eq!(&plan.patched[8..], &original[8..]);
        assert!(plan.diff.iter().all(|change| change.offset < 8));
    }

    #[test]
    fn default_fps_marker_changes_only_the_seventh_payload_byte() {
        let original = b"%i FPS\0\0%.2fms\0\0Frame : %u\0";
        let plan = plan_patch(original, 0, 8, FPS_MARKER_REPLACEMENT.as_bytes()).unwrap();

        assert_eq!(&plan.patched[..8], b"%i FPS*\0");
        assert_eq!(&plan.patched[8..], &original[8..]);
        assert_eq!(plan.diff.len(), 1);
        assert_eq!(plan.diff[0].offset, 6);
        assert_eq!(plan.diff[0].before, 0);
        assert_eq!(plan.diff[0].after, b'*');
        assert_eq!(plan.write_plan.len(), 2);
        assert_eq!(plan.write_plan[0].offset, 6);
        assert_eq!(plan.write_plan[1].offset, 7);
        assert_eq!(plan.write_plan[1].before, 0);
        assert_eq!(plan.write_plan[1].after, 0);
    }

    #[test]
    fn plus_replacement_has_two_byte_patch_span_and_one_changed_byte() {
        let original = b"%i FPS\0\0%.2fms\0\0Frame : %u\0";
        let plan = plan_patch(original, 0, 8, b"%i FPS+").unwrap();

        assert_eq!(plan.diff.len(), 1);
        assert_eq!(plan.diff[0].offset, 6);
        assert_eq!(plan.write_plan.len(), 2);
        assert_eq!(plan.write_plan[1].offset, 7);
        assert_eq!(plan.write_plan[1].before, 0);
        assert_eq!(plan.write_plan[1].after, 0);
        assert_eq!(&plan.patched[8..], &original[8..]);
    }

    #[test]
    fn expanded_fps_label_is_unsupported_before_adjacent_row_bytes_change() {
        let original = FPS_SIGNATURE;
        let result = plan_patch(original, 0, 8, b"%i FPS (DASRU)");

        assert!(matches!(result, Err(PatchError::DoesNotFit)));
    }

    #[test]
    fn missing_terminator_does_not_expand_the_last_string_capacity() {
        let bytes = b"%i FPS\0\0%.2fms\0\0Frame : %u";

        assert!(parse_boundaries(bytes, 0).is_err());
    }
}
