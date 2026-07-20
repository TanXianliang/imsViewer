use super::hash::sha256_hex;
use super::types::EMBEDDING_DIM;
use image::imageops::FilterType;
use image::DynamicImage;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct SscdModel {
    session: Mutex<Session>,
    provider: String,
    model_path: PathBuf,
    model_sha256: String,
}

impl SscdModel {
    pub fn load(
        model_path: &Path,
        prefer_directml: bool,
        intra_threads: usize,
    ) -> Result<Self, String> {
        if !model_path.is_file() {
            return Err(format!("SSCD 模型不存在: {}", model_path.display()));
        }

        let model_bytes =
            fs::read(model_path).map_err(|error| format!("读取 SSCD 模型失败: {}", error))?;
        let model_sha256 = sha256_hex(&model_bytes);

        #[cfg(target_os = "windows")]
        if prefer_directml {
            let directml_session = load_directml_session(model_path);
            if let Ok(session) = directml_session {
                return Ok(Self {
                    session: Mutex::new(session),
                    provider: "DirectML".to_string(),
                    model_path: model_path.to_path_buf(),
                    model_sha256,
                });
            }
        }

        let session = Session::builder()
            .map_err(|error| format!("初始化 ONNX Runtime 失败: {}", error))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|error| format!("设置 ONNX 优化失败: {}", error))?
            .with_intra_threads(intra_threads.max(1))
            .map_err(|error| format!("设置 ONNX 线程失败: {}", error))?
            .commit_from_file(model_path)
            .map_err(|error| format!("加载 SSCD ONNX 模型失败: {}", error))?;

        Ok(Self {
            session: Mutex::new(session),
            provider: "CPU".to_string(),
            model_path: model_path.to_path_buf(),
            model_sha256,
        })
    }

    pub fn infer(&self, image: &DynamicImage) -> Result<[f32; EMBEDDING_DIM], String> {
        let resized = image.resize_exact(320, 320, FilterType::Triangle).to_rgb8();
        let mean = [0.485f32, 0.456, 0.406];
        let std = [0.229f32, 0.224, 0.225];
        let mut input = vec![0.0f32; 3 * 320 * 320];

        for (x, y, pixel) in resized.enumerate_pixels() {
            let offset = y as usize * 320 + x as usize;
            for channel in 0..3 {
                input[channel * 320 * 320 + offset] =
                    (pixel[channel] as f32 / 255.0 - mean[channel]) / std[channel];
            }
        }

        let tensor = Tensor::from_array(([1usize, 3, 320, 320], input))
            .map_err(|error| format!("创建 SSCD 输入失败: {}", error))?;
        let mut session = self
            .session
            .lock()
            .map_err(|error| format!("锁定 SSCD 模型失败: {}", error))?;
        let outputs = session
            .run(ort::inputs![tensor])
            .map_err(|error| format!("SSCD 推理失败: {}", error))?;
        let (_, output) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("读取 SSCD 输出失败: {}", error))?;
        if output.len() != EMBEDDING_DIM {
            return Err(format!("SSCD 输出维度异常: {}", output.len()));
        }

        let norm = output.iter().map(|value| value * value).sum::<f32>().sqrt();
        if norm <= f32::EPSILON {
            return Err("SSCD 输出向量范数为 0".to_string());
        }
        let mut embedding = [0.0f32; EMBEDDING_DIM];
        for (target, value) in embedding.iter_mut().zip(output.iter()) {
            *target = *value / norm;
        }
        Ok(embedding)
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    pub fn model_sha256(&self) -> &str {
        &self.model_sha256
    }
}

#[cfg(target_os = "windows")]
fn load_directml_session(model_path: &Path) -> Result<Session, String> {
    Session::builder()
        .map_err(|error| error.to_string())?
        .with_optimization_level(GraphOptimizationLevel::Level1)
        .map_err(|error| error.to_string())?
        .with_dimension_override("batch", 1)
        .map_err(|error| error.to_string())?
        .with_execution_providers([ort::ep::DirectML::default().build()])
        .map_err(|error| error.to_string())?
        .commit_from_file(model_path)
        .map_err(|error| error.to_string())
}

pub fn cosine_similarity(left: &[f32; EMBEDDING_DIM], right: &[f32; EMBEDDING_DIM]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    #[test]
    fn cosine_identity_is_one() {
        let mut vector = [0.0f32; EMBEDDING_DIM];
        vector[0] = 1.0;
        assert!((cosine_similarity(&vector, &vector) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn bundled_model_produces_normalized_embedding_with_acceleration_fallback() {
        let model_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("models")
            .join("sscd_disc_mixup.onnx");
        let model = SscdModel::load(&model_path, true, 2).unwrap();
        assert!(matches!(model.provider(), "DirectML" | "CPU"));
        eprintln!("SSCD test provider: {}", model.provider());
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(320, 320, Rgb([80, 120, 160])));
        let embedding = model.infer(&image).unwrap();
        let norm = embedding
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }
}
