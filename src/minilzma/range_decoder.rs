/*
   This code has largely been adapted from the lzma-rust2 crate.
   https://github.com/hasenbanck/lzma-rust2
   Licensed under Apache 2.0
*/

use std::io::Error;
use std::vec::Vec;

use super::{
    BIT_MODEL_TOTAL_BITS, ByteReader, MOVE_BITS, RC_BIT_MODEL_OFFSET, Read, SHIFT_BITS, error_eof,
    error_invalid_data, error_invalid_input, error_other,
};

pub(crate) struct RangeDecoder<R> {
    inner: R,
    range: u32,
    code: u32,
}

impl<R> RangeDecoder<R> {
    pub(crate) fn into_inner(self) -> R {
        self.inner
    }

    pub(crate) fn inner(&self) -> &R {
        &self.inner
    }

    pub(crate) fn inner_mut(&mut self) -> &mut R {
        &mut self.inner
    }
}

impl RangeDecoder<RangeDecoderBuffer> {
    pub(crate) fn new_buffer(size: usize) -> Self {
        Self {
            inner: RangeDecoderBuffer::new(size - 5),
            code: 0,
            range: 0,
        }
    }
}

impl<R: RangeReader> RangeDecoder<R> {
    pub(crate) fn new_stream(mut inner: R) -> anyhow::Result<RangeDecoder<R>> {
        let b = inner.try_read_u8()?;
        if b != 0x00 {
            return Err(anyhow::Error::new(error_invalid_input(
                "range decoder first byte is not zero",
            )));
        }
        let code = inner.read_u32_be()?;
        Ok(Self {
            inner,
            code,
            range: 0xFFFFFFFFu32,
        })
    }

    pub(crate) fn is_stream_finished(&self) -> bool {
        self.code == 0
    }
}

impl<R: RangeReader> RangeDecoder<R> {
    #[inline(always)]
    pub(crate) fn normalize(&mut self) {
        if self.range < 0x0100_0000 {
            let b = self.inner.read_u8() as u32;
            self.code = (self.code << SHIFT_BITS) | b;
            self.range <<= SHIFT_BITS;
        }
    }

    #[inline(always)]
    pub(crate) fn decode_bit(&mut self, prob: &mut u16) -> i32 {
        self.normalize();
        let bound = (self.range >> BIT_MODEL_TOTAL_BITS) * (*prob as u32);

        // This mask will be 0 for bit 0, and 0xFFFFFFFF for bit 1.
        let mask = 0u32.wrapping_sub((self.code >= bound) as u32);

        self.range = (bound & !mask) | ((self.range - bound) & mask);
        self.code -= bound & mask;

        let p = *prob as u32;
        let offset = RC_BIT_MODEL_OFFSET & !mask;
        *prob = p.wrapping_sub((p.wrapping_add(offset)) >> MOVE_BITS) as u16;

        (mask & 1) as i32
    }

    pub(crate) fn decode_bit_tree(&mut self, probs: &mut [u16]) -> i32 {
        let mut symbol = 1;
        loop {
            symbol = (symbol << 1) | self.decode_bit(&mut probs[symbol as usize]);
            if symbol >= probs.len() as i32 {
                break;
            }
        }
        symbol - probs.len() as i32
    }

    pub(crate) fn decode_reverse_bit_tree(&mut self, probs: &mut [u16]) -> i32 {
        let mut symbol = 1;
        let mut i = 0;
        let mut result = 0;
        loop {
            let bit = self.decode_bit(&mut probs[symbol as usize]);
            symbol = (symbol << 1) | bit;
            result |= bit << i;
            i += 1;
            if symbol >= probs.len() as i32 {
                break;
            }
        }
        result
    }

    pub(crate) fn decode_direct_bits(&mut self, count: u32) -> i32 {
        // The following loop is the original function structured in a way,
        // that hopefully the compiler can optimize better.
        let mut result = 0;
        let mut count = count;

        'outer: loop {
            // Fast Path
            while self.range >= 0x0100_0000 {
                if count == 0 {
                    break 'outer;
                }
                count -= 1;

                self.range >>= 1;
                let t = self.code.wrapping_sub(self.range) >> 31;
                self.code -= self.range & t.wrapping_sub(1);
                result = (result << 1) | (1 - t);
            }

            if count == 0 {
                break 'outer;
            }

            // Slow Path
            let b = self.inner.read_u8() as u32;
            self.code = (self.code << SHIFT_BITS) | b;
            self.range <<= SHIFT_BITS;
        }

        result as _
    }
}

pub(crate) struct RangeDecoderBuffer {
    buf: Vec<u8>,
    pos: usize,
}

impl RangeDecoder<RangeDecoderBuffer> {
    pub(crate) fn prepare<R: Read + ByteReader>(
        &mut self,
        mut reader: R,
        len: usize,
    ) -> Result<(), Error> {
        if len < 5 {
            return Err(error_invalid_input("buffer len must >= 5"));
        }

        let b = reader.read_u8()?;
        if b != 0x00 {
            return Err(error_invalid_input("first byte is 0"));
        }
        self.code = reader.read_u32_be()?;

        self.range = 0xFFFFFFFFu32;
        let len = len - 5;
        let pos = self.inner.buf.len() - len;
        let end = pos + len;
        self.inner.pos = pos;
        reader.read_exact(&mut self.inner.buf[pos..end])
    }

    #[inline]
    pub(crate) fn is_finished(&self) -> bool {
        self.inner.pos == self.inner.buf.len() && self.code == 0
    }
}

impl RangeDecoderBuffer {
    pub(crate) fn new(len: usize) -> Self {
        Self {
            buf: vec![0; len],
            pos: len,
        }
    }
}

pub(crate) trait RangeReader {
    fn read_u8(&mut self) -> u8;

    fn try_read_u8(&mut self) -> crate::Result<u8>;

    fn read_u32_be(&mut self) -> crate::Result<u32>;

    #[inline(always)]
    fn is_buffer(&self) -> bool {
        false
    }

    #[inline(always)]
    fn pos(&self) -> usize {
        unimplemented!("not a buffer reader")
    }

    #[inline(always)]
    fn set_pos(&mut self, _pos: usize) {
        unimplemented!("not a buffer reader")
    }

    #[inline(always)]
    fn buf(&self) -> &[u8] {
        unimplemented!("not a buffer reader")
    }
}

impl<T: Read> RangeReader for T {
    #[inline(always)]
    fn read_u8(&mut self) -> u8 {
        // Out of bound reads return an 1, which is fine, since the
        // LZMA reader will then throw a "dist overflow" error.
        // Not returning an error results in code that can be better
        // optimized in the hot path and overall 10% better decoding
        // performance.
        let mut buf = [0; 1];
        match self.read_exact(&mut buf) {
            Ok(_) => buf[0],
            Err(_) => 1,
        }
    }

    fn try_read_u8(&mut self) -> crate::Result<u8> {
        let mut buf = [0; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    #[inline(always)]
    fn read_u32_be(&mut self) -> crate::Result<u32> {
        let mut buf = [0; 4];
        self.read_exact(buf.as_mut())?;
        Ok(u32::from_be_bytes(buf))
    }
}

impl RangeReader for RangeDecoderBuffer {
    #[inline(always)]
    fn read_u8(&mut self) -> u8 {
        // Out of bound reads return an 1, which is fine, since the
        // LZMA reader will then throw a "dist overflow" error.
        // Not returning an error results in code that can be better
        // optimized in the hot path and overall 10% better decoding
        // performance.
        let byte = *self.buf.get(self.pos).unwrap_or(&1);
        self.pos += 1;
        byte
    }

    fn try_read_u8(&mut self) -> Result<u8, anyhow::Error> {
        self.buf
            .get(self.pos)
            .copied()
            .ok_or_else(error_eof)
            .map_err(anyhow::Error::from)
    }

    #[inline(always)]
    fn read_u32_be(&mut self) -> crate::Result<u32> {
        let array: [u8; 4] = self
            .buf
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| error_invalid_data("not enough data for reading u32 BE bytes"))?
            .try_into()
            .map_err(|_| error_other("slice doesn't match array size for u32 BE bytes"))?;
        let b = u32::from_be_bytes(array);
        self.pos += 4;
        Ok(b)
    }

    #[inline(always)]
    fn is_buffer(&self) -> bool {
        true
    }

    #[inline(always)]
    fn pos(&self) -> usize {
        self.pos
    }

    #[inline(always)]
    fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    #[inline(always)]
    fn buf(&self) -> &[u8] {
        self.buf.as_slice()
    }
}
