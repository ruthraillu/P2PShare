//! Bounded static profile images and deterministic crop/encoding.
use anyhow::{Result, ensure};
pub type Source = image::RgbaImage;
pub const MAX_BYTES: usize = 48 * 1024;
pub fn source(bytes: &[u8]) -> Result<image::RgbaImage> {
    ensure!(bytes.len() <= 16 * 1024 * 1024, "Image file exceeds 16 MB");
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode()?.to_rgba8())
}
pub fn validate(bytes: &[u8]) -> Result<image::RgbaImage> {
    ensure!(bytes.len() <= MAX_BYTES, "Avatar exceeds 48 KB");
    let dimensions = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()?
        .into_dimensions()?;
    ensure!(dimensions == (256, 256), "Avatar must be 256 square");
    source(bytes)
}
pub fn crop(image: &image::RgbaImage, zoom: f32, x: f32, y: f32) -> Result<Vec<u8>> {
    let side =
        ((image.width().min(image.height()) as f32 / zoom.clamp(1., 8.)).round() as u32).max(1);
    let left = ((image.width() - side) as f32 * x.clamp(0., 1.)).round() as u32;
    let top = ((image.height() - side) as f32 * y.clamp(0., 1.)).round() as u32;
    let cropped = image::imageops::crop_imm(image, left, top, side, side).to_image();
    let scaled = image::imageops::resize(&cropped, 256, 256, image::imageops::FilterType::Lanczos3);
    let mut scaled = scaled;
    for pixel in scaled.pixels_mut() {
        let a = pixel[3] as u16;
        for (i, bg) in [21u16, 22, 28].iter().enumerate() {
            pixel[i] = ((pixel[i] as u16 * a + bg * (255 - a)) / 255) as u8;
        }
        pixel[3] = 255;
    }
    let rgb = image::DynamicImage::ImageRgba8(scaled).to_rgb8();
    let mut bytes = Vec::new();
    for quality in [85, 70, 50, 30, 10] {
        bytes.clear();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality).encode(
            rgb.as_raw(),
            256,
            256,
            image::ExtendedColorType::Rgb8,
        )?;
        if bytes.len() <= MAX_BYTES {
            return Ok(bytes);
        }
    }
    anyhow::bail!("Cannot encode avatar")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_is_bounded_and_valid() {
        let image = image::RgbaImage::from_pixel(700, 400, image::Rgba([40, 80, 120, 255]));
        let bytes = crop(&image, 2., 0.8, 0.2).unwrap();
        assert_eq!(validate(&bytes).unwrap().dimensions(), (256, 256));
        assert!(validate(&vec![0; MAX_BYTES + 1]).is_err());
    }
}
