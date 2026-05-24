use std::io::{Read, Write};

use crate::minilzma::lzma2_reader::Lzma2Reader;
use anyhow::Result;

pub fn decompress_lzma2<R: Read, W: Write, F: FnMut(&mut Lzma2Reader<R>) -> Result<()>>(
    input: R,
    output: &mut W,
    chunk_size: usize,
    mut on_progress: F,
) -> Result<()> {
    let mut reader = Lzma2Reader::new(input, 1 << 23, None); // Use 8MiB as a safe default dict size
    let mut buffer = vec![0u8; chunk_size];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        output.write_all(&buffer[..bytes_read])?;
        on_progress(&mut reader)?;
    }

    Ok(())
}
