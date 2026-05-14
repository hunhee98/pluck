//! Tool description text shown to the agent at MCP handshake.
//!
//! Each description carries:
//!   - one-line summary
//!   - WHEN to use (concrete triggers)
//!   - WHY (token math, comparison vs cat/grep)
//!   - WHEN to fall back to Bash cat/grep
//!
//! These strings are the most important asset in the project — they decide
//! whether agents choose pluck or fall back to Bash.

#![allow(dead_code)]

pub const READ: &str = include_str!("../../../docs/mcp-descriptions/read.md");
pub const GREP: &str = include_str!("../../../docs/mcp-descriptions/grep.md");
pub const SEARCH: &str = include_str!("../../../docs/mcp-descriptions/search.md");
pub const SYMBOL: &str = include_str!("../../../docs/mcp-descriptions/symbol.md");
pub const PEEK: &str = include_str!("../../../docs/mcp-descriptions/peek.md");
pub const EXPAND: &str = include_str!("../../../docs/mcp-descriptions/expand.md");
