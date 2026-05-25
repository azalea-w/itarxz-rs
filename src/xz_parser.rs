use std::io::{Read, Seek, SeekFrom};

use anyhow::{Context, Result, bail};
use crc32fast;

use crate::types::*;

pub fn parse_stream_header<R: Read + Seek>(file: &mut R) -> Result<StreamHeader> {
    let mut buf = [0u8; 12];
    file.read_exact(&mut buf).context("reading stream header")?;

    if buf[..6] != XZ_HEADER_MAGIC {
        bail!("Invalid XZ stream header magic");
    }

    let flags = [buf[6], buf[7]];
    let expected_crc = u32::from_le_bytes(buf[8..12].try_into()?);
    let computed_crc = crc32fast::hash(&flags);

    if computed_crc != expected_crc {
        bail!(
            "Invalid XZ stream header CRC: expected 0x{:08X}, got 0x{:08X}",
            expected_crc,
            computed_crc
        );
    }

    Ok(StreamHeader {
        stream_flags: flags,
    })
}

pub fn parse_stream_footer<R: Read + Seek>(file: &mut R) -> Result<StreamFooter> {
    file.seek(SeekFrom::End(-12))
        .context("seeking to stream footer")?;

    let mut buf = [0u8; 12];
    file.read_exact(&mut buf).context("reading stream footer")?;

    let footer_crc = u32::from_le_bytes(buf[..4].try_into()?);
    let backward_size_encoded = u32::from_le_bytes(buf[4..8].try_into()?);
    let flags = [buf[8], buf[9]];
    let magic = &buf[10..12];

    if magic != b"YZ" {
        bail!("Invalid XZ footer magic");
    }

    let mut crc_data = [0u8; 6];
    crc_data[..4].copy_from_slice(&buf[4..8]);
    crc_data[4..].copy_from_slice(&flags);
    let computed_crc = crc32fast::hash(&crc_data);
    if footer_crc != computed_crc {
        bail!(
            "Invalid XZ footer CRC: expected 0x{:08X}, got 0x{:08X}",
            footer_crc,
            computed_crc
        );
    }

    let backward_size = ((backward_size_encoded + 1) as u64) << 2;
    Ok(StreamFooter {
        backward_size: backward_size as u32,
        stream_flags: flags,
    })
}

pub fn get_check_size(flags: [u8; 2]) -> usize {
    let check_id = flags[1] & 0x0F;
    match check_id {
        0 => 0,
        1 => 4,   // * CRC32
        4 => 8,   // * CRC64
        10 => 32, // * SHA-256
        _ => 0,
    }
}

pub fn parse_index<R: Read + Seek>(
    file: &mut R,
    file_size: u64,
    footer_backward_size: u32,
) -> Result<IndexInfo> {
    let index_offset = file_size - 12 - footer_backward_size as u64;
    file.seek(SeekFrom::Start(index_offset))
        .context("seeking to Index")?;

    let index_len = footer_backward_size as usize;
    let mut index_data = vec![0u8; index_len];
    file.read_exact(&mut index_data).context("reading Index")?;

    if index_len < 2 {
        bail!("Index too short");
    }
    let index_indicator = index_data[0];
    if index_indicator != 0x00 {
        bail!("Unexpected Index Indicator");
    }
    let (num_records, mut pos) =
        read_multibyte_integer(&index_data[1..]).context("parsing Number of Records")?;
    pos += 1; // * account for index indicator

    let mut records = Vec::with_capacity(num_records as usize);
    for _ in 0..num_records {
        let (unpadded_size, unpadded_len) =
            read_multibyte_integer(&index_data[pos..]).context("parsing Unpadded Size")?;
        pos += unpadded_len;
        let (uncompressed_size, uncompressed_len) =
            read_multibyte_integer(&index_data[pos..]).context("parsing Uncompressed Size")?;
        pos += uncompressed_len;
        records.push(IndexRecord {
            unpadded_size,
            uncompressed_size,
        });
    }

    let stored_crc = u32::from_le_bytes(index_data[index_len - 4..].try_into()?);
    let computed_crc = crc32fast::hash(&index_data[..index_len - 4]);
    if stored_crc != computed_crc {
        bail!(
            "Invalid Index CRC: expected 0x{:08X}, got 0x{:08X}",
            stored_crc,
            computed_crc
        );
    }

    Ok(IndexInfo { records })
}

pub fn parse_block_header<R: Read + Seek>(
    file: &mut R,
    record: &IndexRecord,
) -> Result<BlockHeader> {
    let mut size_byte = [0u8; 1];
    file.read_exact(&mut size_byte)
        .context("reading block header size")?;
    let block_total_size = ((size_byte[0] as usize) + 1) * 4;
    if block_total_size < 2 {
        bail!("Block header too small");
    }

    let rest_len = block_total_size - 1;
    let mut rest = vec![0u8; rest_len];
    file.read_exact(&mut rest)
        .context("reading rest of block header")?;

    let mut full_header = Vec::with_capacity(block_total_size);
    full_header.extend_from_slice(&size_byte);
    full_header.extend_from_slice(&rest);

    if full_header.len() < 4 {
        bail!("Block header too short for CRC");
    }
    let stored_crc = u32::from_le_bytes(full_header[full_header.len() - 4..].try_into()?);
    let computed_crc = crc32fast::hash(&full_header[..full_header.len() - 4]);
    if stored_crc != computed_crc {
        bail!(
            "Invalid block header CRC: expected 0x{:08X}, got 0x{:08X}",
            stored_crc,
            computed_crc
        );
    }

    let flags = rest[0];
    let compressed_size_present = (flags & 0x02) != 0;
    let uncompressed_size_present = (flags & 0x01) != 0;

    let mut pos = 1;

    let compressed_size = if compressed_size_present {
        let (val, len) = read_multibyte_integer(&rest[pos..]).context("parsing compressed size")?;
        pos += len;
        val + 1
    } else {
        record.unpadded_size - block_total_size as u64
    };

    let uncompressed_size = if uncompressed_size_present {
        let (val, _) = read_multibyte_integer(&rest[pos..]).context("parsing uncompressed size")?;
        val + 1
    } else {
        record.uncompressed_size
    };

    Ok(BlockHeader {
        compressed_size,
        uncompressed_size,
        block_total_size,
    })
}
