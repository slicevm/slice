//! RFC 1951 `DEFLATE` Decompressor (`inflate`) with 9-Bit Primary Huffman LUT and Sandbox Fuel/Memory Caging.

use super::{
    DIST_BASE, DIST_EXTRA, LENGTH_BASE, LENGTH_EXTRA, charge_inflate_fuel,
    effective_compression_mem_budget,
};

const FAST_HUFFMAN_BITS: usize = 9;
const FAST_HUFFMAN_SIZE: usize = 1 << FAST_HUFFMAN_BITS;
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

struct HuffmanTree {
    counts: [u16; 16],
    symbols: Vec<u16>,
    /// Primary 9-bit LSB lookup table: `(bit_len << 12) | sym` when `bit_len in 1..=9`, else `0`.
    fast_lut: [u16; FAST_HUFFMAN_SIZE],
}

impl HuffmanTree {
    fn from_code_lengths(lengths: &[u8]) -> Result<Self, &'static str> {
        let mut counts = [0u16; 16];
        for &len in lengths {
            if len as usize >= 16 {
                return Err("Invalid Huffman code length");
            }
            if len > 0 {
                counts[len as usize] += 1;
            }
        }
        let mut offsets = [0u16; 16];
        let mut total = 0u16;
        for bits in 1..16 {
            offsets[bits] = total;
            total += counts[bits];
        }
        let mut next_code = [0u16; 16];
        let mut code = 0u16;
        for bits in 1..16 {
            code = (code + counts[bits - 1]) << 1;
            next_code[bits] = code;
        }

        let mut symbols = vec![0u16; total as usize];
        let mut fast_lut = [0u16; FAST_HUFFMAN_SIZE];
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                let len_usize = len as usize;
                let idx = offsets[len_usize] as usize;
                offsets[len_usize] += 1;
                symbols[idx] = sym as u16;

                let msb_code = next_code[len_usize];
                next_code[len_usize] = msb_code.wrapping_add(1);
                if len_usize <= FAST_HUFFMAN_BITS {
                    let lsb_code = (msb_code.reverse_bits() >> (16 - len_usize)) as usize;
                    let entry = ((len as u16) << 12) | (sym as u16);
                    let step = 1usize << len_usize;
                    let mut slot = lsb_code;
                    while slot < FAST_HUFFMAN_SIZE {
                        fast_lut[slot] = entry;
                        slot += step;
                    }
                }
            }
        }
        Ok(Self {
            counts,
            symbols,
            fast_lut,
        })
    }
}

fn fixed_huffman_trees() -> &'static (HuffmanTree, HuffmanTree) {
    static FIXED_TREES: std::sync::OnceLock<(HuffmanTree, HuffmanTree)> =
        std::sync::OnceLock::new();
    FIXED_TREES.get_or_init(|| {
        let mut lit_lengths = [0u8; 288];
        lit_lengths[0..144].fill(8);
        lit_lengths[144..256].fill(9);
        lit_lengths[256..280].fill(7);
        lit_lengths[280..288].fill(8);
        let dist_lengths = [5u8; 32];
        (
            HuffmanTree::from_code_lengths(&lit_lengths).expect("fixed lit tree"),
            HuffmanTree::from_code_lengths(&dist_lengths).expect("fixed dist tree"),
        )
    })
}

struct InflateBitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_buf: u32,
    bit_count: u8,
}

impl<'a> InflateBitReader<'a> {
    #[inline]
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_buf: 0,
            bit_count: 0,
        }
    }

    #[inline]
    fn refill_up_to_24(&mut self) {
        while self.bit_count <= 24 && self.byte_pos < self.data.len() {
            self.bit_buf |= (self.data[self.byte_pos] as u32) << self.bit_count;
            self.byte_pos += 1;
            self.bit_count += 8;
        }
    }

    #[inline]
    fn ensure_bits(&mut self, n: u8) -> Result<(), &'static str> {
        while self.bit_count < n {
            if self.byte_pos >= self.data.len() {
                return Err("Unexpected end of DEFLATE stream");
            }
            self.bit_buf |= (self.data[self.byte_pos] as u32) << self.bit_count;
            self.byte_pos += 1;
            self.bit_count += 8;
        }
        Ok(())
    }

    #[inline]
    fn read_bits(&mut self, n: u8) -> Result<u32, &'static str> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure_bits(n)?;
        let val = self.bit_buf & ((1u32 << n) - 1);
        self.bit_buf >>= n;
        self.bit_count -= n;
        Ok(val)
    }

    #[inline]
    fn align_byte(&mut self) {
        let drop_bits = self.bit_count & 7;
        self.bit_buf >>= drop_bits;
        self.bit_count -= drop_bits;
    }

    #[inline]
    fn read_u16_aligned(&mut self) -> Result<u16, &'static str> {
        let lo = self.read_bits(8)? as u16;
        let hi = self.read_bits(8)? as u16;
        Ok(lo | (hi << 8))
    }

    #[inline]
    fn decode_huffman(&mut self, tree: &HuffmanTree) -> Result<u16, &'static str> {
        if self.bit_count < FAST_HUFFMAN_BITS as u8 {
            self.refill_up_to_24();
        }
        if self.bit_count >= FAST_HUFFMAN_BITS as u8 {
            let entry = tree.fast_lut[(self.bit_buf as usize) & (FAST_HUFFMAN_SIZE - 1)];
            if entry != 0 {
                let len = (entry >> 12) as u8;
                self.bit_buf >>= len;
                self.bit_count -= len;
                return Ok(entry & 0x0FFF);
            }
        }
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= self.read_bits(1)? as i32;
            let count = tree.counts[len] as i32;
            if code - count < first {
                return tree
                    .symbols
                    .get((index + (code - first)) as usize)
                    .copied()
                    .ok_or("Corrupt Huffman symbol table");
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("Invalid Huffman code in DEFLATE stream")
    }
}

/// Decompresses a raw RFC 1951 DEFLATE stream (`BTYPE = 00`, `01`, or `10`).
pub(crate) fn inflate_raw_decompress(input: &[u8]) -> Result<Vec<u8>, &'static str> {
    let effective_max = effective_compression_mem_budget();
    let mut consecutive_empty_blocks = 0usize;
    let mut total_empty_blocks = 0usize;
    let mut symbol_steps = 0u32;
    let mut br = InflateBitReader::new(input);
    let mut out = Vec::with_capacity(input.len().saturating_mul(2).min(65536).min(effective_max));

    loop {
        let out_before_block = out.len();
        let bfinal = br.read_bits(1)?;
        let btype = br.read_bits(2)?;
        match btype {
            0 => {
                charge_inflate_fuel(4)?;
                br.align_byte();
                let len = br.read_u16_aligned()?;
                let nlen = br.read_u16_aligned()?;
                if len != !nlen {
                    return Err("Corrupt stored DEFLATE block length complement");
                }
                if out.len().saturating_add(len as usize) > effective_max {
                    return Err("Decompressed payload exceeds sandbox memory limit");
                }
                if len > 0 {
                    charge_inflate_fuel(((len as u64) / 512).max(1))?;
                }
                for _ in 0..len {
                    out.push(br.read_bits(8)? as u8);
                }
            }
            1 | 2 => {
                let dynamic_trees: Option<(HuffmanTree, HuffmanTree)>;
                let (lit_tree, dist_tree) = if btype == 1 {
                    charge_inflate_fuel(4)?;
                    let (lt, dt) = fixed_huffman_trees();
                    (lt, dt)
                } else {
                    charge_inflate_fuel(24)?;
                    let hlit = br.read_bits(5)? as usize + 257;
                    let hdist = br.read_bits(5)? as usize + 1;
                    let hclen = br.read_bits(4)? as usize + 4;
                    if hlit > 286 || hdist > 30 {
                        return Err("Invalid dynamic Huffman table header");
                    }
                    let mut cl_lengths = [0u8; 19];
                    for i in 0..hclen {
                        cl_lengths[CL_ORDER[i]] = br.read_bits(3)? as u8;
                    }
                    let cl_tree = HuffmanTree::from_code_lengths(&cl_lengths)?;
                    let total_codes = hlit + hdist;
                    let mut unpacked = Vec::with_capacity(total_codes);
                    while unpacked.len() < total_codes {
                        let sym = br.decode_huffman(&cl_tree)?;
                        match sym {
                            0..=15 => unpacked.push(sym as u8),
                            16 => {
                                let prev = *unpacked
                                    .last()
                                    .ok_or("Invalid repeat code 16 at start of table")?;
                                let repeat = br.read_bits(2)? as usize + 3;
                                if unpacked.len() + repeat > total_codes {
                                    return Err("Dynamic Huffman code lengths overflow");
                                }
                                unpacked.resize(unpacked.len() + repeat, prev);
                            }
                            17 => {
                                let repeat = br.read_bits(3)? as usize + 3;
                                if unpacked.len() + repeat > total_codes {
                                    return Err("Dynamic Huffman code lengths overflow");
                                }
                                unpacked.resize(unpacked.len() + repeat, 0);
                            }
                            18 => {
                                let repeat = br.read_bits(7)? as usize + 11;
                                if unpacked.len() + repeat > total_codes {
                                    return Err("Dynamic Huffman code lengths overflow");
                                }
                                unpacked.resize(unpacked.len() + repeat, 0);
                            }
                            _ => return Err("Invalid code length symbol"),
                        }
                    }
                    dynamic_trees = Some((
                        HuffmanTree::from_code_lengths(&unpacked[..hlit])?,
                        HuffmanTree::from_code_lengths(&unpacked[hlit..])?,
                    ));
                    let pair = dynamic_trees.as_ref().unwrap();
                    (&pair.0, &pair.1)
                };

                loop {
                    symbol_steps = symbol_steps.wrapping_add(1);
                    if (symbol_steps & 0xFF) == 0 {
                        charge_inflate_fuel(2)?;
                    }
                    let sym = br.decode_huffman(lit_tree)?;
                    if sym < 256 {
                        if out.len() >= effective_max {
                            return Err("Decompressed payload exceeds sandbox memory limit");
                        }
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let l_idx = (sym - 257) as usize;
                        if l_idx >= LENGTH_BASE.len() {
                            return Err("Invalid DEFLATE length code");
                        }
                        let length = LENGTH_BASE[l_idx] as usize
                            + br.read_bits(LENGTH_EXTRA[l_idx])? as usize;
                        let d_sym = br.decode_huffman(dist_tree)? as usize;
                        if d_sym >= DIST_BASE.len() {
                            return Err("Invalid DEFLATE distance code");
                        }
                        let dist =
                            DIST_BASE[d_sym] as usize + br.read_bits(DIST_EXTRA[d_sym])? as usize;
                        if dist == 0 || dist > out.len() {
                            return Err("DEFLATE back-reference distance out of bounds");
                        }
                        if out.len().saturating_add(length) > effective_max {
                            return Err("Decompressed payload exceeds sandbox memory limit");
                        }
                        if dist == 1 {
                            let fill = out[out.len() - 1];
                            out.resize(out.len() + length, fill);
                        } else if dist >= length {
                            let start = out.len() - dist;
                            out.extend_from_within(start..start + length);
                        } else {
                            for _ in 0..length {
                                let b = out[out.len() - dist];
                                out.push(b);
                            }
                        }
                    }
                }
            }
            _ => return Err("Invalid DEFLATE block type (BTYPE=11)"),
        }

        if out.len() == out_before_block {
            consecutive_empty_blocks += 1;
            total_empty_blocks += 1;
            if consecutive_empty_blocks > 64 || total_empty_blocks > 1024 {
                return Err("DEFLATE stream exceeds empty block limit");
            }
        } else {
            consecutive_empty_blocks = 0;
        }

        if bfinal != 0 {
            break;
        }
    }

    Ok(out)
}
