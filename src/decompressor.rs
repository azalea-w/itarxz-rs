use std::io::{BufRead, Read, Write};

use anyhow::Result;
use crate::minilzma::lzma2_reader::Lzma2Reader;

pub fn decompress_lzma2<R: Read + BufRead, W: Write>(
    input: R,
    output: &mut W,
    chunk_size: usize,
) -> Result<()> {
    let mut reader = Lzma2Reader::new(input, 1 << 23, None); // Use 8MiB as a safe default dict size
    let mut buffer = vec![0u8; chunk_size];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        output.write_all(&buffer[..bytes_read])?;
    }

    Ok(())
}
