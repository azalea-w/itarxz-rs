pub mod decompressor;
pub mod minilzma;
pub mod tar_parser;
pub mod types;
pub mod xz_parser;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Take, Write};

use anyhow::{Context, Result, bail};

fn strip_file_prefix(file: &mut File, path: &str, length: u64, buffer_size: usize) -> Result<()> {
    let metadata = file
        .metadata()
        .context("getting file metadata for stripping")?;
    let total_size = metadata.len();

    if length == 0 || length >= total_size {
        return Ok(());
    }

    // * Use a separate handle for reading to have an independent offset.
    // * This allows the OS to optimize read-ahead and write-behind separately
    // * and eliminates the need for frequent seeks between read and write positions.
    let mut reader = File::open(path).context("opening file for reading during stripping")?;
    reader
        .seek(SeekFrom::Start(length))
        .context("seeking to read position")?;

    file.seek(SeekFrom::Start(0))
        .context("seeking to write position")?;

    // * Use buffered IO to minimize the number of system calls.
    let mut buffered_reader = std::io::BufReader::with_capacity(buffer_size, reader);
    let mut buffered_writer = std::io::BufWriter::with_capacity(buffer_size, file);

    std::io::copy(&mut buffered_reader, &mut buffered_writer)
        .context("copying data during stripping")?;
    buffered_writer
        .flush()
        .context("flushing buffered writer")?;

    let written_size = total_size - length;
    let file_inner = buffered_writer.into_inner().unwrap();

    file_inner
        .set_len(written_size)
        .context("truncating file after stripping")?;
    file_inner
        .sync_data()
        .context("syncing file data after stripping")?;
    file_inner
        .seek(SeekFrom::Start(0))
        .context("resetting file pointer after stripping")?;
    Ok(())
}

fn parse_size(s: &str) -> Result<u64> {
    let s = s.to_uppercase();
    if s.ends_with("GB") || s.ends_with("G") {
        let num: u64 = s
            .trim_end_matches("GB")
            .trim_end_matches('G')
            .parse()
            .context("invalid GB size")?;
        Ok(num * 1024 * 1024 * 1024)
    } else if s.ends_with("MB") || s.ends_with("M") {
        let num: u64 = s
            .trim_end_matches("MB")
            .trim_end_matches('M')
            .parse()
            .context("invalid MB size")?;
        Ok(num * 1024 * 1024)
    } else if s.ends_with("KB") || s.ends_with("K") {
        let num: u64 = s
            .trim_end_matches("KB")
            .trim_end_matches('K')
            .parse()
            .context("invalid KB size")?;
        Ok(num * 1024)
    } else {
        s.parse().context("invalid size in bytes")
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        println!(
            "Usage: {} <input_file.xz> [buffer_size] [strip_threshold]",
            args[0]
        );
        println!("Example: {} input.xz 10MB 100MB", args[0]);
        return Ok(());
    }

    let input_path = &args[1];
    if !input_path.to_lowercase().ends_with(".xz") {
        bail!("Input file must have .xz extension");
    }
    let output_path = &input_path[..input_path.len() - 7];

    let buffer_size = if args.len() >= 3 {
        parse_size(&args[2])? as usize
    } else {
        1024 * 1024 // 1 MiB default
    };

    let strip_threshold = if args.len() >= 4 {
        parse_size(&args[3])?
    } else {
        1024 * 1024 // 1 MiB default
    };

    println!("Configuration:");
    println!("  Buffer size: {} bytes", buffer_size);
    println!("  Strip threshold: {} bytes", strip_threshold);

    let mut infile = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(input_path)
        .with_context(|| format!("opening {} with write access", input_path))?;
    let file_size = infile.seek(SeekFrom::End(0)).context("getting file size")?;
    infile.rewind()?;

    let footer = xz_parser::parse_stream_footer(&mut infile)?;
    let index_info = xz_parser::parse_index(&mut infile, file_size, footer.backward_size)?;
    let stream_header_size = 12u64;
    let block_header = xz_parser::parse_block_header(&mut infile, stream_header_size, &index_info)?;

    let block_data_offset = stream_header_size + block_header.block_total_size as u64;

    println!(
        "XZ headers parsed ({} bytes). Strip will be combined with first data strip.",
        block_data_offset
    );

    println!("Starting decompression...");

    {
        let mut reader = minilzma::lzma2_reader::Lzma2Reader::new(
            infile.try_clone()?.take(block_header.compressed_size),
            1 << 23,
            None,
        );

        drop(infile);

        println!("Starting decompression and untarring to {}...", output_path);

        let mut stripping_reader = StrippingReader {
            reader: &mut reader,
            input_path,
            strip_threshold,
            buffer_size,
            total_uncompressed: block_header.uncompressed_size,
            current_uncompressed: 0,
            initial_offset: block_data_offset,
        };

        let mut tar = tar_parser::TarParser::new(&mut stripping_reader);
        tar.untar(output_path)?;
    }

    println!(
        "\nSuccessfully decompressed and extracted {} bytes into {}",
        block_header.uncompressed_size, output_path
    );

    println!("Removing input file {}...", input_path);
    std::fs::remove_file(input_path)
        .with_context(|| format!("removing input file {}", input_path))?;

    Ok(())
}

struct StrippingReader<'a> {
    reader: &'a mut minilzma::lzma2_reader::Lzma2Reader<Take<File>>,
    input_path: &'a str,
    strip_threshold: u64,
    buffer_size: usize,
    total_uncompressed: u64,
    current_uncompressed: u64,
    initial_offset: u64,
}

impl<'a> Read for StrippingReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let bytes_read = self.reader.read(buf)?;
        if bytes_read == 0 {
            return Ok(0);
        }
        self.current_uncompressed += bytes_read as u64;

        let progress = (self.current_uncompressed as f64 / self.total_uncompressed as f64) * 100.0;
        print!(
            "\rExtracting: {:.2}% ({}/{} bytes)",
            progress, self.current_uncompressed, self.total_uncompressed
        );
        let _ = std::io::stdout().flush();

        let total_in = self.reader.total_in();
        if total_in >= self.strip_threshold {
            let to_strip = total_in + self.initial_offset;

            let remaining_compressed: u64;
            {
                let inner_take = self.reader.inner_mut();
                remaining_compressed = inner_take.limit();
                let inner_file = inner_take.get_mut();
                if let Err(e) =
                    strip_file_prefix(inner_file, self.input_path, to_strip, self.buffer_size)
                {
                    return Err(std::io::Error::other(e));
                }

                inner_file.sync_all()?;
            }

            let new_infile = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(self.input_path)?;

            self.reader
                .replace_inner(new_infile.take(remaining_compressed));
            self.reader.set_count(0);
            self.initial_offset = 0;
        }

        Ok(bytes_read)
    }
}
