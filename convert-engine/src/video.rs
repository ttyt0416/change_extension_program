use crate::{Category, Error, unsupported};
use mp4::{AudioObjectType, Mp4Reader, TrackType};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

pub const OUTPUT_FORMATS: &[&str] = &["MP4", "M4V", "MOV"];
const INPUT_FORMATS: &[&str] = &["mp4", "m4v", "mov"];

pub fn output_formats_for(input: &Path) -> &'static [&'static str] {
    let extension = input
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    if INPUT_FORMATS
        .iter()
        .any(|format| extension.eq_ignore_ascii_case(format))
        && validate_input(input).is_ok()
    {
        OUTPUT_FORMATS
    } else {
        &[]
    }
}

pub fn convert(
    input: &Path,
    target_extension: &str,
    output_folder: &Path,
) -> Result<PathBuf, Error> {
    let extension = input
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    if !INPUT_FORMATS
        .iter()
        .any(|format| extension.eq_ignore_ascii_case(format))
    {
        return Err(Error::Video("지원하지 않는 입력 형식입니다".to_owned()));
    }
    if !OUTPUT_FORMATS
        .iter()
        .any(|format| format.eq_ignore_ascii_case(target_extension))
    {
        return Err(unsupported(Category::Video, target_extension));
    }

    let box_size = validate_input(input)?;
    let mut source = File::open(input)?;
    let mut header = [0u8; 16];
    source.read_exact(&mut header)?;
    let brand = if target_extension.eq_ignore_ascii_case("mov") {
        *b"qt  "
    } else {
        *b"isom"
    };
    header[8..12].copy_from_slice(&brand);

    let (path, file) = reserve_output(input, target_extension, output_folder)?;
    let result = (|| {
        let mut writer = BufWriter::new(file);
        writer.write_all(&header)?;
        if box_size >= 20 {
            let mut compatible_brand = [0u8; 4];
            source.read_exact(&mut compatible_brand)?;
            writer.write_all(&brand)?;
        }
        io::copy(&mut source, &mut writer)?;
        writer.flush()?;
        Ok::<(), Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result.map(|()| path)
}

fn validate_input(input: &Path) -> Result<u64, Error> {
    let source = File::open(input)?;
    let size = source.metadata()?.len();
    let reader = Mp4Reader::read_header(BufReader::new(source), size)
        .map_err(|error| Error::Video(format!("파일 읽기: {error}")))?;
    if reader.is_fragmented() {
        return Err(Error::Video(
            "조각난 MP4 파일은 지원하지 않습니다".to_owned(),
        ));
    }
    let mut has_video = false;
    for track in reader.tracks().values() {
        match track.track_type().map_err(video_error)? {
            TrackType::Video => {
                if track.trak.mdia.minf.stbl.stsd.avc1.is_none() {
                    return Err(Error::Video(
                        "H.264 외의 영상 코덱은 지원하지 않습니다".to_owned(),
                    ));
                }
                if track.sample_count() == 0 {
                    return Err(Error::Video("영상 데이터가 없습니다".to_owned()));
                }
                has_video = true;
            }
            TrackType::Audio => {
                if track.trak.mdia.minf.stbl.stsd.mp4a.is_none() {
                    return Err(Error::Video(
                        "AAC 외의 소리 코덱은 지원하지 않습니다".to_owned(),
                    ));
                }
                let profile = track.audio_profile().map_err(video_error)?;
                if !matches!(
                    profile,
                    AudioObjectType::AacMain
                        | AudioObjectType::AacLowComplexity
                        | AudioObjectType::AacScalableSampleRate
                        | AudioObjectType::AacLongTermPrediction
                        | AudioObjectType::SpectralBandReplication
                        | AudioObjectType::ParametricStereo
                        | AudioObjectType::ErrorResilientAacLowComplexity
                        | AudioObjectType::ErrorResilientAacLongTermPrediction
                        | AudioObjectType::ErrorResilientAacScalable
                        | AudioObjectType::ErrorResilientAacLowDelay
                        | AudioObjectType::ErrorResilientAacEnhancedLowDelay
                ) {
                    return Err(Error::Video(
                        "AAC 외의 소리 코덱은 지원하지 않습니다".to_owned(),
                    ));
                }
            }
            _ => {
                return Err(Error::Video(
                    "자막 및 기타 트랙은 지원하지 않습니다".to_owned(),
                ));
            }
        }
    }
    if !has_video {
        return Err(Error::Video("영상 트랙이 없습니다".to_owned()));
    }

    let mut source = File::open(input)?;
    let mut header = [0u8; 16];
    source.read_exact(&mut header)?;
    let box_size = u32::from_be_bytes(header[..4].try_into().unwrap()) as u64;
    if &header[4..8] != b"ftyp" || box_size < 16 || box_size > size {
        return Err(Error::Video(
            "첫 번째 MP4 형식 정보가 올바르지 않습니다".to_owned(),
        ));
    }
    Ok(box_size)
}

fn video_error(error: impl std::fmt::Display) -> Error {
    Error::Video(error.to_string())
}

fn reserve_output(input: &Path, extension: &str, folder: &Path) -> Result<(PathBuf, File), Error> {
    let stem = input
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| OsStr::new("output"));
    for index in 0..10_000 {
        let mut name = OsString::from(stem);
        if index > 0 {
            name.push(format!("_{index}"));
        }
        name.push(".");
        name.push(extension);
        let path = folder.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Video(
        "같은 이름의 출력 파일이 너무 많습니다".to_owned(),
    ))
}
