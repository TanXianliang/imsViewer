use super::source::{decode_source, source_from_path};
use super::types::{DedupVerificationRequest, DedupVerificationResult};
use bye_orb_rs::orb::{match_brief, orb};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;
use std::collections::HashSet;
use std::path::Path;

pub struct OcrVerifier {
    engine: OcrEngine,
}

impl OcrVerifier {
    pub fn load(detection_path: &Path, recognition_path: &Path) -> Result<Self, String> {
        let detection_model = Model::load_file(detection_path)
            .map_err(|error| format!("加载 OCR 检测模型失败: {}", error))?;
        let recognition_model = Model::load_file(recognition_path)
            .map_err(|error| format!("加载 OCR 识别模型失败: {}", error))?;
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..Default::default()
        })
        .map_err(|error| format!("初始化 OCR 引擎失败: {}", error))?;
        Ok(Self { engine })
    }

    fn recognize(&self, image: &image::DynamicImage) -> Result<String, String> {
        let rgb = image.to_rgb8();
        let source = ImageSource::from_bytes(rgb.as_raw(), rgb.dimensions())
            .map_err(|error| format!("准备 OCR 图片失败: {}", error))?;
        let input = self
            .engine
            .prepare_input(source)
            .map_err(|error| format!("OCR 预处理失败: {}", error))?;
        self.engine
            .get_text(&input)
            .map_err(|error| format!("OCR 识别失败: {}", error))
    }
}

pub fn verify_pair(
    request: &DedupVerificationRequest,
    ocr: &OcrVerifier,
) -> Result<DedupVerificationResult, String> {
    let source_item = source_from_path(Path::new(&request.source_path))?;
    let candidate_item = source_from_path(Path::new(&request.candidate_path))?;
    let source = decode_source(&source_item)?;
    let candidate = decode_source(&candidate_item)?;

    let local_match_score = local_feature_score(&source.image, &candidate.image);
    let source_text = ocr.recognize(&source.image).unwrap_or_default();
    let candidate_text = ocr.recognize(&candidate.image).unwrap_or_default();
    let ocr_similarity = text_similarity(&source_text, &candidate_text);
    let has_text = normalized_text(&source_text).chars().count() >= 4
        && normalized_text(&candidate_text).chars().count() >= 4;
    let template_risk = has_text && request.raw_similarity >= 0.8 && ocr_similarity < 0.35;
    let local_verified = local_match_score >= 0.06;
    let verdict = if template_risk {
        "文字差异较大，疑似相同模板而非重复图片"
    } else if local_verified {
        "局部特征匹配通过，支持同源图片判断"
    } else if has_text && ocr_similarity >= 0.7 {
        "文字内容高度相似"
    } else {
        "局部特征不足，建议人工复核"
    };

    Ok(DedupVerificationResult {
        local_match_score,
        local_verified,
        source_text,
        candidate_text,
        ocr_similarity,
        template_risk,
        verdict: verdict.to_string(),
    })
}

fn local_feature_score(left: &image::DynamicImage, right: &image::DynamicImage) -> f32 {
    let left = left.thumbnail(320, 320);
    let right = right.thumbnail(320, 320);
    let left_descriptors = orb(&left, 200).unwrap_or_default();
    let right_descriptors = orb(&right, 200).unwrap_or_default();
    if left_descriptors.len() < 2 || right_descriptors.len() < 2 {
        return 0.0;
    }
    let left_to_right = match_brief(&left_descriptors, &right_descriptors).len();
    let right_to_left = match_brief(&right_descriptors, &left_descriptors).len();
    left_to_right.min(right_to_left) as f32
        / left_descriptors.len().min(right_descriptors.len()) as f32
}

fn normalized_text(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .take(1024)
        .collect()
}

fn text_similarity(left: &str, right: &str) -> f32 {
    let left = normalized_text(left);
    let right = normalized_text(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left_pairs = character_pairs(&left);
    let right_pairs = character_pairs(&right);
    if left_pairs.is_empty() || right_pairs.is_empty() {
        return if left == right { 1.0 } else { 0.0 };
    }
    let intersection = left_pairs.intersection(&right_pairs).count();
    let union = left_pairs.union(&right_pairs).count();
    intersection as f32 / union as f32
}

fn character_pairs(text: &str) -> HashSet<(char, char)> {
    let chars = text.chars().collect::<Vec<_>>();
    chars.windows(2).map(|pair| (pair[0], pair[1])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    #[test]
    fn text_similarity_detects_changed_content() {
        assert!(text_similarity("Hello world 123", "hello world 123") > 0.99);
        assert!(text_similarity("Quarterly revenue 2025", "Vacation photo") < 0.2);
    }

    #[test]
    fn local_features_survive_resize_and_crop() {
        let source = RgbImage::from_fn(320, 240, |x, y| {
            let checker = (((x / 20) + (y / 20)) % 2) as u8;
            let noise = ((x.wrapping_mul(37) ^ y.wrapping_mul(91) ^ (x * y)) & 0xff) as u8;
            Rgb([
                noise,
                noise.wrapping_add(checker * 96),
                255_u8.wrapping_sub(noise / 2),
            ])
        });
        let source = DynamicImage::ImageRgb8(source);
        let resized = source.resize_exact(480, 360, image::imageops::FilterType::Lanczos3);
        let cropped = source.crop_imm(32, 24, 256, 192);
        let resize_score = local_feature_score(&source, &resized);
        let crop_score = local_feature_score(&source, &cropped);

        assert!(resize_score >= 0.06, "resize score: {resize_score}");
        assert!(crop_score >= 0.03, "crop score: {crop_score}");
    }

    #[test]
    fn bundled_ocr_models_load_and_run() {
        let model_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("models");
        let verifier = OcrVerifier::load(
            &model_dir.join("text-detection.rten"),
            &model_dir.join("text-recognition.rten"),
        )
        .unwrap();
        let image = DynamicImage::ImageRgb8(RgbImage::from_fn(640, 160, |x, y| {
            let stripe = (x / 36) % 2 == 0 && (32..128).contains(&y);
            if stripe {
                Rgb([20, 20, 20])
            } else {
                Rgb([245, 245, 245])
            }
        }));

        verifier.recognize(&image).unwrap();
    }
}
