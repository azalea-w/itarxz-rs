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

    println!("\nStripping {} bytes from {}", length, path);

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
    let mut args: Vec<String> = std::env::args().collect();
    let mut dry_run = false;
    if let Some(pos) = args.iter().position(|a| a == "--dry-run" || a == "-d") {
        dry_run = true;
        args.remove(pos);
    }

    if args.len() < 2 {
        println!(
            "Usage: {} <input_file.xz> [buffer_size] [strip_threshold] [--dry-run|-d]",
            args[0]
        );
        println!("Example: {} input.xz 10MB 100MB --dry-run", args[0]);
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
    if dry_run {
        println!("  Dry-run mode: ENABLED");
    }

    let mut infile = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(input_path)
        .with_context(|| format!("opening {} with write access", input_path))?;
    let file_size = infile.seek(SeekFrom::End(0)).context("getting file size")?;
    infile.rewind()?;
    let footer = xz_parser::parse_stream_footer(&mut infile)?;
    let index_info = xz_parser::parse_index(&mut infile, file_size, footer.backward_size)?;
    infile.seek(SeekFrom::Start(12))?;

    println!("Starting decompression...");

    let total_uncompressed: u64 = index_info.records.iter().map(|r| r.uncompressed_size).sum();
    let check_size = xz_parser::get_check_size(footer.stream_flags);

    {
        let mut stripping_reader = StrippingReader {
            file: infile,
            input_path,
            strip_threshold,
            buffer_size,
            records: index_info.records,
            current_record_idx: 0,
            reader: None,
            total_uncompressed,
            current_uncompressed: 0,
            initial_offset: 12, // * Stream header
            check_size,
            dry_run,
            bytes_processed_since_last_strip: 12,
        };

        println!("Starting decompression and untarring to {}...", output_path);

        let mut tar = tar_parser::TarParser::new(&mut stripping_reader);
        tar.untar(output_path, dry_run)?;
    }

    println!(
        "\nSuccessfully decompressed and extracted {} bytes into {}",
        total_uncompressed, output_path
    );

    if !dry_run {
        println!("Removing input file {}...", input_path);
        std::fs::remove_file(input_path)
            .with_context(|| format!("removing input file {}", input_path))?;
    } else {
        println!("[Dry-run] Would remove input file {}", input_path);
    }

    Ok(())
}

struct StrippingReader<'a> {
    file: File,
    input_path: &'a str,
    strip_threshold: u64,
    buffer_size: usize,
    dry_run: bool,

    records: Vec<crate::types::IndexRecord>,
    current_record_idx: usize,
    reader: Option<minilzma::lzma2_reader::Lzma2Reader<Take<File>>>,

    total_uncompressed: u64,
    current_uncompressed: u64,
    initial_offset: u64,
    check_size: usize,
    bytes_processed_since_last_strip: u64,
}

impl<'a> StrippingReader<'a> {
    fn setup_next_block(&mut self) -> std::io::Result<()> {
        if self.current_record_idx >= self.records.len() {
            return Ok(());
        }

        let record = &self.records[self.current_record_idx];
        let block_header =
            xz_parser::parse_block_header(&mut self.file, record).map_err(std::io::Error::other)?;

        self.initial_offset += block_header.block_total_size as u64;
        self.bytes_processed_since_last_strip += block_header.block_total_size as u64;

        let reader = minilzma::lzma2_reader::Lzma2Reader::new(
            self.file.try_clone()?.take(block_header.compressed_size),
            block_header.dict_size,
            None,
        );
        self.reader = Some(reader);
        Ok(())
    }
}

impl<'a> Read for StrippingReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.reader.is_none() {
                if self.current_record_idx >= self.records.len() {
                    return Ok(0);
                }
                self.setup_next_block()?;
            }

            let reader = self.reader.as_mut().unwrap();
            let prev_in = reader.total_in();
            let bytes_read = reader.read(buf)?;

            if bytes_read > 0 {
                self.current_uncompressed += bytes_read as u64;

                let progress =
                    (self.current_uncompressed as f64 / self.total_uncompressed as f64) * 100.0;
                print!(
                    "\rExtracting: {:.2}% ({}/{} bytes)",
                    progress, self.current_uncompressed, self.total_uncompressed
                );
                let _ = std::io::stdout().flush();

                let total_in = reader.total_in();
                let consumed_in = total_in - prev_in;
                self.bytes_processed_since_last_strip += consumed_in;

                if self.bytes_processed_since_last_strip >= self.strip_threshold {
                    let to_strip = self.bytes_processed_since_last_strip;

                    if self.dry_run {
                        println!(
                            "\n[Dry-run] Would strip {} bytes from {}",
                            to_strip, self.input_path
                        );
                    } else {
                        let remaining_compressed: u64;
                        {
                            let inner_take = reader.inner_mut();
                            remaining_compressed = inner_take.limit();
                            let inner_file = inner_take.get_mut();
                            if let Err(e) = strip_file_prefix(
                                inner_file,
                                self.input_path,
                                to_strip,
                                self.buffer_size,
                            ) {
                                return Err(std::io::Error::other(e));
                            }

                            inner_file.sync_all()?;
                        }

                        let new_infile = std::fs::OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open(self.input_path)?;

                        reader.replace_inner(new_infile.take(remaining_compressed));
                    }
                    reader.set_count(0);
                    self.initial_offset = 0;
                    self.bytes_processed_since_last_strip = 0;
                }

                return Ok(bytes_read);
            }

            // * End of current block
            let reader_owned = self.reader.take().unwrap();
            let total_in = reader_owned.total_in();
            let take = reader_owned.into_inner();
            let mut inner_file = take.into_inner();

            // * Handle padding and check
            let record = &self.records[self.current_record_idx];
            let padding_size = ((record.unpadded_size + 3) & !3) - record.unpadded_size;
            let to_skip = self.check_size as u64 + padding_size;
            if to_skip > 0 {
                inner_file.seek(SeekFrom::Current(to_skip as i64))?;
            }

            self.initial_offset += total_in + self.check_size as u64 + padding_size;
            self.bytes_processed_since_last_strip += self.check_size as u64 + padding_size;
            self.file = inner_file;
            self.current_record_idx += 1;
        }
    }
}
