# `itarxz-rs`
*an in-place .tar.xz extractor*

---

> [!WARNING]
> This **WILL** destroy your .tar.xz file. Please do not use this on files you don't have a backup of.
> This will also not work properly on Windows. This is a proof of concept and not a production-ready tool.

## What's this?
This is an in-place .tar.xz extractor. It will probably not suit your needs, it will probably not even work for your specific use case.
I thought I might share this despite that such that someone else can adapt it to be more dynamic. Who knows? Maybe someone else is out there with storage constraints.

## How does this work?
The tool works by exploiting the block-based structure of the XZ format. It performs decompression while simultaneously reclaiming space from the input file:
1. **Header Stripping**: It first parses the XZ stream header and block header. It then removes these headers from the beginning of the file by shifting the remaining data forward.
2. **Block Decompression**: It starts decompressing the LZMA2 stream within the XZ block.
3. **In-place Truncation**: As decompression progresses and a configurable threshold of compressed data is read, the tool pauses, shifts the remaining compressed data to the beginning of the file, and truncates the file.
4. **Resumption**: It then resumes decompression from the new offset.

This process effectively "eats" the compressed file as it produces the uncompressed output, allowing for decompression in environments with very limited storage space.

## How to use this?
`itarxz-rs <input_file.xz> [buffer_size] [strip_threshold]`

- `input_file.xz`: The path to the .tar.xz file you want to decompress.
- `buffer_size` (optional): The size of the buffer used for file operations (e.g., `10MB`, `1GB`). Defaults to `1MB`.
- `strip_threshold` (optional): How much compressed data should be processed before the input file is stripped and truncated (e.g., `100MB`). Defaults to `1MB`.

**Example:**
```bash
itarxz-rs archive.tar.xz 10MB 100MB
```

## Attribution
The custom LZMA decoder `minilzma` in this project is based on the [lzma-rust2](https://github.com/hasenbanck/lzma-rust2) project by [hasenbanck](https://github.com/hasenbanck) licensed under MIT.
