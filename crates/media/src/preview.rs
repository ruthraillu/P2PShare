//! Bounded JPEG thumbnails. Decode limits prevent remotely supplied oversized images.
use anyhow::{Result, ensure};
use image::{ImageEncoder, ImageReader};
use std::io::Cursor;
pub const MAX_EDGE: u32 = 512;
pub const MAX_BYTES: usize = 48 * 1024;
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        width > 0 && height > 0 && width <= MAX_EDGE && height <= MAX_EDGE,
        "Invalid preview dimensions"
    );
    ensure!(
        rgba.len() == width as usize * height as usize * 4,
        "Invalid preview pixels"
    );
    let rgb: Vec<_> = rgba
        .chunks_exact(4)
        .flat_map(|p| p[..3].iter().copied())
        .collect();
    // Keep previews inside the existing small data-channel message budget.
    // Complex images may need lower JPEG quality at the larger dimensions.
    let mut bytes = Vec::new();
    for quality in [65, 50, 35, 20, 10, 1] {
        bytes.clear();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality).write_image(
            &rgb,
            width,
            height,
            image::ExtendedColorType::Rgb8,
        )?;
        if bytes.len() <= MAX_BYTES {
            return Ok(bytes);
        }
    }
    anyhow::bail!("Preview too large")
}
pub fn decode(bytes: &[u8]) -> Result<Image> {
    ensure!(bytes.len() <= MAX_BYTES, "Preview too large");
    let mut reader = ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(2 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?.to_rgba8();
    Ok(Image {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_interval_limits() {
        for seconds in [10, 60] {
            assert!(
                crate::config::StreamConfig {
                    preview_interval_secs: seconds,
                    ..Default::default()
                }
                .validate()
                .is_ok()
            );
        }
        for seconds in [0, 9, 61] {
            assert!(
                crate::config::StreamConfig {
                    preview_interval_secs: seconds,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn jpeg_bounds_and_roundtrip() {
        for (w, h) in [(512, 288), (288, 512), (512, 512), (256, 144), (1, 1)] {
            let bytes = encode(w, h, &[40, 100, 180, 255].repeat((w * h) as usize)).unwrap();
            let image = decode(&bytes).unwrap();
            assert_eq!((image.width, image.height), (w, h));
            assert!(bytes.len() < MAX_BYTES);
        }
        assert!(encode(513, 1, &[0; 2052]).is_err());
        assert!(decode(&vec![0; MAX_BYTES + 1]).is_err());
        assert!(decode(b"invalid jpeg").is_err());
        let mut huge = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut huge)
            .write_image(
                &vec![0; 1024 * 1024 * 3],
                1024,
                1024,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        assert!(decode(&huge).is_err());
    }
    #[test]
    fn detailed_512_preview_stays_within_transport_budget() {
        let mut random = 0x12345678u32;
        let mut rgba = vec![255; 512 * 512 * 4];
        for pixel in rgba.chunks_exact_mut(4) {
            for channel in &mut pixel[..3] {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                *channel = random as u8;
            }
        }
        let bytes = encode(512, 512, &rgba).unwrap();
        assert!(bytes.len() <= MAX_BYTES);
        let image = decode(&bytes).unwrap();
        assert_eq!((image.width, image.height), (512, 512));
    }
}
