use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpcError {
    InvalidPart,
    ResourceLimit,
    NumericOverflow,
}

pub(crate) struct Part {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

struct CentralRecord {
    name: Vec<u8>,
    crc32: u32,
    size: u32,
    local_offset: u32,
}

pub(crate) fn serialize(parts: Vec<Part>, maximum: u64) -> Result<Vec<u8>, OpcError> {
    let entry_count = u16::try_from(parts.len()).map_err(|_| OpcError::ResourceLimit)?;
    let mut names = BTreeSet::new();
    let mut required = 22_u64;
    for part in &parts {
        validate_name(&part.name)?;
        if !names.insert(part.name.as_str()) {
            return Err(OpcError::InvalidPart);
        }
        let name_len = u16::try_from(part.name.len()).map_err(|_| OpcError::ResourceLimit)?;
        let size = u32::try_from(part.bytes.len()).map_err(|_| OpcError::ResourceLimit)?;
        required = required
            .checked_add(30)
            .and_then(|value| value.checked_add(u64::from(name_len)))
            .and_then(|value| value.checked_add(u64::from(size)))
            .and_then(|value| value.checked_add(46))
            .and_then(|value| value.checked_add(u64::from(name_len)))
            .ok_or(OpcError::NumericOverflow)?;
    }
    if required > maximum {
        return Err(OpcError::ResourceLimit);
    }
    let capacity = usize::try_from(required).map_err(|_| OpcError::ResourceLimit)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(capacity)
        .map_err(|_| OpcError::ResourceLimit)?;
    let mut central = Vec::new();
    central
        .try_reserve_exact(parts.len())
        .map_err(|_| OpcError::ResourceLimit)?;

    for part in parts {
        let name = part.name.into_bytes();
        let name_len = u16::try_from(name.len()).map_err(|_| OpcError::ResourceLimit)?;
        let size = u32::try_from(part.bytes.len()).map_err(|_| OpcError::ResourceLimit)?;
        let local_offset = u32::try_from(output.len()).map_err(|_| OpcError::ResourceLimit)?;
        let crc32 = crc32(&part.bytes);
        push_u32(&mut output, 0x0403_4B50);
        push_u16(&mut output, 20);
        push_u16(&mut output, 0x0800);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0x0021);
        push_u32(&mut output, crc32);
        push_u32(&mut output, size);
        push_u32(&mut output, size);
        push_u16(&mut output, name_len);
        push_u16(&mut output, 0);
        output.extend_from_slice(&name);
        output.extend_from_slice(&part.bytes);
        central.push(CentralRecord {
            name,
            crc32,
            size,
            local_offset,
        });
    }

    let central_offset = u32::try_from(output.len()).map_err(|_| OpcError::ResourceLimit)?;
    for record in &central {
        let name_len = u16::try_from(record.name.len()).map_err(|_| OpcError::ResourceLimit)?;
        push_u32(&mut output, 0x0201_4B50);
        push_u16(&mut output, 20);
        push_u16(&mut output, 20);
        push_u16(&mut output, 0x0800);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0x0021);
        push_u32(&mut output, record.crc32);
        push_u32(&mut output, record.size);
        push_u32(&mut output, record.size);
        push_u16(&mut output, name_len);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0);
        push_u32(&mut output, 0);
        push_u32(&mut output, record.local_offset);
        output.extend_from_slice(&record.name);
    }
    let central_size = u32::try_from(output.len())
        .map_err(|_| OpcError::ResourceLimit)?
        .checked_sub(central_offset)
        .ok_or(OpcError::NumericOverflow)?;
    push_u32(&mut output, 0x0605_4B50);
    push_u16(&mut output, 0);
    push_u16(&mut output, 0);
    push_u16(&mut output, entry_count);
    push_u16(&mut output, entry_count);
    push_u32(&mut output, central_size);
    push_u32(&mut output, central_offset);
    push_u16(&mut output, 0);
    Ok(output)
}

fn validate_name(name: &str) -> Result<(), OpcError> {
    if name.is_empty()
        || name.starts_with('/')
        || name.ends_with('/')
        || name.contains('\\')
        || name
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(OpcError::InvalidPart);
    }
    Ok(())
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
