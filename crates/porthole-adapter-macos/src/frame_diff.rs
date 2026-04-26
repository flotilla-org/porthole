//! Downsampled-grayscale frame fingerprinting for wait stable/dirty.
//!
//! Each sample captures a screenshot and reduces it to a fixed 64x64
//! grayscale buffer. Two fingerprints can be compared pixel-by-pixel to
//! yield a percentage of pixels that differ beyond a small intensity
//! tolerance. This is cheap (~4 KB per sample) and robust to a few
//! blinking pixels like a terminal cursor.

use image::{DynamicImage, GrayImage, ImageBuffer, Luma, imageops::FilterType};

pub const FINGERPRINT_SIDE: u32 = 64;
pub const FINGERPRINT_LEN: usize = (FINGERPRINT_SIDE * FINGERPRINT_SIDE) as usize;
const PIXEL_TOLERANCE: u8 = 10;

#[derive(Clone)]
pub struct Fingerprint(Box<[u8]>); // length FINGERPRINT_LEN

impl Fingerprint {
    pub fn from_png(png_bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png).map_err(|e| format!("png decode failed: {e}"))?;
        Ok(Self::from_dynamic(&img))
    }

    pub fn from_dynamic(img: &DynamicImage) -> Self {
        let gray: GrayImage = img.to_luma8();
        let resized: ImageBuffer<Luma<u8>, Vec<u8>> =
            image::imageops::resize(&gray, FINGERPRINT_SIDE, FINGERPRINT_SIDE, FilterType::Triangle);
        Self(resized.into_raw().into_boxed_slice())
    }

    /// Returns the fraction of pixels (0.0..=100.0) that differ from `other`
    /// by more than PIXEL_TOLERANCE in grayscale intensity.
    pub fn diff_pct(&self, other: &Fingerprint) -> f64 {
        let mut diffs = 0usize;
        for (a, b) in self.0.iter().zip(other.0.iter()) {
            let d = a.abs_diff(*b);
            if d > PIXEL_TOLERANCE {
                diffs += 1;
            }
        }
        (diffs as f64 / FINGERPRINT_LEN as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::*;

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> RgbaImage {
        ImageBuffer::from_pixel(width, height, Rgba(rgba))
    }

    #[test]
    fn identical_images_diff_zero() {
        let a = DynamicImage::ImageRgba8(solid(200, 100, [40, 40, 40, 255]));
        let fp_a = Fingerprint::from_dynamic(&a);
        let fp_b = Fingerprint::from_dynamic(&a);
        assert_eq!(fp_a.diff_pct(&fp_b), 0.0);
    }

    #[test]
    fn all_black_vs_all_white_diff_100() {
        let a = DynamicImage::ImageRgba8(solid(200, 100, [0, 0, 0, 255]));
        let b = DynamicImage::ImageRgba8(solid(200, 100, [255, 255, 255, 255]));
        let fp_a = Fingerprint::from_dynamic(&a);
        let fp_b = Fingerprint::from_dynamic(&b);
        let pct = fp_a.diff_pct(&fp_b);
        assert!(pct > 99.0, "expected near 100%, got {pct}");
    }

    #[test]
    fn small_region_change_is_small_pct() {
        let mut a = solid(200, 100, [40, 40, 40, 255]);
        let mut b = a.clone();
        // Change a 4x4 patch in b (tiny region).
        for y in 0..4 {
            for x in 0..4 {
                b.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let _ = &mut a;
        let fp_a = Fingerprint::from_dynamic(&DynamicImage::ImageRgba8(a));
        let fp_b = Fingerprint::from_dynamic(&DynamicImage::ImageRgba8(b));
        let pct = fp_a.diff_pct(&fp_b);
        assert!(pct < 2.0, "expected small-region change to be under 2%, got {pct}");
    }
}
