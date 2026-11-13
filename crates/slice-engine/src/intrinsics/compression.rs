//! Self-Contained RFC 1951 (DEFLATE), RFC 1950 (Zlib), & RFC 1952 (Gzip) Compression Engine
//! with Compile-Time CRC-32 Lookup Table, O(1) 9-Bit Primary Huffman Decode LUT, and Sandbox Memory Caging.

const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const FAST_HUFFMAN_BITS: usize = 9;
const FAST_HUFFMAN_SIZE: usize = 1 << FAST_HUFFMAN_BITS;

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

const fn build_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if (c & 1) != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const CRC32_TABLE: [u32; 256] = build_crc32_table();

#[inline]
pub fn adler32_checksum(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= MOD_ADLER;
        b %= MOD_ADLER;
    }
    (b << 16) | a
}

#[inline]
pub fn crc32_checksum(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        let idx = ((crc as u8) ^ byte) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[idx];
    }
    !crc
}

#[inline]
fn effective_compression_mem_budget() -> usize {
    crate::boa_realm::BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        if limit > 0 && limit < usize::MAX {
            limit.saturating_sub(used).min(MAX_DECOMPRESSED_BYTES)
        } else {
            MAX_DECOMPRESSED_BYTES
        }
    })
}

#[inline]
fn charge_inflate_fuel(ticks: u64) -> Result<(), &'static str> {
    let exceeded = crate::boa_realm::BOA_FUEL_BUDGET.with(|f| {
        let (consumed, limit) = f.get();
        if limit == u64::MAX {
            return None;
        }
        let next = consumed.saturating_add(ticks);
        f.set((next, limit));
        if next > limit {
            Some(limit)
        } else {
            None
        }
    });
    if let Some(limit) = exceeded {
        let tenant_id = crate::boa_realm::BOA_ACTIVE_TENANT.with(|t| {
            let b = t.borrow();
            if b.is_empty() {
                "boa-tenant".to_string()
            } else {
                b.clone()
            }
        });
        crate::boa_realm::BOA_VIOLATION.with(|v| {
            if v.borrow().is_none() {
                *v.borrow_mut() =
                    Some(slice_sandbox::SandboxViolation::CpuFuelExhausted { tenant_id, limit });
            }
        });
        return Err("CPU fuel budget exhausted during DEFLATE decompression");
    }
    Ok(())
}

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

    let mut head = [usize::MAX; 4096];
    let win_len = input.len().next_power_of_two().clamp(2, 32768);
    let mut prev = vec![usize::MAX; win_len];
    let mask = win_len - 1;
    let mut pos = 0usize;

    while pos < input.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;

        if pos + 3 <= input.len() {
            let h = lz77_hash3(input, pos);
            let mut cand = head[h];
            head[h] = pos;
            if !prev.is_empty() {
                prev[pos & mask] = cand;
            }

            let max_len = (input.len() - pos).min(258);
            let mut chain_steps = 0usize;
            while cand != usize::MAX && pos > cand && (pos - cand) <= 32768 && chain_steps < 24 {
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
                if prev.is_empty() {
                    break;
                }
                let next_cand = prev[cand & mask];
                if next_cand >= cand {
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
                if pos + 3 <= input.len() {
                    let h = lz77_hash3(input, pos);
                    let prev_head = head[h];
                    head[h] = pos;
                    if !prev.is_empty() {
                        prev[pos & mask] = prev_head;
                    }
                }
                pos += 1;
            }
        } else {
            bw.write_fixed_sym(input[pos] as u16);
            pos += 1;
        }
    }

    bw.write_fixed_sym(256);
    bw.finish()
}

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

/// Compresses `input` using `"gzip"` (RFC 1952), `"deflate"` (RFC 1950 zlib), or `"deflate-raw"` (RFC 1951).
pub fn zlib_compress_bytes(input: &[u8], format: &str) -> Result<Vec<u8>, &'static str> {
    if input.len() > effective_compression_mem_budget() {
        return Err("Compression input exceeds sandbox memory limit");
    }
    let raw = deflate_raw_compress(input);
    match format {
        "deflate-raw" => Ok(raw),
        "deflate" => {
            let mut out = Vec::with_capacity(raw.len() + 6);
            out.extend_from_slice(&[0x78, 0x9C]);
            out.extend_from_slice(&raw);
            let adler = adler32_checksum(input);
            out.extend_from_slice(&adler.to_be_bytes());
            Ok(out)
        }
        "gzip" => {
            let mut out = Vec::with_capacity(raw.len() + 18);
            out.extend_from_slice(&[0x1F, 0x8B, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xFF]);
            out.extend_from_slice(&raw);
            let crc = crc32_checksum(input);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(input.len() as u32).to_le_bytes());
            Ok(out)
        }
        _ => Err("Unsupported compression format"),
    }
}

/// Decompresses `input` using `"gzip"` (RFC 1952), `"deflate"` (RFC 1950 zlib), or `"deflate-raw"` (RFC 1951).
pub fn zlib_decompress_bytes(input: &[u8], format: &str) -> Result<Vec<u8>, &'static str> {
    match format {
        "deflate-raw" => inflate_raw_decompress(input),
        "deflate" => {
            if input.len() < 6 {
                return Err("Truncated zlib stream");
            }
            let cmf = input[0];
            let flg = input[1];
            if (cmf & 0x0F) != 8 || !((cmf as u16) << 8 | (flg as u16)).is_multiple_of(31) {
                return Err("Invalid zlib header");
            }
            let start = if (flg & 0x20) != 0 { 6 } else { 2 };
            if input.len() < start + 4 {
                return Err("Truncated zlib stream");
            }
            let end = input.len() - 4;
            let out = inflate_raw_decompress(&input[start..end])?;
            let expected_adler =
                u32::from_be_bytes([input[end], input[end + 1], input[end + 2], input[end + 3]]);
            if adler32_checksum(&out) != expected_adler {
                return Err("Zlib Adler-32 checksum mismatch");
            }
            Ok(out)
        }
        "gzip" => {
            if input.len() < 18 {
                return Err("Truncated gzip stream");
            }
            if input[0] != 0x1F || input[1] != 0x8B || input[2] != 0x08 {
                return Err("Invalid gzip header magic");
            }
            let flg = input[3];
            let end = input.len() - 8;
            let mut pos = 10usize;
            if (flg & 0x04) != 0 {
                if pos + 2 > end {
                    return Err("Truncated gzip FEXTRA header");
                }
                let xlen = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
                pos = pos
                    .checked_add(2 + xlen)
                    .ok_or("Truncated gzip FEXTRA header")?;
                if pos > end {
                    return Err("Truncated gzip FEXTRA header");
                }
            }
            if (flg & 0x08) != 0 {
                let nul_off = input
                    .get(pos..end)
                    .and_then(|s| memchr::memchr(0, s))
                    .ok_or("Truncated gzip FNAME header")?;
                pos += nul_off + 1;
            }
            if (flg & 0x10) != 0 {
                let nul_off = input
                    .get(pos..end)
                    .and_then(|s| memchr::memchr(0, s))
                    .ok_or("Truncated gzip FCOMMENT header")?;
                pos += nul_off + 1;
            }
            if (flg & 0x02) != 0 {
                if pos + 2 > end {
                    return Err("Truncated gzip FHCRC header");
                }
                pos += 2;
            }
            if pos > end {
                return Err("Truncated gzip trailer");
            }
            let out = inflate_raw_decompress(&input[pos..end])?;
            let expected_crc =
                u32::from_le_bytes([input[end], input[end + 1], input[end + 2], input[end + 3]]);
            let expected_isize = u32::from_le_bytes([
                input[end + 4],
                input[end + 5],
                input[end + 6],
                input[end + 7],
            ]);
            if crc32_checksum(&out) != expected_crc || (out.len() as u32) != expected_isize {
                return Err("Gzip CRC-32 or ISIZE trailer mismatch");
            }
            Ok(out)
        }
        _ => Err("Unsupported decompression format"),
    }
}
