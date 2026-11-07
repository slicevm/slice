## Summary

<!-- Briefly describe the changes in this Pull Request and link any related issue. -->

## Subsystem

- [ ] `slice-cli`
- [ ] `slice-engine` (`bindings`, `intrinsics`, `isolate`, `vm`, `runtime`, `net`, `preludes`)
- [ ] `slice-cas`
- [ ] `slice-sandbox`
- [ ] `slice-io`
- [ ] Documentation / Benchmarks / CI

## Verification & Security Checklist

- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes with zero warnings
- [ ] New host bindings or I/O paths enforce Object-Capability (`CapabilitySet`) checks, sandbox memory limits, and Egress DLP (`check_host_egress_dlp`) where applicable
- [ ] Unit or integration tests added for new functionality or bug fixes
