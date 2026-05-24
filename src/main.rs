pub mod decompressor;
pub mod minilzma;
pub mod types;
pub mod xz_parser;

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};

use anyhow::{Context, Result};

// const CHUNK_SIZE: usize = 1 * 1024 * 1024; // 1 MiB

fn main() -> Result<()> {
    let mut infile = File::open("test.tar.xz").context("opening test.tar.xz")?;
    let file_size = infile.seek(SeekFrom::End(0)).context("getting file size")?;
    infile.rewind()?;

    let _stream_header = xz_parser::parse_stream_header(&mut infile)?;
    let stream_header_size = 12u64;

    let footer = xz_parser::parse_stream_footer(&mut infile)?;
    let index_info = xz_parser::parse_index(&mut infile, file_size, footer.backward_size)?;

    let block_header = xz_parser::parse_block_header(&mut infile, stream_header_size, &index_info)?;

    let block_data_offset = stream_header_size + block_header.block_total_size as u64;
    infile
        .seek(SeekFrom::Start(block_data_offset))
        .context("seeking to compressed data")?;

    let compressed_reader = BufReader::new(infile.take(block_header.compressed_size));

    let mut outfile = File::create("test.tar").context("creating test.tar")?;
    decompressor::decompress_lzma2(compressed_reader, &mut outfile)
        .context("decompression failed")?;

    println!(
        "Successfully decompressed {} bytes into test.tar",
        block_header.uncompressed_size
    );
    Ok(())
}
