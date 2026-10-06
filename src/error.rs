use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EngineError {

    #[error("Unexpected end of file / buffer")]
    UnexpectedEof,

    #[error("checksum mismatch: data corrupted")]
    InvalidChecksum,
    
    #[error("I/O error: {0}")]
    Io(#[from] io::Error), // automatically converts std::io::Error into EngineError!
}

pub type Result<T> = std::result::Result<T, EngineError>;