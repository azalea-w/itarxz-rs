use anyhow::{Result, bail};

pub const XZ_HEADER_MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];

#[derive(Debug)]
pub struct StreamHeader {
    pub stream_flags: [u8; 2],
}

#[derive(Debug)]
pub struct StreamFooter {
    pub backward_size: u32,
    pub stream_flags: [u8; 2],
}

#[derive(Debug, Clone)]
pub struct IndexRecord {
    pub unpadded_size: u64,
    pub uncompressed_size: u64,
}

#[derive(Debug)]
pub struct IndexInfo {
    pub records: Vec<IndexRecord>,
}

#[derive(Debug)]
pub struct BlockHeader {
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub block_total_size: usize,
    pub dict_size: u32,
}

pub fn read_multibyte_integer(data: &[u8]) -> Result<(u64, usize)> {
    let mut value: u64 = 0;
    for (i, &byte) in data.iter().enumerate() {
        value |= ((byte & 0x7F) as u64) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
    }
    bail!("Incomplete multibyte integer");
}
