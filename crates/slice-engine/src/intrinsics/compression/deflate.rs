//! RFC 1951 `DEFLATE` LZ77 + Fixed Huffman Bitstream Compressor.

use super::{DIST_BASE, DIST_EXTRA, LENGTH_BASE, LENGTH_EXTRA};

struct DeflateBitWriter {
    out: Vec<u8>,
    bit_buf: u32,
    bit_count: u8,
}

impl DeflateBitWriter {
    #[inline]
    fn with_capacity(cap: usize) -> Self {
        Self {
            out: Vec::with_capacity(cap),
            bit_buf: 0,
            bit_count: 0,
        }
    }

    #[inline]
    fn write_bits_lsb(&mut self, val: u32, nbits: u8) {
        self.bit_buf |= (val & ((1u32 << nbits) - 1)) << self.bit_count;
        self.bit_count += nbits;
        while self.bit_count >= 8 {
            self.out.push((self.bit_buf & 0xFF) as u8);
            self.bit_buf >>= 8;
            self.bit_count -= 8;
        }
    }

    #[inline]
    fn write_huffman_msb(&mut self, code: u16, nbits: u8) {
        let rev = (code.reverse_bits() >> (16 - nbits)) as u32;
        self.write_bits_lsb(rev, nbits);
    }

    #[inline]
    fn write_fixed_sym(&mut self, sym: u16) {
        match sym {
            0..=143 => self.write_huffman_msb(0x30 + sym, 8),
            144..=255 => self.write_huffman_msb(0x190 + (sym - 144), 9),
            256..=279 => self.write_huffman_msb(sym - 256, 7),
            _ => self.write_huffman_msb(0xC0 + (sym - 280), 8),
        }
    }

    #[inline]
    fn write_length_dist(&mut self, length: usize, dist: usize) {
        let mut l_idx = 0usize;
        for (i, &base) in LENGTH_BASE.iter().enumerate() {
            if length >= base as usize {
                l_idx = i;
            } else {
                break;
            }
        }
        self.write_fixed_sym(257 + l_idx as u16);
        let l_extra = LENGTH_EXTRA[l_idx];
        if l_extra > 0 {
            self.write_bits_lsb((length - LENGTH_BASE[l_idx] as usize) as u32, l_extra);
        }

        let mut d_idx = 0usize;
        for (i, &base) in DIST_BASE.iter().enumerate() {
            if dist >= base as usize {
                d_idx = i;
            } else {
                break;
            }
        }
        self.write_huffman_msb(d_idx as u16, 5);
        let d_extra = DIST_EXTRA[d_idx];
        if d_extra > 0 {
            self.write_bits_lsb((dist - DIST_BASE[d_idx] as usize) as u32, d_extra);
        }
    }

    #[inline]
    fn finish(mut self) -> Vec<u8> {
        if self.bit_count > 0 {
            self.out.push((self.bit_buf & 0xFF) as u8);
        }
        self.out
    }
}

#[inline]
fn lz77_hash3(bytes: &[u8], pos: usize) -> usize {
    let v = (bytes[pos] as u32) | ((bytes[pos + 1] as u32) << 8) | ((bytes[pos + 2] as u32) << 16);
    ((v.wrapping_mul(0x1E35_A7BD)) >> 20) as usize & 0xFFF
}

/// Compresses `input` into a raw RFC 1951 DEFLATE bitstream using LZ77 + Fixed Huffman encoding.
pub(crate) fn deflate_raw_compress(input: &[u8]) -> Vec<u8> {
    let mut bw = DeflateBitWriter::with_capacity(input.len() / 2 + 16);
    // BFINAL = 1, BTYPE = 01 (Fixed Huffman)
    bw.write_bits_lsb(1, 1);
    bw.write_bits_lsb(1, 2);

    if input.len() < 3 {
        for &b in input {
            bw.write_fixed_sym(b as u16);
        }
        bw.write_fixed_sym(256);
        return bw.finish();
    }

    struct DeflateScratch {
        head: [u32; 4096],
        prev: [u32; 32768],
    }

    thread_local! {
        static DEFLATE_SCRATCH: std::cell::RefCell<Box<DeflateScratch>> = std::cell::RefCell::new(
            Box::new(DeflateScratch {
                head: [u32::MAX; 4096],
                prev: [u32::MAX; 32768],
            }),
        );
    }

    const MASK: usize = 32767;
    DEFLATE_SCRATCH.with(|cell| {
        let mut fallback;
        let scratch: &mut DeflateScratch = match cell.try_borrow_mut() {
            Ok(mut guard) => {
                guard.head.fill(u32::MAX);
                return compress_lz77_with_scratch(input, &mut bw, &mut guard);
            }
            Err(_) => {
                fallback = Box::new(DeflateScratch {
                    head: [u32::MAX; 4096],
                    prev: [u32::MAX; 32768],
                });
                &mut fallback
            }
        };
        compress_lz77_with_scratch(input, &mut bw, scratch)
    });

    fn compress_lz77_with_scratch(
        input: &[u8],
        bw: &mut DeflateBitWriter,
        scratch: &mut DeflateScratch,
    ) {
        let mut pos = 0usize;
        while pos < input.len() {
            let mut best_len = 0usize;
            let mut best_dist = 0usize;

            if pos + 3 <= input.len() && pos < u32::MAX as usize {
                let h = lz77_hash3(input, pos);
                let mut cand = scratch.head[h] as usize;
                scratch.head[h] = pos as u32;
                scratch.prev[pos & MASK] = cand as u32;

                let max_len = (input.len() - pos).min(258);
                let mut chain_steps = 0usize;
                while cand != u32::MAX as usize
                    && pos > cand
                    && (pos - cand) <= 32768
                    && chain_steps < 24
                {
                    chain_steps += 1;
                    if input[cand + best_len] == input[pos + best_len]
                        && input[cand..cand + 3] == input[pos..pos + 3]
                    {
                        let mut mlen = 3usize;
                        while mlen < max_len && input[cand + mlen] == input[pos + mlen] {
                            mlen += 1;
                        }
                        if mlen > best_len {
                            best_len = mlen;
                            best_dist = pos - cand;
                            if best_len == max_len {
                                break;
                            }
                        }
                    }
                    let next_cand = scratch.prev[cand & MASK] as usize;
                    if next_cand == u32::MAX as usize || next_cand >= cand {
                        break;
                    }
                    cand = next_cand;
                }
            }

            if best_len >= 3 {
                bw.write_length_dist(best_len, best_dist);
                let step_end = (pos + best_len).min(input.len());
                pos += 1;
                while pos < step_end {
                    if pos + 3 <= input.len() && pos < u32::MAX as usize {
                        let h = lz77_hash3(input, pos);
                        let prev_head = scratch.head[h];
                        scratch.head[h] = pos as u32;
                        scratch.prev[pos & MASK] = prev_head;
                    }
                    pos += 1;
                }
            } else {
                bw.write_fixed_sym(input[pos] as u16);
                pos += 1;
            }
        }
    }

    bw.write_fixed_sym(256);
    bw.finish()
}
