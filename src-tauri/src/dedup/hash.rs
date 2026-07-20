use image::imageops::FilterType;
use image::DynamicImage;
use rustdct::DctPlanner;
use sha2::{Digest, Sha256};

pub fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut result = [0u8; 32];
    result.copy_from_slice(&digest);
    result
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha256_bytes(bytes))
}

pub fn perceptual_hash(image: &DynamicImage) -> u64 {
    const SIZE: usize = 32;
    const HASH_SIZE: usize = 8;

    let gray = image
        .resize_exact(SIZE as u32, SIZE as u32, FilterType::Lanczos3)
        .to_luma8();
    let mut values: Vec<f32> = gray.pixels().map(|pixel| pixel[0] as f32).collect();
    let mut planner = DctPlanner::new();
    let dct = planner.plan_dct2(SIZE);

    for row in values.chunks_exact_mut(SIZE) {
        dct.process_dct2(row);
    }

    let mut column = vec![0.0f32; SIZE];
    for x in 0..SIZE {
        for y in 0..SIZE {
            column[y] = values[y * SIZE + x];
        }
        dct.process_dct2(&mut column);
        for y in 0..SIZE {
            values[y * SIZE + x] = column[y];
        }
    }

    let mut low_frequency = Vec::with_capacity(HASH_SIZE * HASH_SIZE - 1);
    for y in 0..HASH_SIZE {
        for x in 0..HASH_SIZE {
            if x != 0 || y != 0 {
                low_frequency.push(values[y * SIZE + x]);
            }
        }
    }
    let mut median_values = low_frequency.clone();
    median_values.sort_by(|left, right| left.total_cmp(right));
    let median = median_values[median_values.len() / 2];
    let noise_floor = low_frequency
        .iter()
        .map(|value| value.abs())
        .fold(0.0f32, f32::max)
        * 0.001;

    let mut hash = 0u64;
    for (index, value) in low_frequency.into_iter().enumerate() {
        if value > median + noise_floor {
            hash |= 1u64 << index;
        }
    }
    hash
}

pub fn phash_distance(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    fn gradient(width: u32, height: u32) -> DynamicImage {
        let mut image = RgbImage::new(width, height);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = Rgb([
                ((x * 255) / width.max(1)) as u8,
                ((y * 255) / height.max(1)) as u8,
                64,
            ]);
        }
        DynamicImage::ImageRgb8(image)
    }

    #[test]
    fn phash_is_stable_across_resize() {
        let small = gradient(64, 48);
        let large = gradient(1024, 768);
        let small_hash = perceptual_hash(&small);
        let large_hash = perceptual_hash(&large);
        let distance = phash_distance(small_hash, large_hash);
        assert!(
            distance <= 4,
            "distance={distance}, small={small_hash:016x}, large={large_hash:016x}"
        );
    }

    #[test]
    fn phash_distinguishes_different_structure() {
        let gradient = gradient(256, 192);
        let checker = DynamicImage::ImageRgb8(RgbImage::from_fn(256, 192, |x, y| {
            if ((x / 16) + (y / 16)) % 2 == 0 {
                Rgb([245, 245, 245])
            } else {
                Rgb([12, 12, 12])
            }
        }));

        assert_ne!(perceptual_hash(&gradient), perceptual_hash(&checker));
    }

    #[test]
    fn sha256_is_deterministic() {
        assert_eq!(sha256_hex(b"imsViewer"), sha256_hex(b"imsViewer"));
        assert_ne!(sha256_hex(b"imsViewer"), sha256_hex(b"imsviewer"));
    }
}
