pub(crate) mod doctor_and_bench;
pub(crate) mod mcp;
pub(crate) mod repl;

pub(crate) use doctor_and_bench::{cmd_doctor, cmd_microvm_bench, cmd_verify_security};
#[cfg(test)]
pub(crate) use doctor_and_bench::{
    measure_current_process_rss_kb, run_security_containment_checks,
};
pub(crate) use mcp::cmd_mcp;
#[cfg(test)]
pub(crate) use mcp::{handle_mcp_jsonrpc_line, MAX_MCP_LINE_BYTES};
pub(crate) use repl::cmd_repl;
