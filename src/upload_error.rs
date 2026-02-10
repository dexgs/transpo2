use crate::files::WriterError;

use std::io::{Error, ErrorKind};


#[derive(Debug)]
pub enum UploadError {
    FileSize = 1,
    Quota = 2,
    Storage = 3,
    Protocol = 4,
    Cancelled = 5,

    Other = 0
}

impl From<u8> for UploadError {
    fn from(b: u8) -> Self {
        match b {
            1 => Self::FileSize,
            2 => Self::Quota,
            3 => Self::Storage,
            4 => Self::Protocol,
            5 => Self::Cancelled,
            _ => Self::Other
        }
    }
}

impl From<Error> for UploadError {
    fn from(e: Error) -> Self {
        match e.kind() {
            ErrorKind::InvalidInput => Self::Protocol,
            _ => Self::from(WriterError::from(e))
        }
    }
}

impl From<WriterError> for UploadError {
    fn from(e: WriterError) -> Self {
        match e {
            WriterError::QuotaExceeded => Self::Quota,
            WriterError::StorageLimitExceeded => Self::Storage,
            WriterError::FileSizeLimitExceeded => Self::FileSize,
            WriterError::NotSupported => Self::Protocol,
            _ => Self::Other
        }
    }
}
