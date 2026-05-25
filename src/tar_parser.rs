use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::path::Path;

pub struct TarParser<R: Read> {
    reader: R,
}

impl<R: Read> TarParser<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn untar<P: AsRef<Path>>(&mut self, dest: P, dry_run: bool) -> Result<()> {
        let dest = dest.as_ref();
        if !dry_run && !dest.exists() {
            std::fs::create_dir_all(dest).context("creating destination directory")?;
        }

        let mut header = [0u8; 512];
        loop {
            self.reader
                .read_exact(&mut header)
                .context("reading tar header")?;

            if header.iter().all(|&b| b == 0) {
                break;
            }

            let name = self.parse_name(&header)?;
            let size = self.parse_octal(&header[124..136])?;
            let type_flag = header[156];

            let path = dest.join(name);

            match type_flag {
                b'0' | b'\0' => {
                    if dry_run {
                        println!("\n[Dry-run] Would create file {:?}", path);
                    } else {
                        if let Some(parent) = path.parent() {
                            std::fs::create_dir_all(parent).context("creating parent directory")?;
                        }
                        let mut file = std::fs::File::create(&path)
                            .with_context(|| format!("creating file {:?}", path))?;

                        let mut remaining = size;
                        let mut buffer = [0u8; 8192];
                        while remaining > 0 {
                            let to_read = remaining.min(buffer.len() as u64) as usize;
                            self.reader.read_exact(&mut buffer[..to_read])?;
                            file.write_all(&buffer[..to_read])?;
                            remaining -= to_read as u64;
                        }
                    }

                    if dry_run {
                        let mut remaining = size;
                        let mut buffer = [0u8; 8192];
                        while remaining > 0 {
                            let to_read = remaining.min(buffer.len() as u64) as usize;
                            self.reader.read_exact(&mut buffer[..to_read])?;
                            remaining -= to_read as u64;
                        }
                    }

                    let padding = (512 - (size % 512)) % 512;
                    if padding > 0 {
                        let mut pad_buf = [0u8; 512];
                        self.reader.read_exact(&mut pad_buf[..padding as usize])?;
                    }
                }
                b'5' => {
                    if dry_run {
                        println!("\n[Dry-run] Would create directory {:?}", path);
                    } else {
                        std::fs::create_dir_all(&path).context("creating directory")?;
                    }
                }
                _ => {
                    if dry_run {
                        println!(
                            "\n[Dry-run] Would skip entry of type {} at {:?}",
                            type_flag, path
                        );
                    }
                    let padding = (512 - (size % 512)) % 512;
                    let to_skip = size + padding;
                    std::io::copy(
                        &mut self.reader.by_ref().take(to_skip),
                        &mut std::io::sink(),
                    )?;
                }
            }
        }
        Ok(())
    }

    fn parse_name(&self, header: &[u8; 512]) -> Result<String> {
        let name_bytes = &header[0..100];
        let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(100);
        let name = std::str::from_utf8(&name_bytes[..end]).context("invalid UTF-8 in tar name")?;

        if &header[257..262] == b"ustar" {
            let prefix_bytes = &header[345..500];
            let prefix_end = prefix_bytes.iter().position(|&b| b == 0).unwrap_or(155);
            if prefix_end > 0 {
                let prefix = std::str::from_utf8(&prefix_bytes[..prefix_end])
                    .context("invalid UTF-8 in tar prefix")?;
                return Ok(format!("{}/{}", prefix, name));
            }
        }

        Ok(name.to_string())
    }

    fn parse_octal(&self, bytes: &[u8]) -> Result<u64> {
        if !bytes.is_empty() && (bytes[0] & 0x80) != 0 {
            let mut val: u64 = 0;
            for (i, &b) in bytes.iter().enumerate() {
                if i == 0 {
                    val = (b & 0x7f) as u64;
                } else {
                    val = (val << 8) | (b as u64);
                }
            }
            return Ok(val);
        }

        let s = std::str::from_utf8(bytes).context("invalid UTF-8 in octal field")?;
        let s = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        if s.is_empty() {
            return Ok(0);
        }
        u64::from_str_radix(s, 8).context("parsing octal")
    }
}
