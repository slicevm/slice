//! Attenuated Virtual Filesystem (VFS) Path Normalization & Directory-Boundary OCap Verification.

mod guard;
mod normalize;
mod prefix;

pub use crate::compartment::{exec_binary_matches, has_exec_shell_metachar};
pub(crate) use guard::has_dotdot_segment;
pub use guard::is_rejected_vfs_candidate;
pub use normalize::{
    anchor_vfs_path_to_cwd, cached_normalized_cwd, is_abs_normalized_vfs_path,
    normalize_vfs_path_cow, normalize_vfs_path_lexical,
};
#[cfg(test)]
pub(crate) use prefix::{
    CachedOsTmpInfo, build_os_tmp_info, vfs_normalized_candidate_matches_prefix_with_tmp,
};
pub use prefix::{
    vfs_normalized_candidate_matches_prefix, vfs_path_matches_any_prefix, vfs_prefix_matches,
};
