//! pluck MCP library — the same modules the `pluckd` binary uses,
//! re-exported so integration tests and benchmarks can drive the server
//! handler directly without a stdio round-trip.

pub mod server;
pub mod session;
