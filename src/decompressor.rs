use std::io::{BufRead, Read, Write};

use anyhow::Result;
use lzma_rs::lzma2_decompress;

pub fn decompress_lzma2<R: Read + BufRead, W: Write>(mut input: R, output: &mut W) -> Result<()> {
    lzma2_decompress(&mut input, output).map_err(Into::into)
}
