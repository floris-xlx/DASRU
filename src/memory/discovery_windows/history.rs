use super::objects::bounded_region_length;
use super::*;

const ROW_HISTORY_HEADER_SIZE: usize = 16;

#[derive(Clone, Copy)]
struct RemoteSpanSegment {
    address: u64,
    length: u64,
}

struct RowHistoryRead {
    header: Option<RowHistoryHeader>,
    status: &'static str,
    data_allocation_base: Option<u64>,
    capacity_bytes: Option<u64>,
    elements: Vec<Vec<u8>>,
    error: Option<ReportError>,
}

pub(super) fn inspect_row_history(
    handle: HANDLE,
    slot_address: u64,
    row_address: u64,
    slot_allocation_base: u64,
    row_allocation_base: u64,
) -> MetricsRowHistory {
    let read = read_row_history_state(
        handle,
        slot_address,
        row_address,
        slot_allocation_base,
        row_allocation_base,
    );
    let data_address = read.header.map(|header| header.data_address);
    let count = read.header.map(|header| header.count);
    let capacity = read.header.map(|header| header.capacity);
    let elements = read
        .elements
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            let offset = (index as u64) * u64::from(ROW_HISTORY_ELEMENT_STRIDE);
            MetricsRowHistoryElement {
                index: index as u32,
                address: data_address
                    .and_then(|base| base.checked_add(offset))
                    .expect("validated history span contains every reported element"),
                bytes_hex: hex(bytes),
            }
        })
        .collect::<Vec<_>>();
    let observed_at_unix_ms = now_unix_ms();
    let sample = MetricsRowHistorySample {
        observed_at_unix_ms,
        status: read.status,
        data_address,
        count,
        capacity,
        capacity_bytes: read.capacity_bytes,
        data_allocation_base: read.data_allocation_base,
        data_pointer_changed: None,
        count_changed: None,
        capacity_changed: None,
        changed_elements: Vec::new(),
        removed_element_indices: Vec::new(),
        read_error: read.error.clone(),
    };
    let inspected_element_count = read.elements.len() as u32;
    MetricsRowHistory {
        row_address,
        status: read.status,
        data_address,
        count,
        capacity,
        data_allocation_base: read.data_allocation_base,
        capacity_bytes: read.capacity_bytes,
        allocation_validation: "COMMITTED_READABLE_VIRTUAL_ALLOCATION; HEAP_BLOCK_BOUNDARY_UNOBSERVED",
        element_stride_bytes: ROW_HISTORY_ELEMENT_STRIDE,
        interpretation: "RAW_BYTES_AND_ALIGNED_4_BYTE_VIEWS; FIELD_SEMANTICS_UNCONFIRMED",
        inspected_element_count,
        omitted_element_count: count
            .map_or(0, |count| count.saturating_sub(inspected_element_count)),
        elements,
        samples: vec![sample],
        last_header: read.header,
        last_elements: read.elements,
    }
}

fn read_row_history_state(
    handle: HANDLE,
    slot_address: u64,
    row_address: u64,
    slot_allocation_base: u64,
    row_allocation_base: u64,
) -> RowHistoryRead {
    let Some(slot_region) = query_region(handle, slot_address) else {
        return history_read_error(
            None,
            "ROW_SLOT_UNMAPPED",
            "HISTORY_ROW_SLOT_UNMAPPED",
            format!("row pointer slot 0x{slot_address:X} is not mapped"),
        );
    };
    if slot_region.state != MEM_COMMIT
        || !readable_protection(slot_region.protection)
        || slot_region.allocation_base != slot_allocation_base
    {
        return history_read_error(
            None,
            "ROW_SLOT_REGION_CHANGED",
            "HISTORY_ROW_SLOT_REGION_CHANGED",
            format!("row pointer slot 0x{slot_address:X} changed region"),
        );
    }
    let mut row_pointer_bytes = [0u8; size_of::<u64>()];
    if !read_exact(handle, slot_address, &mut row_pointer_bytes)
        || u64::from_le_bytes(row_pointer_bytes) != row_address
    {
        return history_read_error(
            None,
            "ROW_POINTER_CHANGED",
            "HISTORY_ROW_POINTER_CHANGED",
            format!("row pointer slot 0x{slot_address:X} no longer targets 0x{row_address:X}"),
        );
    }

    let Some(row_region) = query_region(handle, row_address) else {
        return history_read_error(
            None,
            "ROW_TARGET_UNMAPPED",
            "HISTORY_ROW_TARGET_UNMAPPED",
            format!("row object 0x{row_address:X} is not mapped"),
        );
    };
    if row_region.state != MEM_COMMIT
        || !readable_protection(row_region.protection)
        || row_region.allocation_base != row_allocation_base
    {
        return history_read_error(
            None,
            "ROW_TARGET_REGION_CHANGED",
            "HISTORY_ROW_TARGET_REGION_CHANGED",
            format!("row object 0x{row_address:X} changed region"),
        );
    }
    let Some(header_address) = row_address.checked_add(u64::from(ROW_DATA_POINTER_OFFSET)) else {
        return history_read_error(
            None,
            "ROW_HEADER_ADDRESS_OVERFLOW",
            "HISTORY_ROW_HEADER_ADDRESS_OVERFLOW",
            format!("row object 0x{row_address:X} has an overflowing history-header address"),
        );
    };
    let Some(header_region) = query_region(handle, header_address) else {
        return history_read_error(
            None,
            "ROW_HEADER_UNMAPPED",
            "HISTORY_ROW_HEADER_UNMAPPED",
            format!("history header at 0x{header_address:X} is not mapped"),
        );
    };
    if header_region.state != MEM_COMMIT
        || !readable_protection(header_region.protection)
        || header_region.allocation_base != row_allocation_base
        || bounded_region_length(header_address, header_region, ROW_HISTORY_HEADER_SIZE)
            != ROW_HISTORY_HEADER_SIZE
    {
        return history_read_error(
            None,
            "ROW_HEADER_REGION_INVALID",
            "HISTORY_ROW_HEADER_REGION_INVALID",
            format!(
                "history header at 0x{header_address:X} is outside its readable row allocation"
            ),
        );
    }
    let mut header_bytes = [0u8; ROW_HISTORY_HEADER_SIZE];
    if !read_exact(handle, header_address, &mut header_bytes) {
        return history_read_error(
            None,
            "ROW_HEADER_READ_FAILED",
            "HISTORY_ROW_HEADER_READ_FAILED",
            format!("could not read history header at 0x{header_address:X}"),
        );
    }
    let header = match parse_row_history_header(&header_bytes) {
        Ok(header) => header,
        Err(error) => {
            return history_read_error(
                None,
                "INCOMPLETE_HEADER",
                "HISTORY_HEADER_INCOMPLETE",
                format!("could not decode row history header: {error:?}"),
            );
        }
    };
    let capacity_bytes = match row_history_capacity_bytes(header) {
        Ok(bytes) => bytes,
        Err(error) => {
            let (status, code) = history_header_error_status(error);
            return history_read_error(
                Some(header),
                status,
                code,
                format!("row history header failed validation: {error:?}"),
            );
        }
    };
    if capacity_bytes == 0 {
        return RowHistoryRead {
            header: Some(header),
            status: "EMPTY",
            data_allocation_base: None,
            capacity_bytes: Some(0),
            elements: Vec::new(),
            error: None,
        };
    }
    if !is_aligned_pointer_candidate(header.data_address, MIN_USER_ADDRESS, MAX_USER_ADDRESS) {
        return history_read_error(
            Some(header),
            "INVALID_DATA_POINTER",
            "HISTORY_DATA_POINTER_INVALID",
            format!(
                "history data pointer 0x{:X} is outside the aligned user range",
                header.data_address
            ),
        );
    }
    let (data_allocation_base, segments) =
        match readable_span_segments(handle, header.data_address, capacity_bytes) {
            Ok(span) => span,
            Err(error) => {
                return RowHistoryRead {
                    header: Some(header),
                    status: "ARRAY_ALLOCATION_INVALID",
                    data_allocation_base: None,
                    capacity_bytes: Some(capacity_bytes),
                    elements: Vec::new(),
                    error: Some(error),
                };
            }
        };
    let element_bytes = (header.count as usize) * usize::from(ROW_HISTORY_ELEMENT_STRIDE);
    let bytes = match read_remote_segments(handle, &segments, element_bytes) {
        Ok(bytes) => bytes,
        Err(error) => {
            return RowHistoryRead {
                header: Some(header),
                status: "ARRAY_READ_FAILED",
                data_allocation_base: Some(data_allocation_base),
                capacity_bytes: Some(capacity_bytes),
                elements: Vec::new(),
                error: Some(error),
            };
        }
    };
    let elements = bytes
        .chunks_exact(usize::from(ROW_HISTORY_ELEMENT_STRIDE))
        .map(<[u8]>::to_vec)
        .collect();
    RowHistoryRead {
        header: Some(header),
        status: if header.count == 0 {
            "EMPTY"
        } else {
            "READ_OK"
        },
        data_allocation_base: Some(data_allocation_base),
        capacity_bytes: Some(capacity_bytes),
        elements,
        error: None,
    }
}

fn history_header_error_status(error: RowHistoryHeaderError) -> (&'static str, &'static str) {
    match error {
        RowHistoryHeaderError::IncompleteHeader => {
            ("INCOMPLETE_HEADER", "HISTORY_HEADER_INCOMPLETE")
        }
        RowHistoryHeaderError::CountExceedsCapacity => {
            ("COUNT_EXCEEDS_CAPACITY", "HISTORY_COUNT_EXCEEDS_CAPACITY")
        }
        RowHistoryHeaderError::CapacityExceedsLimit => {
            ("CAPACITY_LIMIT_EXCEEDED", "HISTORY_CAPACITY_LIMIT_EXCEEDED")
        }
        RowHistoryHeaderError::NullDataAddress => {
            ("NULL_DATA_POINTER", "HISTORY_DATA_POINTER_NULL")
        }
        RowHistoryHeaderError::MisalignedDataAddress => {
            ("MISALIGNED_DATA_POINTER", "HISTORY_DATA_POINTER_MISALIGNED")
        }
        RowHistoryHeaderError::SpanOverflow => {
            ("ARRAY_SPAN_OVERFLOW", "HISTORY_ARRAY_SPAN_OVERFLOW")
        }
        RowHistoryHeaderError::MalformedElement => {
            ("ELEMENT_SIZE_INVALID", "HISTORY_ELEMENT_SIZE_INVALID")
        }
    }
}

fn history_read_error(
    header: Option<RowHistoryHeader>,
    status: &'static str,
    code: &'static str,
    message: String,
) -> RowHistoryRead {
    RowHistoryRead {
        header,
        status,
        data_allocation_base: None,
        capacity_bytes: None,
        elements: Vec::new(),
        error: Some(ReportError {
            field: "row_history",
            code,
            message,
        }),
    }
}

fn readable_span_segments(
    handle: HANDLE,
    address: u64,
    length: u64,
) -> Result<(u64, Vec<RemoteSpanSegment>), ReportError> {
    let Some(end) = address.checked_add(length) else {
        return Err(ReportError {
            field: "row_history",
            code: "HISTORY_ARRAY_SPAN_OVERFLOW",
            message: "history array end address overflowed".to_owned(),
        });
    };
    let mut cursor = address;
    let mut allocation_base = None;
    let mut segments = Vec::new();
    while cursor < end {
        let Some(region) = query_region(handle, cursor) else {
            return Err(ReportError {
                field: "row_history",
                code: "HISTORY_ARRAY_REGION_QUERY_FAILED",
                message: format!("could not query history array at 0x{cursor:X}"),
            });
        };
        let Some(region_end) = region.region_base.checked_add(region.region_size) else {
            return Err(ReportError {
                field: "row_history",
                code: "HISTORY_ARRAY_REGION_OVERFLOW",
                message: format!("history array region at 0x{cursor:X} has invalid bounds"),
            });
        };
        if cursor < region.region_base
            || cursor >= region_end
            || region.state != MEM_COMMIT
            || !readable_protection(region.protection)
        {
            return Err(ReportError {
                field: "row_history",
                code: "HISTORY_ARRAY_REGION_UNREADABLE",
                message: format!("history array at 0x{cursor:X} crosses an unreadable region"),
            });
        }
        if allocation_base.is_some_and(|base| base != region.allocation_base) {
            return Err(ReportError {
                field: "row_history",
                code: "HISTORY_ARRAY_ALLOCATION_MISMATCH",
                message: "history array capacity crosses allocation boundaries".to_owned(),
            });
        }
        allocation_base.get_or_insert(region.allocation_base);
        let segment_end = end.min(region_end);
        segments.push(RemoteSpanSegment {
            address: cursor,
            length: segment_end - cursor,
        });
        cursor = segment_end;
    }
    Ok((allocation_base.unwrap_or_default(), segments))
}

fn read_remote_segments(
    handle: HANDLE,
    segments: &[RemoteSpanSegment],
    length: usize,
) -> Result<Vec<u8>, ReportError> {
    let mut bytes = Vec::with_capacity(length);
    for segment in segments {
        let remaining = length.saturating_sub(bytes.len());
        if remaining == 0 {
            break;
        }
        let segment_length = usize::try_from(segment.length)
            .unwrap_or(usize::MAX)
            .min(remaining);
        let mut part = vec![0; segment_length];
        if !read_exact(handle, segment.address, &mut part) {
            return Err(ReportError {
                field: "row_history",
                code: "HISTORY_ARRAY_READ_FAILED",
                message: format!(
                    "ReadProcessMemory failed at 0x{:X} for {segment_length} bytes (Win32 error {})",
                    segment.address,
                    unsafe { GetLastError() }
                ),
            });
        }
        bytes.extend_from_slice(&part);
    }
    if bytes.len() != length {
        return Err(ReportError {
            field: "row_history",
            code: "HISTORY_ARRAY_READ_SHORT",
            message: format!("read {} of {length} requested history bytes", bytes.len()),
        });
    }
    Ok(bytes)
}

pub(super) fn sample_row_history_once(
    handle: HANDLE,
    slot_address: u64,
    slot_allocation_base: u64,
    row_allocation_base: u64,
    history: &mut MetricsRowHistory,
    observed_at_unix_ms: u64,
) -> bool {
    let previous_header = history.last_header;
    let read = read_row_history_state(
        handle,
        slot_address,
        history.row_address,
        slot_allocation_base,
        row_allocation_base,
    );
    let current_header = read.header;
    let mut sample = MetricsRowHistorySample {
        observed_at_unix_ms,
        status: read.status,
        data_address: current_header.map(|header| header.data_address),
        count: current_header.map(|header| header.count),
        capacity: current_header.map(|header| header.capacity),
        capacity_bytes: read.capacity_bytes,
        data_allocation_base: read.data_allocation_base,
        data_pointer_changed: previous_header
            .zip(current_header)
            .map(|(before, after)| before.data_address != after.data_address),
        count_changed: previous_header
            .zip(current_header)
            .map(|(before, after)| before.count != after.count),
        capacity_changed: previous_header
            .zip(current_header)
            .map(|(before, after)| before.capacity != after.capacity),
        changed_elements: Vec::new(),
        removed_element_indices: Vec::new(),
        read_error: read.error.clone(),
    };
    let mut complete = matches!(read.status, "READ_OK" | "EMPTY") && read.error.is_none();
    if complete {
        let header = current_header.expect("successful history reads have a decoded header");
        match compare_row_history_elements(
            &history.last_elements,
            &read.elements,
            header.data_address,
        ) {
            Ok((changed, removed)) => {
                sample.changed_elements = changed;
                sample.removed_element_indices = removed;
                history.last_elements = read.elements;
            }
            Err(error) => {
                complete = false;
                sample.status = "ELEMENT_DIFF_INVALID";
                sample.read_error = Some(ReportError {
                    field: "row_history",
                    code: "HISTORY_ELEMENT_DIFF_INVALID",
                    message: format!("could not compare row history samples: {error:?}"),
                });
            }
        }
    }
    if let Some(header) = current_header {
        history.last_header = Some(header);
    }
    history.samples.push(sample);
    complete
}
