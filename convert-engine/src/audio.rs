use crate::{Category, Error, unsupported};
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const OUTPUT_FORMATS: &[&str] = &["WAV", "FLAC", "MP3"];
const OUTPUT_FORMATS_NO_MP3: &[&str] = &["WAV", "FLAC"];
const OUTPUT_FORMATS_WAV_ONLY: &[&str] = &["WAV"];
const INPUT_FORMATS: &[&str] = &[
    "wav", "mp3", "flac", "ogg", "aac", "m4a", "aiff", "aif", "caf",
];
const MAX_SAMPLES: usize = 100_000_000;

pub fn output_formats_for(input: &Path) -> &'static [&'static str] {
    let extension = input
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    if !INPUT_FORMATS
        .iter()
        .any(|format| extension.eq_ignore_ascii_case(format))
    {
        return &[];
    }
    match decode(input) {
        Ok(audio) if audio.sample_rate == 0 || audio.sample_rate > 655_350 => {
            OUTPUT_FORMATS_WAV_ONLY
        }
        Ok(audio)
            if audio.channels > 2
                || !shine_rs::SUPPORTED_SAMPLE_RATES.contains(&audio.sample_rate) =>
        {
            OUTPUT_FORMATS_NO_MP3
        }
        Ok(_) => OUTPUT_FORMATS,
        Err(_) => &[],
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
        return Err(Error::Audio("지원하지 않는 입력 형식입니다".to_owned()));
    }
    if !OUTPUT_FORMATS
        .iter()
        .any(|format| format.eq_ignore_ascii_case(target_extension))
    {
        return Err(unsupported(Category::Audio, target_extension));
    }
    let audio = decode(input)?;
    match target_extension {
        "wav" => write_wav(input, output_folder, &audio),
        "flac" => {
            let encoded = encode_flac(&audio)?;
            write_bytes(input, target_extension, output_folder, &encoded)
        }
        "mp3" => {
            let encoded = encode_mp3(&audio)?;
            write_bytes(input, target_extension, output_folder, &encoded)
        }
        _ => Err(unsupported(Category::Audio, target_extension)),
    }
}

struct Audio {
    samples: Vec<i32>,
    sample_rate: u32,
    channels: usize,
    bits_per_sample: u16,
}

fn decode(input: &Path) -> Result<Audio, Error> {
    if input
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("flac"))
    {
        return decode_flac(input);
    }
    let source = MediaSourceStream::new(Box::new(File::open(input)?), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = input.extension().and_then(OsStr::to_str) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| Error::Audio(format!("입력 형식 확인: {error}")))?
        .format;
    let track = format
        .default_track()
        .filter(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| Error::Audio("디코딩 가능한 소리 트랙이 없습니다".to_owned()))?;
    let track_id = track.id;
    let bits_per_sample = if track.codec_params.bits_per_sample.unwrap_or(16) > 16 {
        24
    } else {
        16
    };
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| Error::Audio(format!("디코더 생성: {error}")))?;
    let mut audio = Audio {
        samples: Vec::new(),
        sample_rate: 0,
        channels: 0,
        bits_per_sample,
    };
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(error) => return Err(Error::Audio(format!("오디오 패킷 읽기: {error}"))),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|error| Error::Audio(format!("오디오 패킷 디코딩: {error}")))?;
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        if channels == 0 || channels > 8 {
            return Err(Error::Audio("지원하지 않는 채널 수입니다".to_owned()));
        }
        if audio.sample_rate == 0 {
            audio.sample_rate = spec.rate;
            audio.channels = channels;
        } else if audio.sample_rate != spec.rate || audio.channels != channels {
            return Err(Error::Audio(
                "파일 안에서 샘플레이트나 채널 수가 바뀝니다".to_owned(),
            ));
        }
        if audio
            .samples
            .len()
            .saturating_add(decoded.frames() * channels)
            > MAX_SAMPLES
        {
            return Err(Error::Audio(
                "메모리 제한을 넘는 긴 소리 파일입니다".to_owned(),
            ));
        }
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        let scale = if bits_per_sample == 24 {
            8_388_608.0
        } else {
            32_768.0
        };
        for &sample in buffer.samples() {
            if !sample.is_finite() {
                return Err(Error::Audio(
                    "유효하지 않은 오디오 샘플이 있습니다".to_owned(),
                ));
            }
            audio.samples.push(
                (sample.clamp(-1.0, 1.0) * scale)
                    .round()
                    .clamp(-scale, scale - 1.0) as i32,
            );
        }
    }
    if audio.samples.is_empty() {
        return Err(Error::Audio("변환할 소리 데이터가 없습니다".to_owned()));
    }
    Ok(audio)
}

fn decode_flac(input: &Path) -> Result<Audio, Error> {
    let mut reader = claxon::FlacReader::open(input)
        .map_err(|error| Error::Audio(format!("FLAC 읽기: {error}")))?;
    let info = reader.streaminfo();
    let channels = info.channels as usize;
    if channels == 0 || channels > 8 || info.bits_per_sample > 24 {
        return Err(Error::Audio(
            "지원하지 않는 FLAC 채널 수 또는 비트 깊이입니다".to_owned(),
        ));
    }
    let bits_per_sample = if info.bits_per_sample > 16 { 24 } else { 16 };
    let shift = bits_per_sample - info.bits_per_sample as u16;
    let mut samples = Vec::new();
    for sample in reader.samples() {
        if samples.len() >= MAX_SAMPLES {
            return Err(Error::Audio(
                "메모리 제한을 넘는 긴 소리 파일입니다".to_owned(),
            ));
        }
        samples
            .push(sample.map_err(|error| Error::Audio(format!("FLAC 디코딩: {error}")))? << shift);
    }
    if samples.is_empty() {
        return Err(Error::Audio("변환할 소리 데이터가 없습니다".to_owned()));
    }
    Ok(Audio {
        samples,
        sample_rate: info.sample_rate,
        channels,
        bits_per_sample,
    })
}

fn encode_flac(audio: &Audio) -> Result<Vec<u8>, Error> {
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|error| Error::Audio(format!("FLAC 설정 오류: {error:?}")))?;
    let source = flacenc::source::MemSource::from_samples(
        &audio.samples,
        audio.channels,
        audio.bits_per_sample as usize,
        audio.sample_rate as usize,
    );
    let stream = flacenc::encode_with_fixed_block_size(&config, source, 4096)
        .map_err(|error| Error::Audio(format!("FLAC 인코딩 오류: {error:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|error| Error::Audio(format!("FLAC 저장 오류: {error:?}")))?;
    Ok(sink.into_inner())
}

fn encode_mp3(audio: &Audio) -> Result<Vec<u8>, Error> {
    if audio.channels > 2 {
        return Err(Error::Audio("MP3는 1~2채널 입력만 지원합니다".to_owned()));
    }
    if !shine_rs::SUPPORTED_SAMPLE_RATES.contains(&audio.sample_rate) {
        return Err(Error::Audio(format!(
            "MP3 인코딩이 지원하지 않는 샘플레이트: {} Hz",
            audio.sample_rate
        )));
    }
    let bitrate = if audio.sample_rate >= 32_000 {
        192
    } else if audio.sample_rate >= 16_000 {
        128
    } else {
        64
    };
    let config = shine_rs::Mp3EncoderConfig::new()
        .sample_rate(audio.sample_rate)
        .bitrate(bitrate)
        .channels(audio.channels as u8)
        .stereo_mode(if audio.channels == 1 {
            shine_rs::StereoMode::Mono
        } else {
            shine_rs::StereoMode::JointStereo
        });
    let shift = if audio.bits_per_sample == 24 { 8 } else { 0 };
    let samples: Vec<i16> = audio
        .samples
        .iter()
        .map(|sample| (sample >> shift) as i16)
        .collect();
    shine_rs::encode_pcm_to_mp3(config, &samples)
        .map_err(|error| Error::Audio(format!("MP3 인코딩 오류: {error:?}")))
}

fn write_wav(input: &Path, folder: &Path, audio: &Audio) -> Result<PathBuf, Error> {
    let (path, file) = reserve_output(input, "wav", folder)?;
    let result = (|| {
        let spec = hound::WavSpec {
            channels: audio.channels as u16,
            sample_rate: audio.sample_rate,
            bits_per_sample: audio.bits_per_sample,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(BufWriter::new(file), spec)
            .map_err(|error| Error::Audio(error.to_string()))?;
        for &sample in &audio.samples {
            writer
                .write_sample(sample)
                .map_err(|error| Error::Audio(error.to_string()))?;
        }
        writer
            .finalize()
            .map_err(|error| Error::Audio(error.to_string()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result.map(|()| path)
}

fn write_bytes(
    input: &Path,
    extension: &str,
    folder: &Path,
    bytes: &[u8],
) -> Result<PathBuf, Error> {
    let (path, mut file) = reserve_output(input, extension, folder)?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.flush()) {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(error.into());
    }
    Ok(path)
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
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Audio(
        "사용 가능한 출력 파일 이름이 없습니다".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(label: &str) -> PathBuf {
        let unique = format!(
            "convert-engine-audio-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        fs::create_dir(&directory).unwrap();
        directory
    }

    fn sample_wav(path: &Path) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for frame in 0..44_100 {
            let sample =
                ((frame as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 12_000.0) as i16;
            writer.write_sample(sample).unwrap();
            writer.write_sample(-sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn wav_flac_round_trip_preserves_samples_and_avoids_overwrite() {
        let directory = test_directory("flac");
        let input = directory.join("tone.wav");
        sample_wav(&input);
        let flac = convert(&input, "flac", &directory).unwrap();
        assert!(fs::read(&flac).unwrap().starts_with(b"fLaC"));
        let restored = convert(&flac, "wav", &directory).unwrap();
        assert_ne!(input, restored);
        let original = decode(&input).unwrap();
        let decoded = decode(&restored).unwrap();
        assert_eq!(decoded.sample_rate, original.sample_rate);
        assert_eq!(decoded.channels, original.channels);
        assert_eq!(decoded.samples, original.samples);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn encodes_mp3_that_can_be_decoded_again() {
        let directory = test_directory("mp3");
        let input = directory.join("tone.wav");
        sample_wav(&input);
        let mp3 = convert(&input, "mp3", &directory).unwrap();
        assert!(fs::metadata(&mp3).unwrap().len() > 1_000);
        let decoded = decode(&mp3).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.channels, 2);
        assert!(!decoded.samples.is_empty());
        let wav = convert(&mp3, "wav", &directory).unwrap();
        let flac = convert(&mp3, "flac", &directory).unwrap();
        assert!(fs::read(&wav).unwrap().starts_with(b"RIFF"));
        assert!(fs::read(&flac).unwrap().starts_with(b"fLaC"));
        assert!(matches!(
            convert(&input, "opus", &directory),
            Err(Error::UnsupportedFormat { .. })
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn multichannel_audio_does_not_offer_mp3() {
        let directory = test_directory("multichannel");
        let input = directory.join("three_channels.wav");
        let spec = hound::WavSpec {
            channels: 3,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&input, spec).unwrap();
        for _ in 0..300 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();
        let formats = output_formats_for(&input);
        assert!(formats.contains(&"WAV"));
        assert!(formats.contains(&"FLAC"));
        assert!(!formats.contains(&"MP3"));
        assert!(convert(&input, "flac", &directory).is_ok());
        fs::remove_dir_all(directory).unwrap();
    }
}
