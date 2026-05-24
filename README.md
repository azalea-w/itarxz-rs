# `itarxz-rs`
*an in-place .tar.xz extractor*

---

> [!WARNING]
> This **WILL** destroy your .tar.xz file. It works by stripping the input file as it processes it and finally removes the input file entirely upon successful completion. Please do not use this on files you don't have a backup of.

## What's this?
This is an in-place .tar.xz extractor. Unlike traditional extractors that decompress to a `.tar` file first, `itarxz-rs` parses the decompressed stream and extracts files whilst also stripping the compressed data from the input file meaning that you don't need twice the archive data for extraction.

I thought I might share this such that someone else can adapt it to be more dynamic. Who knows? Maybe someone else is out there with storage constraints.

## How does this work?
The tool works by exploiting the block-based structure of the XZ format and streaming the decompressed data directly into a custom TAR parser:
1. **Header Stripping**: It first parses the XZ stream header and block header. It then removes these headers from the beginning of the file by shifting the remaining data forward.
2. **Streaming Decompression & Extraction**: It starts decompressing the LZMA2 stream. The decompressed output is fed directly into a TAR parser.
3. **In-place Truncation**: As decompression progresses and a configurable threshold of compressed data is read, the tool pauses, shifts the remaining compressed data to the beginning of the file, and truncates the file to reclaim space.
4. **Direct Untarring**: The TAR parser identifies file and directory entries in the stream and writes them directly to the destination directory.
5. **Cleanup**: Once extraction is complete, the input `.xz` file is automatically deleted.

This process effectively "eats" the compressed file as it produces the extracted contents, allowing for extraction in environments with extremely limited storage space.

## How to use this?
`itarxz-rs <input_file.tar.xz> [buffer_size] [strip_threshold]`

- `input_file.tar.xz`: The path to the .tar.xz file you want to decompress. The output will be a directory with the same name (minus the `.tar.xz` extension).
- `buffer_size` (optional): The size of the buffer used for file operations (e.g., `10MB`, `1GB`). Defaults to `1MB`.
- `strip_threshold` (optional): How much compressed data should be processed before the input file is stripped and truncated (e.g., `100MB`). Defaults to `1MB`.

**Example:**
```bash
itarxz-rs archive.tar.xz 10MB 100MB
```

## Attribution
The custom LZMA decoder `minilzma` in this project is based on the [lzma-rust2](https://github.com/hasenbanck/lzma-rust2) project by [hasenbanck](https://github.com/hasenbanck) licensed under Apache 2.0.
