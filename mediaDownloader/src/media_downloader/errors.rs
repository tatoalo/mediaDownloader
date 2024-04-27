use std::error::Error;
use std::fmt::{self, Display};
use std::io;

use crate::{CHONK, CROSS_MARK, FAILED, MONKEY, RADIOACTIVE};

#[derive(Debug)]
pub enum MediaDownloaderError {
    GenericError,
    UnsupportedDomain,
    BlobRetrievingError,
    DownloadError,
    CouldNotExtractId,
    InvalidUrl,
    FileSizeExceeded,
    ImagesNotDownloaded,
    IoErrorDirectory(io::Error),
    CustomParsingError(String),
    ParsingError,
    UnreachableResource,
    DriverError,
}

impl Error for MediaDownloaderError {}

impl From<io::Error> for MediaDownloaderError {
    fn from(error: io::Error) -> Self {
        MediaDownloaderError::IoErrorDirectory(error)
    }
}

impl Display for MediaDownloaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaDownloaderError::GenericError => {
                write!(f, "{} Failed to download resource!", CROSS_MARK)
            }
            MediaDownloaderError::UnsupportedDomain => {
                write!(f, "{} Domain not supported!", MONKEY)
            }
            MediaDownloaderError::BlobRetrievingError => {
                write!(f, "{} Error retrieving file!", CROSS_MARK)
            }
            MediaDownloaderError::DownloadError => {
                write!(f, "{} Error downloading video!", RADIOACTIVE)
            }
            MediaDownloaderError::CouldNotExtractId => {
                write!(f, "{} Error extracting video id!", FAILED)
            }
            MediaDownloaderError::InvalidUrl => {
                write!(f, "{} Invalid URL!", FAILED)
            }
            MediaDownloaderError::FileSizeExceeded => {
                write!(f, "{} File size exceeded!", CHONK)
            }
            MediaDownloaderError::ImagesNotDownloaded => {
                write!(f, "{} Images not downloaded, try again!", FAILED)
            }
            MediaDownloaderError::IoErrorDirectory(_) => {
                write!(f, "{} Error creating `images` directory!", MONKEY)
            }
            MediaDownloaderError::CustomParsingError(_) => {
                write!(f, "{}", self)
            }
            MediaDownloaderError::ParsingError => MediaDownloaderError::GenericError.fmt(f),
            MediaDownloaderError::UnreachableResource => MediaDownloaderError::GenericError.fmt(f),
            MediaDownloaderError::DriverError => MediaDownloaderError::GenericError.fmt(f),
        }
    }
}
