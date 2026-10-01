use crate::{Category, Error, unsupported};
use image::{AnimationDecoder, ImageFormat, ImageReader};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::BufReader;
use std::path::{Path, PathBuf};

pub const OUTPUT_FORMATS: &[&str] = &[
    "PNG", "JPG", "JPEG", "GIF", "BMP", "TIFF", "TIF", "WEBP", "ICO", "TGA", "PNM", "QOI",
];
const OUTPUT_FORMATS_NO_ICO: &[&str] = &[
    "PNG", "JPG", "JPEG", "GIF", "BMP", "TIFF", "TIF", "WEBP", "TGA", "PNM", "QOI",
];

pub fn output_formats_for(input: &Path) -> &'static [&'static str] {
    match decode_image(input) {
        Ok(image) if image.width() > 256 || image.height() > 256 => OUTPUT_FORMATS_NO_ICO,
        Ok(_) => OUTPUT_FORMATS,
        Err(_) => &[],
    }
}

pub fn convert(
    input: &Path,
    target_extension: &str,
    output_folder: &Path,
) -> Result<PathBuf, Error> {
    if !OUTPUT_FORMATS
        .iter()
        .any(|format| format.eq_ignore_ascii_case(target_extension))
    {
        return Err(unsupported(Category::Image, target_extension));
    }
    let format = ImageFormat::from_extension(target_extension)
        .ok_or_else(|| unsupported(Category::Image, target_extension))?;
    let mut decoded = decode_image(input)?;
    if format == ImageFormat::Ico {
        decoded = image::DynamicImage::ImageRgba8(decoded.into_rgba8());
    }
    let (path, mut file) = reserve_output(input, target_extension, output_folder)?;
    if let Err(error) = decoded.write_to(&mut file, format) {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(error.into());
    }
    Ok(path)
}

fn decode_image(input: &Path) -> Result<image::DynamicImage, Error> {
    let reader = ImageReader::open(input)?.with_guessed_format()?;
    match reader.format() {
        Some(ImageFormat::Gif) => {
            let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(File::open(input)?))?;
            let mut frames = decoder.into_frames();
            let _ = frames.next().transpose()?;
            if frames.next().transpose()?.is_some() {
                return Err(Error::UnsupportedAnimation(input.to_path_buf()));
            }
        }
        Some(ImageFormat::WebP) => {
            let decoder =
                image::codecs::webp::WebPDecoder::new(BufReader::new(File::open(input)?))?;
            if decoder.has_animation() {
                return Err(Error::UnsupportedAnimation(input.to_path_buf()));
            }
        }
        _ => {}
    }
    Ok(reader.decode()?)
}

fn reserve_output(
    input: &Path,
    target_extension: &str,
    output_folder: &Path,
) -> Result<(PathBuf, File), Error> {
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
        name.push(target_extension);
        let path = output_folder.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "사용 가능한 출력 파일 이름이 없습니다",
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_png_to_jpeg_without_overwriting_existing_output() {
        let unique = format!(
            "convert-engine-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        fs::create_dir(&directory).unwrap();
        let input = directory.join("sample.png");
        image::DynamicImage::new_rgb8(2, 2)
            .save_with_format(&input, ImageFormat::Png)
            .unwrap();

        let first = convert(&input, "jpg", &directory).unwrap();
        let second = convert(&input, "jpg", &directory).unwrap();
        assert_ne!(first, second);
        assert_eq!(&fs::read(first).unwrap()[..2], &[0xFF, 0xD8]);
        assert_eq!(&fs::read(second).unwrap()[..2], &[0xFF, 0xD8]);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn every_advertised_output_format_can_be_written_and_read() {
        let unique = format!(
            "convert-engine-formats-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        fs::create_dir(&directory).unwrap();
        let input = directory.join("sample.png");
        image::DynamicImage::new_rgb8(2, 2)
            .save_with_format(&input, ImageFormat::Png)
            .unwrap();

        for &extension in OUTPUT_FORMATS {
            let output = convert(&input, &extension.to_ascii_lowercase(), &directory)
                .unwrap_or_else(|error| panic!("{extension}: {error}"));
            let decoded = ImageReader::open(&output)
                .unwrap()
                .with_guessed_format()
                .unwrap()
                .decode()
                .unwrap_or_else(|error| panic!("{extension}: {error}"));
            assert_eq!((decoded.width(), decoded.height()), (2, 2), "{extension}");
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn animated_gif_is_rejected_instead_of_losing_frames() {
        let unique = format!(
            "convert-engine-animation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        fs::create_dir(&directory).unwrap();
        let input = directory.join("animated.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(File::create(&input).unwrap());
        encoder
            .encode_frames([
                image::Frame::new(image::RgbaImage::new(2, 2)),
                image::Frame::new(image::RgbaImage::new(2, 2)),
            ])
            .unwrap();
        drop(encoder);

        assert!(matches!(
            convert(&input, "png", &directory),
            Err(Error::UnsupportedAnimation(_))
        ));
        assert!(output_formats_for(&input).is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn large_image_does_not_offer_ico() {
        let directory = std::env::temp_dir().join(format!(
            "convert-engine-large-image-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let input = directory.join("wide.png");
        image::DynamicImage::new_rgb8(257, 1)
            .save_with_format(&input, ImageFormat::Png)
            .unwrap();
        let formats = output_formats_for(&input);
        assert!(formats.contains(&"PNG"));
        assert!(!formats.contains(&"ICO"));
        fs::remove_dir_all(directory).unwrap();
    }
}
