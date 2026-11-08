use std::path::{Path, PathBuf};

/// Maximum directory recursion depth when computing watch fingerprints (`CLI-SEC-03`).
pub(crate) const MAX_WATCH_WALK_DEPTH: usize = 64;

/// Computes a deterministic 64-bit metadata fingerprint over watched source files under `target`,
/// skipping symlinks and bounding directory recursion to `MAX_WATCH_WALK_DEPTH`.
pub(crate) fn collect_watch_fingerprint(target: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();

    fn hash_file_meta(file_path: &Path, hasher: &mut DefaultHasher) {
        if let Ok(meta) = std::fs::symlink_metadata(file_path) {
            if meta.file_type().is_symlink() {
                return;
            }
            file_path.to_string_lossy().hash(hasher);
            meta.len().hash(hasher);
            if let Ok(m) = meta.modified() {
                if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                    d.as_nanos().hash(hasher);
                }
            }
        }
    }

    fn visit_watch(dir: &Path, depth: usize, hasher: &mut DefaultHasher) {
        if depth > MAX_WATCH_WALK_DEPTH {
            return;
        }
        let Ok(meta) = std::fs::symlink_metadata(dir) else {
            return;
        };
        let ft = meta.file_type();
        if ft.is_symlink() {
            return;
        }
        if ft.is_file() {
            hash_file_meta(dir, hasher);
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<(PathBuf, std::fs::FileType)> = rd
            .flatten()
            .filter_map(|e| e.file_type().ok().map(|t| (e.path(), t)))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (p, entry_ft) in entries {
            if entry_ft.is_symlink() {
                continue;
            }
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
            if matches!(name, "node_modules" | "target" | ".git" | ".archive") {
                continue;
            }
            if entry_ft.is_dir() {
                visit_watch(&p, depth + 1, hasher);
            } else if entry_ft.is_file() {
                let ext = p.extension().and_then(|s| s.to_str()).unwrap_or_default();
                if matches!(
                    ext,
                    "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "json" | "toml" | "env" | "wasm"
                ) || name.starts_with(".env")
                {
                    hash_file_meta(&p, hasher);
                }
            }
        }
    }

    visit_watch(target, 0, &mut hasher);
    hasher.finish()
}

/// OS-native filesystem watcher backed by `notify` (`inotify` on Linux, `ReadDirectoryChangesW` on Windows,
/// `FSEvents`/`kqueue` on macOS) with a 15ms debounce window and automatic fingerprint fallback.
pub(crate) struct OsNativeFileWatcher {
    target: PathBuf,
    prev_fp: u64,
    _watcher: Option<notify::RecommendedWatcher>,
    rx: std::sync::mpsc::Receiver<notify::Event>,
}

impl OsNativeFileWatcher {
    pub(crate) fn new(target: &Path) -> Self {
        use notify::{RecursiveMode, Watcher};
        let prev_fp = collect_watch_fingerprint(target);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher_opt =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(ev) = res {
                    let _ = tx.send(ev);
                }
            })
            .ok();
        if let Some(ref mut w) = watcher_opt {
            let watch_root = if target.is_dir() {
                target
            } else {
                target.parent().unwrap_or(target)
            };
            let _ = w.watch(watch_root, RecursiveMode::Recursive);
        }
        Self {
            target: target.to_path_buf(),
            prev_fp,
            _watcher: watcher_opt,
            rx,
        }
    }

    /// Blocks on the OS kernel event channel (with zero polling CPU when idle) or until `watch_once`
    /// deadline expires, debouncing rapid save bursts (15 ms) and confirming fingerprint change.
    pub(crate) fn wait_for_change(&mut self, watch_once: bool) -> bool {
        use std::time::{Duration, Instant};
        let wait_start = Instant::now();
        let timeout = if watch_once {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(500)
        };
        loop {
            let _ = self.rx.recv_timeout(timeout);
            // Drain any coalesced save events within a 15ms debounce window
            let debounce_deadline = Instant::now() + Duration::from_millis(15);
            while self
                .rx
                .recv_timeout(debounce_deadline.saturating_duration_since(Instant::now()))
                .is_ok()
            {}
            let cur_fp = collect_watch_fingerprint(&self.target);
            if cur_fp != self.prev_fp || (watch_once && wait_start.elapsed().as_millis() >= 250) {
                self.prev_fp = cur_fp;
                return true;
            }
        }
    }
}
