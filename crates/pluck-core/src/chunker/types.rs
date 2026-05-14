use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChunkKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Impl,
    Trait,
    Module,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub symbol: String,
    pub kind: ChunkKind,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: u32,
    pub end_byte: u32,
    pub content: String,
    pub signature: String,
}
