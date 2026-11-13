//! Zero-Stat Memory-Mapped Content-Addressed Storage Reader (`MappedCasArchive`)
//! and `SLICEEXE` Standalone Executable Packer.

use crate::archive::{CasBlobEntry, CasIndexHeader, CACHE_LINE_ALIGN, CAS_MAGIC, CAS_VERSION};
use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

/// Zero-Stat Memory-Mapped Content-Addressed Storage Archive (`mmap(MAP_SHARED)`).
pub struct MappedCasArchive {
    mmap: Arc<Mmap>,
    pub header: CasIndexHeader,
    pub(crate) data_start_offset: usize,
}

impl MappedCasArchive {
    /// Validates and constructs a `MappedCasArchive` from an already mapped `Arc<Mmap>` region
    /// spanning `cas_offset..cas_offset + cas_len`.
    ///
    /// Performs full bounds and UTF-8 validation on the CAS header and every `CasBlobEntry`
    /// once at open time so hot-path `get_source_slice` and `get_bytecode_slice` calls
    /// are guaranteed in-bounds and valid UTF-8 with zero runtime overhead.
    fn from_mmap_region(
        mmap: Arc<Mmap>,
        cas_offset: usize,
        cas_len: usize,
        display_label: &str,
    ) -> Result<Self> {
        let cas_end = cas_offset
            .checked_add(cas_len)
            .filter(|&end| end <= mmap.len() && cas_len >= 32)
            .with_context(|| format!("Invalid CAS archive offset/length in {}", display_label))?;

        let slice = &mmap[cas_offset..cas_end];
        if &slice[0..8] != CAS_MAGIC {
            bail!("Invalid SLICE_CAS magic header in {}", display_label);
        }

        let version = u32::from_le_bytes(slice[8..12].try_into()?);
        if version != CAS_VERSION {
            bail!("Unsupported SLICE_CAS version: {}", version);
        }

        let header_len = usize::try_from(u64::from_le_bytes(slice[16..24].try_into()?))
            .context("CAS header length overflow")?;
        let rel_data_start = usize::try_from(u64::from_le_bytes(slice[24..32].try_into()?))
            .context("CAS data_start_offset overflow")?;

        let header_end = 32usize
            .checked_add(header_len)
            .filter(|&end| end <= slice.len() && end <= rel_data_start)
            .with_context(|| format!("Corrupted or truncated CAS header in {}", display_label))?;

        let header: CasIndexHeader = serde_json::from_slice(&slice[32..header_end])
            .with_context(|| format!("Malformed CAS index JSON in {}", display_label))?;

        let data_start_offset = cas_offset
            .checked_add(rel_data_start)
            .filter(|&off| off <= cas_end)
            .with_context(|| {
                format!(
                    "Invalid CAS data_start_offset out of bounds in {}",
                    display_label
                )
            })?;

        for (hash_key, entry) in &header.blobs {
            let src_off =
                usize::try_from(entry.offset).context("CAS blob source offset overflow")?;
            let src_len =
                usize::try_from(entry.source_len).context("CAS blob source length overflow")?;
            let src_start = data_start_offset
                .checked_add(src_off)
                .context("CAS blob source start overflow")?;
            let src_end = src_start
                .checked_add(src_len)
                .filter(|&end| end <= cas_end)
                .with_context(|| {
                    format!(
                        "CAS blob '{}' source range [{}..{}] out of bounds (archive end: {})",
                        hash_key,
                        src_start,
                        src_start.saturating_add(src_len),
                        cas_end
                    )
                })?;

            if std::str::from_utf8(&mmap[src_start..src_end]).is_err() {
                bail!(
                    "CAS blob '{}' contains invalid UTF-8 in source slice",
                    hash_key
                );
            }

            let bc_off = usize::try_from(entry.bytecode_offset)
                .context("CAS blob bytecode offset overflow")?;
            let bc_len =
                usize::try_from(entry.bytecode_len).context("CAS blob bytecode length overflow")?;
            let bc_start = data_start_offset
                .checked_add(bc_off)
                .context("CAS blob bytecode start overflow")?;
            let _bc_end = bc_start
                .checked_add(bc_len)
                .filter(|&end| end <= cas_end)
                .with_context(|| {
                    format!(
                        "CAS blob '{}' bytecode range [{}..{}] out of bounds (archive end: {})",
                        hash_key,
                        bc_start,
                        bc_start.saturating_add(bc_len),
                        cas_end
                    )
                })?;
        }

        Ok(Self {
            mmap,
            header,
            data_start_offset,
        })
    }

    /// Opens and memory-maps a `.cas` archive file with zero heap copies of module payloads.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open CAS archive {}", path.display()))?;
        let mmap = unsafe { Mmap::map(&file)? };
        let len = mmap.len();
        Self::from_mmap_region(Arc::new(mmap), 0, len, &path.display().to_string())
    }

    /// Resolves an import specifier in O(1) time without touching the OS filesystem.
    #[inline]
    pub fn resolve_specifier(&self, specifier: &str) -> Option<&CasBlobEntry> {
        let hash = self
            .header
            .radix_resolution_map
            .get(specifier)
            .or_else(|| {
                let trimmed = specifier.trim_start_matches("./");
                self.header.radix_resolution_map.get(trimmed)
            })?;
        self.header.blobs.get(hash)
    }

    /// Returns a zero-copy slice to the normalized JavaScript source directly from the `mmap` region.
    #[inline]
    pub fn get_source_slice<'a>(&'a self, entry: &CasBlobEntry) -> &'a str {
        let Some(start) = self.data_start_offset.checked_add(entry.offset as usize) else {
            return "";
        };
        let Some(end) = start.checked_add(entry.source_len as usize) else {
            return "";
        };
        let Some(bytes) = self.mmap.get(start..end) else {
            return "";
        };
        std::str::from_utf8(bytes).unwrap_or("")
    }

    /// Returns a zero-copy slice to the pre-compiled bytecode directly from the `mmap` region.
    #[inline]
    pub fn get_bytecode_slice<'a>(&'a self, entry: &CasBlobEntry) -> &'a [u8] {
        let Some(start) = self
            .data_start_offset
            .checked_add(entry.bytecode_offset as usize)
        else {
            return &[];
        };
        let Some(end) = start.checked_add(entry.bytecode_len as usize) else {
            return &[];
        };
        self.mmap.get(start..end).unwrap_or(&[])
    }

    /// Returns a clone of the underlying `Arc<Mmap>` and the validated `(start, end)` byte range
    /// for `entry`'s bytecode/payload so callers can share zero-copy `mmap` slices across threads.
    #[inline]
    pub fn get_shared_bytecode_view(
        &self,
        entry: &CasBlobEntry,
    ) -> Option<(Arc<Mmap>, usize, usize)> {
        let start = self
            .data_start_offset
            .checked_add(entry.bytecode_offset as usize)?;
        let end = start.checked_add(entry.bytecode_len as usize)?;
        if end <= self.mmap.len() {
            Some((Arc::clone(&self.mmap), start, end))
        } else {
            None
        }
    }

    /// Returns the entrypoint blob entry.
    #[inline]
    pub fn entrypoint_blob(&self) -> Option<&CasBlobEntry> {
        self.header.blobs.get(&self.header.entrypoint_hash)
    }

    /// Returns the embedded `slice.toml` manifest string if one was packed into this `.cas` archive.
    #[inline]
    pub fn get_manifest_toml(&self) -> Option<&str> {
        let entry = self.resolve_specifier("manifest:/slice.toml")?;
        Some(self.get_source_slice(entry))
    }

    /// Reconstructs the `// #slice-source:` annotated JS source from `self.header.source_map_markers`
    /// in $O(L + K)$ time so `.cas` archives and standalone `SLICEEXE` binaries map uncaught errors
    /// back to exact TypeScript lines.
    pub fn reconstruct_annotated_source(&self, entry: &CasBlobEntry) -> String {
        let src = self.get_source_slice(entry);
        let markers = &self.header.source_map_markers;
        if markers.is_empty() || src.contains("// #slice-source: ") || src.contains("//@sm:") {
            return src.to_string();
        }
        let mut out = String::with_capacity(src.len() + markers.len() * 64);
        let is_monotonic = markers.windows(2).all(|w| w[0].0 <= w[1].0);
        if is_monotonic {
            let mut m_idx = 0usize;
            for (idx, line) in src.lines().enumerate() {
                let line_1 = idx + 1;
                while m_idx < markers.len() && markers[m_idx].0 < line_1 {
                    m_idx += 1;
                }
                while m_idx < markers.len() && markers[m_idx].0 == line_1 {
                    out.push_str("// #slice-source: ");
                    out.push_str(&markers[m_idx].1);
                    out.push('\n');
                    m_idx += 1;
                }
                out.push_str(line);
                out.push('\n');
            }
        } else {
            for (idx, line) in src.lines().enumerate() {
                let line_1 = idx + 1;
                for (m_line, marker_rest) in markers {
                    if *m_line == line_1 {
                        out.push_str("// #slice-source: ");
                        out.push_str(marker_rest);
                        out.push('\n');
                    }
                }
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    /// Opens a `.cas` archive embedded at `cas_offset` inside a larger file (such as a standalone executable).
    pub fn open_from_offset(path: &Path, cas_offset: usize, cas_len: usize) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open standalone binary {}", path.display()))?;
        let mmap = unsafe { Mmap::map(&file)? };
        Self::from_mmap_region(
            Arc::new(mmap),
            cas_offset,
            cas_len,
            &path.display().to_string(),
        )
    }

    /// Checks if the current running executable has an embedded `SLICEEXE` trailer and maps it in O(1)
    /// using a single `Mmap` call.
    pub fn open_embedded_self() -> Result<Option<Self>> {
        let exe_path = std::env::current_exe()?;
        let file = File::open(&exe_path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        if mmap.len() < 24 {
            return Ok(None);
        }
        let tail_start = mmap.len() - 24;
        if &mmap[tail_start + 16..tail_start + 24] != b"SLICEEXE" {
            return Ok(None);
        }
        let cas_offset = usize::try_from(u64::from_le_bytes(
            mmap[tail_start..tail_start + 8].try_into()?,
        ))
        .context("SLICEEXE cas_offset overflow")?;
        let cas_len = usize::try_from(u64::from_le_bytes(
            mmap[tail_start + 8..tail_start + 16].try_into()?,
        ))
        .context("SLICEEXE cas_len overflow")?;
        let archive = Self::from_mmap_region(
            Arc::new(mmap),
            cas_offset,
            cas_len,
            &exe_path.display().to_string(),
        )?;
        Ok(Some(archive))
    }
}

/// Embeds a compiled `.cas` archive into a copy of the `slice` runtime binary to produce a
/// zero-dependency standalone executable (`slice compile`).
///
/// Streams both the base runtime binary and the `.cas` archive through bounded 256 KB
/// `BufReader`/`BufWriter` buffers into an atomic sibling staging file so packing never
/// allocates multi-megabyte heap copies of the executable image.
pub fn embed_cas_into_executable(
    base_exe_path: &Path,
    cas_file_path: &Path,
    output_exe_path: &Path,
) -> Result<u64> {
    use std::io::{BufReader, BufWriter, Write};
    use std::sync::atomic::{AtomicU64, Ordering};

    static STAGING_SEQ: AtomicU64 = AtomicU64::new(0);

    let parent_dir = output_exe_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let out_name = output_exe_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("slice_out");
    let seq = STAGING_SEQ.fetch_add(1, Ordering::Relaxed);
    let staging_path = parent_dir.join(format!(
        ".{}.slice_compile_{}_{}.tmp",
        out_name,
        std::process::id(),
        seq
    ));

    let write_result = (|| -> Result<u64> {
        let base_file = std::fs::File::open(base_exe_path).with_context(|| {
            format!("Failed to open base executable {}", base_exe_path.display())
        })?;
        let cas_file = std::fs::File::open(cas_file_path)
            .with_context(|| format!("Failed to open CAS archive {}", cas_file_path.display()))?;
        let out_file = std::fs::File::create(&staging_path).with_context(|| {
            format!(
                "Failed to create staging executable {}",
                staging_path.display()
            )
        })?;

        let mut base_reader = BufReader::with_capacity(256 * 1024, base_file);
        let mut cas_reader = BufReader::with_capacity(256 * 1024, cas_file);
        let mut writer = BufWriter::with_capacity(256 * 1024, out_file);

        let exe_len = std::io::copy(&mut base_reader, &mut writer).with_context(|| {
            format!(
                "Failed to stream base executable {}",
                base_exe_path.display()
            )
        })?;

        let rem = (exe_len as usize) % CACHE_LINE_ALIGN;
        let pad_len = if rem != 0 { CACHE_LINE_ALIGN - rem } else { 0 };
        if pad_len > 0 {
            let zero_pad = [0u8; CACHE_LINE_ALIGN];
            writer.write_all(&zero_pad[..pad_len])?;
        }

        let cas_offset = exe_len + (pad_len as u64);
        let cas_len = std::io::copy(&mut cas_reader, &mut writer)
            .with_context(|| format!("Failed to stream CAS archive {}", cas_file_path.display()))?;

        writer.write_all(&cas_offset.to_le_bytes())?;
        writer.write_all(&cas_len.to_le_bytes())?;
        writer.write_all(b"SLICEEXE")?;
        writer.flush()?;

        let inner_file = writer.into_inner().map_err(|e| e.into_error())?;
        inner_file.sync_data()?;
        Ok(cas_offset + cas_len + 24)
    })();

    let total_len = match write_result {
        Ok(n) => n,
        Err(err) => {
            let _ = std::fs::remove_file(&staging_path);
            return Err(err);
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&staging_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&staging_path, perms)?;
    }

    if let Err(_first_err) = std::fs::rename(&staging_path, output_exe_path) {
        if output_exe_path.exists() {
            let _ = std::fs::remove_file(output_exe_path);
        }
        if let Err(rename_err) = std::fs::rename(&staging_path, output_exe_path) {
            let _ = std::fs::remove_file(&staging_path);
            return Err(rename_err).with_context(|| {
                format!(
                    "Failed to finalize standalone executable at {}",
                    output_exe_path.display()
                )
            });
        }
    }

    Ok(total_len)
}
