use std::error::Error;
use std::fmt::{self, Display};
use std::io;

use crate::{CHONK, CROSS_MARK, FAILED, MONKEY, RADIOACTIVE};

#[derive(Debug)]
pub enum MediaDownloaderError {
    UnsupportedDomain,
    BlobRetrievingError,
    DownloadError,
    CouldNotExtractId,
    InvalidUrl,
    FileSizeExceeded,
    ImagesNotDownloaded,
    IoErrorImagesDirectory(io::Error),
}

impl Error for MediaDownloaderError {}

impl From<io::Error> for MediaDownloaderError {
    fn from(error: io::Error) -> Self {
        MediaDownloaderError::IoErrorImagesDirectory(error)
    }
}

impl Display for MediaDownloaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
            MediaDownloaderError::IoErrorImagesDirectory(_) => {
                write!(f, "{} Error creating `images` directory!", MONKEY)
            }
        }
    }
}
