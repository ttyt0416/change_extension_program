pub mod audio;
pub mod document;
pub mod image;
pub mod video;

use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Audio,
    Video,
    Image,
    Document,
}

#[derive(Debug)]
pub enum Error {
    InvalidInput(PathBuf),
    InvalidOutputFolder(PathBuf),
    UnsupportedFormat {
        category: Category,
        extension: String,
    },
    UnsupportedAnimation(PathBuf),
    Document(String),
    Audio(String),
    Video(String),
    Io(std::io::Error),
    Image(::image::ImageError),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(path) => write!(
                formatter,
                "입력 파일을 찾을 수 없습니다: {}",
                path.display()
            ),
            Self::InvalidOutputFolder(path) => {
                write!(
                    formatter,
                    "출력 폴더를 찾을 수 없습니다: {}",
                    path.display()
                )
            }
            Self::UnsupportedFormat {
                category,
                extension,
            } => {
                write!(
                    formatter,
                    "{category:?} 변환에서 지원하지 않는 형식입니다: {extension}"
                )
            }
            Self::UnsupportedAnimation(path) => {
                write!(
                    formatter,
                    "애니메이션 이미지는 아직 지원하지 않습니다: {}",
                    path.display()
                )
            }
            Self::Document(message) => write!(formatter, "문서 변환 오류: {message}"),
            Self::Audio(message) => write!(formatter, "소리 변환 오류: {message}"),
            Self::Video(message) => write!(formatter, "동영상 변환 오류: {message}"),
            Self::Io(error) => write!(formatter, "파일 처리 오류: {error}"),
            Self::Image(error) => write!(formatter, "이미지 변환 오류: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<::image::ImageError> for Error {
    fn from(error: ::image::ImageError) -> Self {
        Self::Image(error)
    }
}

pub fn convert(
    category: Category,
    input: &Path,
    target_extension: &str,
    output_folder: &Path,
) -> Result<PathBuf, Error> {
    if !input.is_file() {
        return Err(Error::InvalidInput(input.to_path_buf()));
    }
    if !output_folder.is_dir() {
        return Err(Error::InvalidOutputFolder(output_folder.to_path_buf()));
    }

    let extension = target_extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    match category {
        Category::Image => image::convert(input, &extension, output_folder),
        Category::Audio => audio::convert(input, &extension, output_folder),
        Category::Video => video::convert(input, &extension, output_folder),
        Category::Document => document::convert(input, &extension, output_folder),
    }
}

fn unsupported(category: Category, extension: &str) -> Error {
    Error::UnsupportedFormat {
        category,
        extension: extension.to_owned(),
    }
}
