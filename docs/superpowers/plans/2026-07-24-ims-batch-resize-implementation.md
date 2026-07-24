# IMS 子图批量分辨率调整工具 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 imsViewer 中新增就地批量调整 IMS 文件子图分辨率的工具：用户设定最长边像素，对扫描到的 IMS 文件逐子图判断，仅缩小最长边超标的子图（保持长宽比、Lanczos3），未超标子图保留原字节，原子重写整个 IMS 文件。

**Architecture:** 完全镜像现有 `upgrade_ims_file`（`ims_pack.rs`）+ `start_batch_upgrade`（`lib.rs`）范式。在 `ims_pack.rs` 新增 `resize_ims_file` 核心函数与 `BatchResizeConfig`/`ResizeOutcome` 类型、AVIF 解码与 alpha-flatten 辅助函数；在 `lib.rs` 新增 `start_batch_resize` Tauri 命令（复用 `scan_ims_files`/`batch-progress`/rayon 双模式/`IMS_WRITE_LOCK`/`ImsCache` 失效）；前端新增 `#resize-modal`（复用 `.modal-*` 共享样式，多文件夹输入）与菜单项、app.js 控制函数。

**Tech Stack:** Rust 2021（`image` 0.25 jpeg/png/webp features、`avif-rs` 26.7.1、`rayon`、`walkdir`、`serde`）、Tauri v2、原生 JS（无框架）+ Vite。

## Global Constraints

- **不新增** Cargo 依赖（`image`/`avif`/`rayon`/`walkdir`/`serde` 均已在 `Cargo.toml`）
- **不新增** CSS（复用 `.modal-*`/`.output-dir-row`/`.depth-input`/`.modal-options`/`.progress-section`/`.folder-row`）
- **不改动** `ims.rs`、`Cargo.toml`、`styles.css`
- **不重构** 现有 `upgrade_ims_file`（仅提取 alpha-flatten 为共享辅助函数，行为不变）
- 格式码：JPEG=0, PNG=1, WebP=2, AVIF=3（与 `ImsImage.format`/`UpEntry.format` 一致）
- `image` crate **未启用 avif 特性**，AVIF 字节须用 `avif::decode` 解码（参照 `dedup/source.rs:120-122`）
- JPEG 重编码质量常量 `RESIZE_JPEG_QUALITY: u8 = 88`
- 最长边下限 64、默认 2560、不放大（`longest > max_edge` 才缩）、超 32768 前端 confirm
- 命令名 `start_batch_resize`；事件复用 `batch-progress`（`ProgressPayload { current, total, message, status }`）
- 文件结构码与提交信息沿用现有中文风格

---

## File Structure

| 文件 | 责任 | 改动类型 |
|---|---|---|
| `src-tauri/src/ims_pack.rs` | 核心：`resize_ims_file` + 辅助函数 + 配置类型 + 测试 | 修改 |
| `src-tauri/src/lib.rs` | Tauri 命令 `start_batch_resize` + 注册 + 导入 | 修改 |
| `index.html` | 工具菜单项 + `#resize-modal` 对话框 | 修改 |
| `src/app.js` | 菜单分发 + modal 控制函数 | 修改 |

---

## Task 1: 提取 `flatten_alpha_to_rgb` 辅助函数并新增 AVIF 解码辅助（`ims_pack.rs`）

**Files:**
- Modify: `src-tauri/src/ims_pack.rs:30-61`（`create_embedded_thumbnail` 提取 alpha-flatten）、新增 `decode_avif` 函数

**Interfaces:**
- Produces: `fn flatten_alpha_to_rgb(rgba: image::RgbaImage) -> image::RgbImage`（供本文件 `create_embedded_thumbnail` 与 Task 2 的 `resize_ims_file` 共用）
- Produces: `fn decode_avif(bytes: &[u8]) -> Result<image::DynamicImage, String>`（供 Task 2 调用；与 `dedup/source.rs:120-122` 实现等价，但本模块独立定义以避免跨 `dedup` 依赖）

- [ ] **Step 1: 在 `ims_pack.rs` 顶部 `use` 区追加 `image::RgbaImage`/`RgbImage` 已隐含可用，确认 `avif` crate 可见**

在 `ims_pack.rs` 现有 `use` 语句之后（第 11 行 `use std::path::{Path, PathBuf};` 之后）无需新增 `use`——`image::RgbaImage`/`image::RgbImage` 已通过 `image::` 路径全限定调用，`avif` crate 已是 `Cargo.toml` 依赖。跳过此步，仅在后续步骤中以全限定路径 `image::RgbaImage` 形式使用。

- [ ] **Step 2: 在 `create_embedded_thumbnail` 之前（第 30 行前）新增 `flatten_alpha_to_rgb` 函数**

```rust
/// 将带 alpha 的 RGBA 图像合成到白底并返回 RGB 图像（用于 JPEG 编码）。
///
/// 合成公式与 `create_embedded_thumbnail` 原有逻辑一致：
/// `out = (src * alpha + 255 * (255 - alpha) + 127) / 255`
fn flatten_alpha_to_rgb(rgba: image::RgbaImage) -> image::RgbImage {
    let width = rgba.width();
    let height = rgba.height();
    let mut rgb = image::RgbImage::new(width, height);
    for (column, row, source_pixel) in rgba.enumerate_pixels() {
        let alpha = source_pixel[3] as u16;
        let target_pixel = rgb.get_pixel_mut(column, row);
        for channel_index in 0..3 {
            let source_channel = source_pixel[channel_index] as u16;
            target_pixel[channel_index] =
                ((source_channel * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
    }
    rgb
}
```

- [ ] **Step 3: 重构 `create_embedded_thumbnail`（第 30-61 行）改为调用 `flatten_alpha_to_rgb`**

将 `create_embedded_thumbnail` 第 38-48 行（`let mut rgb = image::RgbImage::new(...)` 到 `for` 循环结束）替换为：

```rust
    let rgba = decoded
        .thumbnail(THUMBNAIL_MAX_SIZE, THUMBNAIL_MAX_SIZE)
        .to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    let rgb = flatten_alpha_to_rgb(rgba);
```

（即删掉原内联的 `for (column, row, source_pixel)` 循环，改调 `flatten_alpha_to_rgb`。第 31-34 行的 `decoded`/`rgba` 声明保留，第 49 行起的 `let mut bytes = Vec::new();` 起保留。）

- [ ] **Step 4: 在 `flatten_alpha_to_rgb` 之后新增 `decode_avif` 函数**

```rust
/// 解码 AVIF 字节为 `DynamicImage`（`image` crate 未启用 avif 特性时的专用路径）。
fn decode_avif(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    avif::decode(bytes).map_err(|error| error.to_string())
}
```

- [ ] **Step 5: 运行现有测试确认无回归**

Run: `cd src-tauri && cargo test --lib ims_pack::tests::embedded_thumbnail_trailer_contains_valid_metadata -- --exact`
Expected: PASS（该测试验证缩略图 trailer，alpha-flatten 提取不应改变其行为）

- [ ] **Step 6: 运行完整 `ims_pack` 测试模块**

Run: `cd src-tauri && cargo test --lib ims_pack`
Expected: PASS（4 个测试全过）

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/ims_pack.rs
git commit -m "refactor: extract flatten_alpha_to_rgb helper and add decode_avif in ims_pack"
```

---

## Task 2: 新增 `BatchResizeConfig` 与 `ResizeOutcome` 类型（`ims_pack.rs`）

**Files:**
- Modify: `src-tauri/src/ims_pack.rs`（在 `BatchExtractConfig` 之后第 151 行附近追加）

**Interfaces:**
- Consumes: `FolderConfig`（`ims_pack.rs:153-157`，已定义 `pub struct FolderConfig { pub path: String, pub depth: u32 }`）
- Produces: `pub struct BatchResizeConfig { folders: Vec<FolderConfig>, max_edge: u32, multi_threaded: bool }`
- Produces: `enum ResizeOutcome { Resized { sub_images_resized: usize }, Skipped }`

- [ ] **Step 1: 在 `BatchExtractConfig` 结构体定义之后（第 151 行 `}` 之后、第 153 行 `FolderConfig` 之前）插入 `BatchResizeConfig`**

```rust
/// 批量调整分辨率配置（从前端传入）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResizeConfig {
    pub folders: Vec<FolderConfig>,
    /// 最长边像素下限 64
    pub max_edge: u32,
    pub multi_threaded: bool,
}
```

- [ ] **Step 2: 在 `BatchResizeConfig` 之后插入 `ResizeOutcome` 枚举**

```rust
/// 单个 IMS 文件的调整结果
#[derive(Debug, Clone)]
pub enum ResizeOutcome {
    /// 有子图被缩小，文件已重写；`sub_images_resized` 为被缩小的子图数
    Resized { sub_images_resized: usize },
    /// 全部子图未超标，文件未改动
    Skipped,
}
```

- [ ] **Step 3: 编译确认类型可被序列化/导入**

Run: `cd src-tauri && cargo check`
Expected: 编译通过（无错误；可能有 unused 警告，因为尚未被使用，可忽略）

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/ims_pack.rs
git commit -m "feat: add BatchResizeConfig and ResizeOutcome types for batch resize"
```

---

## Task 3: 实现 `resize_ims_file` 核心函数（`ims_pack.rs`）+ 单元测试

**Files:**
- Modify: `src-tauri/src/ims_pack.rs`（在 `upgrade_ims_file` 之后第 518 行附近新增函数；在 `tests` 模块第 805 行附近新增测试）
- Test: `src-tauri/src/ims_pack.rs`（`#[cfg(test)] mod tests` 内）

**Interfaces:**
- Consumes: `crate::ims::{parse_ims, read_sub_image}`（`ims.rs`，已 `pub`）
- Consumes: `UpEntry`（`ims_pack.rs:628-634`）、`write_ims_entries`（`ims_pack.rs:553`）、`flatten_alpha_to_rgb`（Task 1）、`decode_avif`（Task 1）
- Produces: `pub fn resize_ims_file(filepath: &str, max_edge: u32) -> Result<ResizeOutcome, String>`

- [ ] **Step 1: 在 `ims_pack.rs` 顶部常量区（第 21 行 `THUMBNAIL_JPEG_QUALITY` 之后）新增质量常量**

```rust
const RESIZE_JPEG_QUALITY: u8 = 88;
```

- [ ] **Step 2: 在 `upgrade_ims_file` 函数之后（第 518 行 `}` 之后）新增 `resize_ims_file` 函数**

```rust
/// 就地调整 IMS 文件中子图的分辨率。
///
/// 逐子图判断：若子图原始最长边 `> max_edge`，按保持长宽比缩小到最长边等于
/// `max_edge`（`Lanczos3`）并重新编码；未超标子图保留原始字节不重编码。
/// AVIF 子图用 `avif` 解码后转 PNG 重新编码。只要任一子图被缩小，整个
/// IMS 文件会通过临时文件 + rename 原子重写；若全部子图未超标返回 `Skipped`。
/// 任一子图解码/编码失败则整文件跳过（返回 `Err`，原文件保持完好）。
pub fn resize_ims_file(filepath: &str, max_edge: u32) -> Result<ResizeOutcome, String> {
    use crate::ims::{parse_ims, read_sub_image};

    let ims_info = parse_ims(filepath)?;

    let mut entries: Vec<UpEntry> = Vec::with_capacity(ims_info.images.len());
    let mut resized_count: usize = 0;

    for img in &ims_info.images {
        let bytes = read_sub_image(filepath, img.data_offset, img.data_length)?;

        // 解码：AVIF 走 avif crate，其余走 image crate
        let (decoded, out_format, out_filename) = if img.format == 3 {
            // AVIF → 解码后转 PNG
            let d = decode_avif(&bytes)
                .map_err(|e| format!("解码 AVIF 子图 {} 失败: {}", img.filename, e))?;
            let new_name = with_png_extension(&img.filename);
            (d, 1u8, new_name)
        } else {
            let d = image::load_from_memory(&bytes)
                .map_err(|e| format!("解码子图 {} 失败: {}", img.filename, e))?;
            (d, img.format, img.filename.clone())
        };

        let longest = decoded.width().max(decoded.height());
        if longest <= max_edge {
            // 未超标：原样放回（保留原字节）
            entries.push(UpEntry {
                filename: img.filename.clone(),
                format: img.format,
                bytes,
                is_cover: img.is_cover,
            });
            continue;
        }

        // 超标：按比例缩小（longest 对应边 = max_edge）
        let scale = max_edge as f64 / longest as f64;
        let new_w = std::cmp::max(1, (decoded.width() as f64 * scale).round() as u32);
        let new_h = std::cmp::max(1, (decoded.height() as f64 * scale).round() as u32);
        let resized = decoded
            .resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3);

        // 编码到目标格式
        let (final_bytes, final_format, final_filename) =
            encode_resized(&resized, out_format, &out_filename)?;

        entries.push(UpEntry {
            filename: final_filename,
            format: final_format,
            bytes: final_bytes,
            is_cover: img.is_cover,
        });
        resized_count += 1;
    }

    if resized_count == 0 {
        return Ok(ResizeOutcome::Skipped);
    }

    // 封面排第一
    entries.sort_by_key(|e| !e.is_cover);

    let tmp_path = format!("{}.tmp_resize", filepath);
    write_ims_entries(&tmp_path, &entries)
        .map_err(|e| {
            // 清理失败的临时文件
            let _ = std::fs::remove_file(&tmp_path);
            e
        })?;
    std::fs::remove_file(filepath).map_err(|e| format!("删除旧文件失败: {}", e))?;
    std::fs::rename(&tmp_path, filepath).map_err(|e| format!("替换文件失败: {}", e))?;

    Ok(ResizeOutcome::Resized {
        sub_images_resized: resized_count,
    })
}

/// 将文件名的扩展名替换为 `.png`（用于 AVIF 子图转 PNG 后的命名）。
fn with_png_extension(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    format!("{}.png", stem)
}

/// 将缩小后的 `DynamicImage` 按目标格式编码。
///
/// - JPEG(0)：若带 alpha 先合成到白底，再用 `JpegEncoder` q88 编码
/// - PNG(1)：直接 `write_to` PNG（保留 alpha）
/// - 其余格式统一按 PNG 编码（与现有 WebP/AVIF→PNG 惯例一致）
fn encode_resized(
    image: &image::DynamicImage,
    out_format: u8,
    out_filename: &str,
) -> Result<(Vec<u8>, u8, String), String> {
    match out_format {
        0 => {
            // JPEG：无 alpha 支持，先 flatten
            let rgb = if matches!(image.color(), image::ColorType::Rgba8) {
                flatten_alpha_to_rgb(image.to_rgba8())
            } else {
                image.to_rgb8()
            };
            let (w, h) = (rgb.width(), rgb.height());
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, RESIZE_JPEG_QUALITY)
                .encode(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
                .map_err(|e| format!("JPEG 编码 {} 失败: {}", out_filename, e))?;
            Ok((bytes, 0u8, out_filename.to_string()))
        }
        _ => {
            // PNG（含原 PNG 与 AVIF 转码）
            let mut bytes = Vec::new();
            image
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .map_err(|e| format!("PNG 编码 {} 失败: {}", out_filename, e))?;
            Ok((bytes, 1u8, with_png_extension(out_filename)))
        }
    }
}
```

- [ ] **Step 3: 在 `tests` 模块（第 805 行 `mod tests {` 内）末尾（第 872 行 `}` 之前）新增测试辅助与用例**

先在 `mod tests {` 顶部 `use super::*;` 之后追加测试辅助：
```rust
    use crate::ims::{parse_ims, read_sub_image};
    use tempfile::tempdir;

    /// 构造一个含指定子图的 IMS 临时文件，返回 (临时目录, ims 路径)。
    fn make_ims_with_sub_images(sub_images: &[(u32, u32, u8)]) -> (tempfile::TempDir, String) {
        // sub_images: [(width, height, format)]，format 0=JPEG,1=PNG
        let dir = tempdir().unwrap();
        let mut entries: Vec<UpEntry> = Vec::new();
        for (i, &(w, h, fmt)) in sub_images.iter().enumerate() {
            let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                w, h, image::Rgb([100, 120, 140]),
            ));
            let mut bytes = Vec::new();
            let (ext, format_byte) = if fmt == 0 {
                img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
                    .unwrap();
                ("jpg", 0u8)
            } else {
                img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                    .unwrap();
                ("png", 1u8)
            };
            let filename = if i == 0 {
                format!("cover.{}", ext)
            } else {
                format!("cover_{:05}_.{}", i, ext)
            };
            entries.push(UpEntry {
                filename,
                format: format_byte,
                bytes,
                is_cover: i == 0,
            });
        }
        let ims_path = dir.path().join("test.ims.jpg").to_string_lossy().to_string();
        // 用 write_ims_entries 写出 IMS 容器
        write_ims_entries(&ims_path, &entries).unwrap();
        (dir, ims_path)
    }
```

然后追加测试用例：
```rust
    #[test]
    fn resize_skips_when_all_sub_images_within_max_edge() {
        let (_dir, ims_path) = make_ims_with_sub_images(&[(100, 100, 0)]);
        let original_bytes = std::fs::read(&ims_path).unwrap();
        let outcome = resize_ims_file(&ims_path, 200).unwrap();
        assert!(matches!(outcome, ResizeOutcome::Skipped));
        // 文件字节未变
        assert_eq!(std::fs::read(&ims_path).unwrap(), original_bytes);
    }

    #[test]
    fn resize_resizes_oversized_jpeg_and_preserves_aspect() {
        let (_dir, ims_path) = make_ims_with_sub_images(&[(800, 600, 0)]);
        let outcome = resize_ims_file(&ims_path, 400).unwrap();
        match outcome {
            ResizeOutcome::Resized { sub_images_resized } => assert_eq!(sub_images_resized, 1),
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        // 800×600 → longest=800 → scale=0.5 → 400×300
        assert_eq!((decoded.width(), decoded.height()), (400, 300));
    }

    #[test]
    fn resize_preserves_undersized_sub_image_bytes() {
        // 两张子图：800×600 超标 + 100×100 未超标
        let (_dir, ims_path) = make_ims_with_sub_images(&[(800, 600, 0), (100, 100, 1)]);
        // 记录未超标子图的原始字节（index 1）
        let info_before = parse_ims(&ims_path).unwrap();
        let small_before =
            read_sub_image(&ims_path, info_before.images[1].data_offset, info_before.images[1].data_length)
                .unwrap();
        resize_ims_file(&ims_path, 400).unwrap();
        // 重写后重新解析，未超标子图字节应与原始一致
        let info_after = parse_ims(&ims_path).unwrap();
        let small_after =
            read_sub_image(&ims_path, info_after.images[1].data_offset, info_after.images[1].data_length)
                .unwrap();
        assert_eq!(small_before, small_after);
    }

    #[test]
    fn resize_returns_err_on_corrupt_sub_image() {
        let (_dir, ims_path) = make_ims_with_sub_images(&[(800, 600, 0)]);
        // 破坏文件：把封面字节区改坏（在数据区写垃圾）
        let mut data = std::fs::read(&ims_path).unwrap();
        // 向数据区前 100 字节写垃圾（保留尾部索引，使 parse_ims 仍成功但解码失败）
        for b in &mut data[0..100] {
            *b = 0xFF;
        }
        std::fs::write(&ims_path, &data).unwrap();
        let result = resize_ims_file(&ims_path, 400);
        assert!(result.is_err(), "corrupt sub-image should error whole file");
    }
```

- [ ] **Step 4: 运行新测试，确认 `resize_skips_*` 与 `resize_resizes_*` 通过、`resize_returns_err_*` 通过**

Run: `cd src-tauri && cargo test --lib ims_pack::tests::resize`
Expected: 4 个 resize 测试通过。若 `resize_returns_err_on_corrupt_sub_image` 因封面被破坏导致 `parse_ims` 失败而非解码失败，调整破坏策略：把索引区保留，仅在 `data_start` 之后小范围写垃圾。若仍不稳定，可改为构造一个 format=0 但字节为非 JPEG 的 `UpEntry` 手工写入 IMS 来精确触发解码失败——替换测试体为：
```rust
    #[test]
    fn resize_returns_err_on_undecodable_sub_image() {
        let dir = tempdir().unwrap();
        let ims_path = dir.path().join("bad.ims.jpg").to_string_lossy().to_string();
        // 构造一张"封面"为合法 100×100 JPEG + 一张"子图"为无效字节
        let mut cover_bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(100, 100, image::Rgb([10, 20, 30])))
            .write_to(&mut std::io::Cursor::new(&mut cover_bytes), image::ImageFormat::Jpeg)
            .unwrap();
        let entries = vec![
            UpEntry { filename: "cover.jpg".into(), format: 0, bytes: cover_bytes, is_cover: true },
            UpEntry { filename: "cover_00001_.jpg".into(), format: 0, bytes: b"not a jpeg".to_vec(), is_cover: false },
        ];
        write_ims_entries(&ims_path, &entries).unwrap();
        let result = resize_ims_file(&ims_path, 50);
        assert!(result.is_err());
    }
```

- [ ] **Step 5: 运行完整 `ims_pack` 测试模块确认无回归**

Run: `cd src-tauri && cargo test --lib ims_pack`
Expected: 全部通过（原有 4 + 新增 4 = 8 个）

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/ims_pack.rs
git commit -m "feat: implement resize_ims_file with per-sub-image downscale and tests"
```

---

## Task 4: 新增 `start_batch_resize` Tauri 命令（`lib.rs`）

**Files:**
- Modify: `src-tauri/src/lib.rs:26-30`（导入）、`lib.rs:1174` 之后（新增命令）、`lib.rs:1575`（注册）

**Interfaces:**
- Consumes: `resize_ims_file`、`BatchResizeConfig`、`ResizeOutcome`（Task 2/3，从 `ims_pack` 导入）
- Consumes: `scan_ims_files`、`ProgressPayload`、`ImsCache`、`IMS_WRITE_LOCK`（`lib.rs` 已有）
- Produces: `#[tauri::command] async fn start_batch_resize(app, config) -> Result<String, String>`

- [ ] **Step 1: 扩展 `use ims_pack::{...}` 导入（第 26-30 行）**

将第 26-30 行的 `use ims_pack::{ ... };` 替换为（在 `upgrade_ims_file` 之后、`BatchConfig` 之前插入新导入）：

```rust
use ims_pack::{
    compute_output_path, load_image_as_entry, pack_ims, resize_ims_file, save_ims_entries_atomic,
    scan_ims_files, scan_multiple_folders, scan_multiple_folders_with_options, upgrade_ims_file,
    BatchConfig, BatchExtractConfig, BatchResizeConfig, ImageGroup, ProgressPayload,
    ResizeOutcome, UpEntry,
};
```

- [ ] **Step 2: 在 `start_batch_upgrade` 函数之后（第 1310 行 `}` 之后，`start_batch_convert` 第 1314 行之前）新增 `start_batch_resize` 命令**

```rust
#[tauri::command]
async fn start_batch_resize(
    app: tauri::AppHandle,
    config: BatchResizeConfig,
) -> Result<String, String> {
    if config.max_edge < 64 {
        return Err("最长边分辨率不能小于 64".into());
    }

    // 扫描所有文件夹的 IMS 文件并按路径去重（保留顺序）
    let mut files: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for folder in &config.folders {
        for fp in scan_ims_files(&folder.path, folder.depth) {
            if seen.insert(fp.clone()) {
                files.push(fp);
            }
        }
    }
    let total = files.len() as u32;

    if total == 0 {
        return Err("未发现 IMS 文件".into());
    }

    let total_msg = format!("共发现 {} 个 IMS 文件", total);
    let _ = app.emit(
        "batch-progress",
        ProgressPayload {
            current: 0,
            total,
            message: total_msg.clone(),
            status: "running".into(),
        },
    );

    let app_handle = app.clone();
    let max_edge = config.max_edge;

    std::thread::spawn(move || {
        let result = if config.multi_threaded {
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

            let completed = AtomicU32::new(0);
            let has_error = AtomicBool::new(false);

            files.par_iter().for_each(|fp| {
                let fname = Path::new(fp)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(fp);
                let _guard = IMS_WRITE_LOCK.lock();
                let outcome = resize_ims_file(fp, max_edge);
                let message = match &outcome {
                    Ok(ResizeOutcome::Resized { sub_images_resized }) => {
                        format!("已调整 {} (缩小 {} 张子图)", fname, sub_images_resized)
                    }
                    Ok(ResizeOutcome::Skipped) => format!("跳过 {} (无子图超标)", fname),
                    Err(e) => format!("失败 {}: {}", fname, e),
                };
                if outcome.is_err() {
                    has_error.store(true, Ordering::SeqCst);
                } else {
                    // 成功重写后失效 ImsCache
                    if let Some(cache) = app_handle.try_state::<ImsCache>() {
                        if let Ok(mut map) = cache.0.lock() {
                            map.remove(fp);
                        }
                    }
                }
                let current = completed.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current,
                        total,
                        message,
                        status: "running".into(),
                    },
                );
            });

            if has_error.load(Ordering::SeqCst) {
                Err("有文件处理失败".into())
            } else {
                Ok(())
            }
        } else {
            let mut completed: u32 = 0;
            let mut has_error = false;
            for fp in &files {
                let fname = Path::new(fp)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(fp);
                let _guard = IMS_WRITE_LOCK.lock();
                let outcome = resize_ims_file(fp, max_edge);
                let message = match &outcome {
                    Ok(ResizeOutcome::Resized { sub_images_resized }) => {
                        format!("已调整 {} (缩小 {} 张子图)", fname, sub_images_resized)
                    }
                    Ok(ResizeOutcome::Skipped) => format!("跳过 {} (无子图超标)", fname),
                    Err(e) => format!("失败 {}: {}", fname, e),
                };
                if outcome.is_err() {
                    has_error = true;
                } else if let ResizeOutcome::Resized { .. } = outcome.unwrap() {
                    if let Some(cache) = app_handle.try_state::<ImsCache>() {
                        if let Ok(mut map) = cache.0.lock() {
                            map.remove(fp);
                        }
                    }
                }
                completed += 1;
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: completed,
                        total,
                        message,
                        status: "running".into(),
                    },
                );
            }
            if has_error {
                Err("有文件处理失败".into())
            } else {
                Ok(())
            }
        };

        match result {
            Ok(()) => {
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: total,
                        total,
                        message: format!("全部完成，共 {} 个文件", total),
                        status: "completed".into(),
                    },
                );
            }
            Err(e) => {
                let _ = app_handle.emit(
                    "batch-progress",
                    ProgressPayload {
                        current: total,
                        total,
                        message: e,
                        status: "error".into(),
                    },
                );
            }
        }
    });

    Ok(total_msg)
}
```

注：`HashSet` 已在第 12 行 `use std::collections::{HashMap, HashSet};` 导入。`Path` 已在第 13 行导入。`IMS_WRITE_LOCK` 在第 53 行定义。`ImsCache` 在第 34 行定义。

- [ ] **Step 3: 在 `generate_handler!` 列表中注册命令（第 1575 行 `start_batch_upgrade,` 之后追加）**

将第 1575 行 `start_batch_upgrade,` 改为：
```rust
            start_batch_upgrade,
            start_batch_resize,
```

- [ ] **Step 4: 编译确认**

Run: `cd src-tauri && cargo check`
Expected: 编译通过（无错误；`resize_ims_file`/`BatchResizeConfig`/`ResizeOutcome` 已被使用，无 unused 警告）

- [ ] **Step 5: 运行全部后端测试确认无回归**

Run: `cd src-tauri && cargo test --lib`
Expected: 全部通过

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: add start_batch_resize Tauri command with progress and cache invalidation"
```

---

## Task 5: 前端 — 工具菜单项与 `#resize-modal` 对话框（`index.html`）

**Files:**
- Modify: `index.html:53`（菜单项）、`index.html:386`（modal）

**Interfaces:**
- Produces: `#resize-modal` 及其内部 ID（`resize-folder-list`、`btn-resize-add-folder`、`resize-max-edge`、`chk-resize-multi`、`resize-progress-fill`、`resize-progress-text`、`btn-start-resize`、`btn-cancel-resize`、`btn-resize-close`）；菜单项 `data-action="batch-resize"`

- [ ] **Step 1: 在工具菜单 `batch-upgrade` 之后新增菜单项（第 53 行之后）**

将第 53-54 行：
```html
            <div class="menu-action" data-action="batch-upgrade">批量升级 IMS 版本...</div>
            <div class="menu-separator"></div>
```
改为：
```html
            <div class="menu-action" data-action="batch-upgrade">批量升级 IMS 版本...</div>
            <div class="menu-action" data-action="batch-resize">批量调整分辨率...</div>
            <div class="menu-separator"></div>
```

- [ ] **Step 2: 在 `#upgrade-modal` 之后（第 386 行 `</div>` 之后、第 388 行 `<!-- 缩略图右键菜单` 之前）新增 `#resize-modal`**

```html

  <!-- 批量调整分辨率模态框 -->
  <div id="resize-modal" class="modal-overlay" style="display:none">
    <div class="modal-container">
      <div class="modal-header">
        <span>批量调整 IMS 子图分辨率</span>
        <button class="btn-modal-close" id="btn-resize-close">&times;</button>
      </div>
      <div class="modal-body">
        <div id="resize-folder-list"></div>
        <button id="btn-resize-add-folder" class="btn-secondary">+ 添加文件夹</button>
        <div class="output-dir-row">
          <label>最长边分辨率 (px)</label>
          <input class="depth-input" id="resize-max-edge" type="number" min="64" value="2560" title="仅缩小最长边超过此值的子图，保持长宽比；不放大" />
        </div>
        <div class="modal-options">
          <label><input type="checkbox" id="chk-resize-multi"> 多线程</label>
        </div>
        <div class="progress-section">
          <div class="progress-bar-container">
            <div class="progress-bar-fill" id="resize-progress-fill"></div>
          </div>
          <div class="progress-text" id="resize-progress-text">准备就绪</div>
        </div>
      </div>
      <div class="modal-footer">
        <button id="btn-start-resize" class="btn-primary">开始调整</button>
        <button id="btn-cancel-resize" class="btn-secondary">关闭</button>
      </div>
    </div>
  </div>
```

- [ ] **Step 3: 提交**

```bash
git add index.html
git commit -m "feat: add batch resize menu entry and modal in index.html"
```

---

## Task 6: 前端 — 菜单分发与 modal 控制逻辑（`src/app.js`）

**Files:**
- Modify: `src/app.js:926-928`（switch 分支）、`src/app.js:1162` 之后（新增 modal 控制函数）

**Interfaces:**
- Consumes: `invoke`、`listen`（已在前文导入使用）、`$`/`$$`/`escapeHtml`（`app.js` 已有）
- Consumes: `pick_folder` Tauri 命令（已有）
- Produces: `openResizeModal`/`closeResizeModal`/`addResizeFolderRow`/`collectResizeConfig`/`startBatchResize`/`setupResizeEvents`、`resizeUnlisten`

- [ ] **Step 1: 在菜单分发 switch 中新增 `batch-resize` 分支（第 927 行 `openUpgradeModal();` 之后）**

将第 926-928 行：
```js
        case 'batch-upgrade':
          openUpgradeModal();
          break;
```
改为：
```js
        case 'batch-upgrade':
          openUpgradeModal();
          break;
        case 'batch-resize':
          openResizeModal();
          break;
```

- [ ] **Step 2: 在 `setupBatchEvents` 函数之后（第 1162 行 `}` 之后，`escapeHtml` 第 1164 行之前）新增 resize modal 控制函数**

```js

// ===== 批量调整分辨率模态框 =====
let resizeUnlisten = null;

function openResizeModal() {
  const modal = $('#resize-modal');
  modal.style.display = 'flex';
  $('#resize-folder-list').innerHTML = '';
  addResizeFolderRow();
  $('#resize-max-edge').value = '2560';
  $('#resize-progress-fill').style.width = '0%';
  $('#resize-progress-text').textContent = '准备就绪';
  $('#btn-start-resize').disabled = false;
  $('#btn-start-resize').textContent = '开始调整';
  setupResizeEvents();
}

function closeResizeModal() {
  $('#resize-modal').style.display = 'none';
  if (resizeUnlisten) {
    resizeUnlisten();
    resizeUnlisten = null;
  }
}

function addResizeFolderRow(path = '', depth = 0) {
  const row = document.createElement('div');
  row.className = 'folder-row';
  row.innerHTML = `
    <input class="path-input" type="text" placeholder="文件夹路径..." value="${escapeHtml(path)}" />
    <button class="btn-browse">浏览</button>
    <label title="0=无限深度">深度 <input class="depth-input" type="number" min="0" max="100" value="${depth}" /></label>
    <button class="btn-remove" title="移除">×</button>
  `;
  row.querySelector('.btn-browse').addEventListener('click', async () => {
    try {
      const dir = await invoke('pick_folder');
      if (dir) row.querySelector('.path-input').value = dir;
    } catch (e) { console.error('选择文件夹失败:', e); }
  });
  row.querySelector('.btn-remove').addEventListener('click', () => {
    row.remove();
    if ($('#resize-folder-list').children.length === 0) addResizeFolderRow();
  });
  $('#resize-folder-list').appendChild(row);
}

function collectResizeConfig() {
  const folders = [];
  $$('#resize-folder-list .folder-row').forEach(row => {
    const path = row.querySelector('.path-input').value.trim();
    const depth = parseInt(row.querySelector('.depth-input').value) || 0;
    if (path) folders.push({ path, depth: Math.min(100, Math.max(0, depth)) });
  });
  const maxEdge = parseInt($('#resize-max-edge').value) || 0;
  return {
    folders,
    max_edge: maxEdge,
    multi_threaded: $('#chk-resize-multi').checked,
  };
}

async function startBatchResize() {
  const config = collectResizeConfig();
  if (config.folders.length === 0) {
    alert('请至少添加一个文件夹');
    return;
  }
  if (config.max_edge < 64) {
    alert('最长边分辨率不能小于 64');
    return;
  }
  if (config.max_edge > 32768) {
    if (!confirm('最长边超过 32768，确定继续吗？')) return;
  }

  const btn = $('#btn-start-resize');
  btn.disabled = true;
  btn.textContent = '调整中...';
  $('#resize-progress-text').textContent = '正在扫描 IMS 文件...';

  try {
    if (resizeUnlisten) resizeUnlisten();
    resizeUnlisten = await listen('batch-progress', (event) => {
      const p = event.payload;
      const pct = p.total > 0 ? Math.round((p.current / p.total) * 100) : 0;
      $('#resize-progress-fill').style.width = pct + '%';
      $('#resize-progress-text').textContent = `[${p.current}/${p.total}] ${p.message}`;

      if (p.status === 'completed' || p.status === 'error') {
        btn.disabled = false;
        btn.textContent = '开始调整';
        $('#resize-progress-text').textContent =
          (p.status === 'completed' ? '✅ ' : '⚠️ ') + p.message;
      }
    });

    await invoke('start_batch_resize', { config });
  } catch (err) {
    alert('启动调整失败: ' + err);
    btn.disabled = false;
    btn.textContent = '开始调整';
  }
}

function setupResizeEvents() {
  $('#btn-resize-add-folder').onclick = () => addResizeFolderRow();
  $('#btn-resize-close').onclick = () => closeResizeModal();
  $('#btn-cancel-resize').onclick = () => closeResizeModal();
  $('#btn-start-resize').onclick = () => startBatchResize();
  if ($('#resize-folder-list').children.length === 0) addResizeFolderRow();
}
```

- [ ] **Step 3: 提交**

```bash
git add src/app.js
git commit -m "feat: wire batch resize modal controls and progress listener in app.js"
```

---

## Task 7: 集成构建与端到端验证

**Files:**
- 无代码改动；本任务为构建与手工验收

- [ ] **Step 1: 后端完整测试**

Run: `cd src-tauri && cargo test --lib`
Expected: 全部通过（含 `ims_pack` 8 个测试 + 其他模块原有测试）

- [ ] **Step 2: 前端构建检查**

Run: `cd f:/Workspace/imsViewer && npm run build`
Expected: Vite 构建成功（无 JS 语法错误、无未定义引用）

- [ ] **Step 3: Tauri 开发模式启动确认**

Run: `cd f:/Workspace/imsViewer && npm run tauri dev`（在能交互的终端；如本环境无 GUI，跳过并标注"需手工验收"）
Expected: 应用启动，工具菜单出现"批量调整分辨率..."，点击弹出 `#resize-modal`

- [ ] **Step 4: 手工验收清单（若可交互）**

准备一个含混合尺寸 IMS 文件的测试目录（例如一张 800×600 JPEG + 一张 100×100 PNG 的 IMS，及一张全部 ≤2560 的 IMS）：
- 工具 → 批量调整分辨率 → 添加文件夹 → 最长边 2560 → 多线程 → 开始调整
- 验证：进度条推进、消息显示"跳过"/"已调整 N 张子图"、终态 ✅
- 重新 `open_file` 处理后的超标 IMS：子图可正常显示、尺寸已缩小、缩略图刷新
- 验证最长边=64 边界（alert 拦截 < 64）、最长边=40000 警告 confirm

- [ ] **Step 5: 最终提交（若有验收修复）**

若手工验收发现问题，修复后提交：
```bash
git add -A
git commit -m "fix: batch resize integration adjustments"
```
若无问题，无需提交，计划完成。

---

## Self-Review 已完成

- **Spec coverage**：设计文档 §2 决策表逐条对应到 Task（就地修改→Task3、AVIF→PNG→Task3 `decode_avif`+`encode_resized`、Lanczos3→Task3、未超标保留原字节→Task3 `continue` 分支、子图级判定→Task3 `resized_count==0→Skipped`、多文件夹→Task5/6、max_edge 范围→Task4 校验+Task6 前端校验、失败整文件跳过→Task3 `Err` 提前返回、不备份→无相关代码、新 modal→Task5）。§5 关键细节（IMS_WRITE_LOCK→Task4、ImsCache 失效→Task4、alpha 处理→Task1+Task3、resize_exact→Task3、`.tmp_resize`→Task3、范围校验→Task4+Task6）均覆盖。§7 测试策略的单元测试全部落到 Task3 Step3。§9 改动文件清单与 File Structure 一致。
- **Placeholder scan**：无 TBD/TODO/"add appropriate error handling"；所有代码步骤均给出完整代码；测试用例给出完整断言。
- **Type consistency**：`resize_ims_file(filepath: &str, max_edge: u32) -> Result<ResizeOutcome, String>`（Task2 定义、Task3 实现、Task4 调用一致）；`BatchResizeConfig { folders, max_edge, multi_threaded }`（Task2 定义、Task4 参数、Task6 `collectResizeConfig` 返回一致）；`ResizeOutcome::Resized { sub_images_resized }`/`Skipped`（Task2 定义、Task3 返回、Task4 `match` 一致）；`encode_resized`/`with_png_extension`/`flatten_alpha_to_rgb`/`decode_avif` 签名在定义与调用处一致；前端 ID（`resize-folder-list`/`resize-max-edge`/`btn-start-resize` 等）在 HTML（Task5）与 JS（Task6）一致。
