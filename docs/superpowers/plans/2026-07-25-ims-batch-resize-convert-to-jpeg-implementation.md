# IMS 批量调分辨率工具 "同时转为 JPEG" 选项 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在已有 IMS 批量调分辨率工具中新增"同时转为 JPEG"开关：勾选后将 IMS 文件中所有非 JPEG 子图（PNG/WebP/AVIF）重新编码为 JPEG q90（无论是否超标），未超标 JPEG 子图保留原字节，以大幅减小文件体积。

**Architecture:** 对现有 `resize_ims_file`（`ims_pack.rs`）就地扩展：`BatchResizeConfig` 加 `convert_to_jpeg` 字段（`#[serde(default)]` 向后兼容），`resize_ims_file` 加 `convert_to_jpeg` 参数，`ResizeOutcome::Resized` 加 `sub_images_converted` 计数，`encode_resized` 加 `jpeg_quality` 参数（原 JPEG 用 88、转 JPEG 用 90），新增 `CONVERT_JPEG_QUALITY`/`with_jpg_extension`。`lib.rs` 的 `start_batch_resize` 传参并适配进度消息。前端 modal 加复选框、`collectResizeConfig` 加字段。

**Tech Stack:** Rust 2021（`image` 0.25 jpeg/png/webp、`avif-rs`）、Tauri v2、原生 JS。

## Global Constraints

- **不新增** Cargo 依赖（`image`/`avif` 均已在 `Cargo.toml`）
- **不新增** CSS（复用 `.modal-options` checkbox label）
- **不改动** `ims.rs`、`Cargo.toml`、`styles.css`
- 格式码：JPEG=0, PNG=1, WebP=2, AVIF=3
- `image` crate 未启用 avif 特性，AVIF 字节用 `decode_avif`（已存在于 `ims_pack.rs:52`）
- `RESIZE_JPEG_QUALITY: u8 = 88`（已存在，用于原 JPEG 子图缩小重编码）
- `CONVERT_JPEG_QUALITY: u8 = 90`（新增，用于非 JPEG 子图转 JPEG）
- `convert_to_jpeg` 用 `#[serde(default)]`，默认 `false`（向后兼容，未勾选行为与现有工具完全一致）
- 转 JPEG 后子图扩展名改 `.jpg`（新增 `with_jpg_extension`）
- 未超标 JPEG 子图保留原字节不动（不二次编码）
- 命令名 `start_batch_resize` 不变；事件复用 `batch-progress`
- 文件结构码与提交信息沿用现有中文风格

---

## File Structure

| 文件 | 责任 | 改动类型 |
|---|---|---|
| `src-tauri/src/ims_pack.rs` | 核心：config 字段、outcome 计数、转码逻辑、编码质量参数、测试 | 修改 |
| `src-tauri/src/lib.rs` | 命令传参 + 进度消息适配 | 修改 |
| `index.html` | modal 加复选框 | 修改 |
| `src/app.js` | `collectResizeConfig` 加字段 | 修改 |

---

## Task 1: 扩展 `BatchResizeConfig`/`ResizeOutcome` 类型 + 新增常量与 `with_jpg_extension`（`ims_pack.rs`）

**Files:**
- Modify: `src-tauri/src/ims_pack.rs:169-185`（config + outcome）、`ims_pack.rs:22`（常量）、`ims_pack.rs:646`（with_jpg_extension）

**Interfaces:**
- Produces: `BatchResizeConfig { folders, max_edge, multi_threaded, convert_to_jpeg: bool }`（`convert_to_jpeg` 带 `#[serde(default)]`）
- Produces: `ResizeOutcome::Resized { sub_images_resized: usize, sub_images_converted: usize }` / `Skipped`
- Produces: `const CONVERT_JPEG_QUALITY: u8 = 90`
- Produces: `fn with_jpg_extension(filename: &str) -> String`

- [ ] **Step 1: 在常量区（第 22 行 `RESIZE_JPEG_QUALITY` 之后）新增 `CONVERT_JPEG_QUALITY`**

```rust
const CONVERT_JPEG_QUALITY: u8 = 90;
```

- [ ] **Step 2: 修改 `BatchResizeConfig`（第 171-176 行）加 `convert_to_jpeg` 字段**

将：
```rust
pub struct BatchResizeConfig {
    pub folders: Vec<FolderConfig>,
    /// 最长边像素下限 64
    pub max_edge: u32,
    pub multi_threaded: bool,
}
```
改为：
```rust
pub struct BatchResizeConfig {
    pub folders: Vec<FolderConfig>,
    /// 最长边像素下限 64
    pub max_edge: u32,
    pub multi_threaded: bool,
    /// 是否将所有非 JPEG 子图转为 JPEG（q90）
    #[serde(default)]
    pub convert_to_jpeg: bool,
}
```

- [ ] **Step 3: 修改 `ResizeOutcome`（第 180-185 行）加 `sub_images_converted` 计数**

将：
```rust
pub enum ResizeOutcome {
    /// 有子图被缩小，文件已重写；`sub_images_resized` 为被缩小的子图数
    Resized { sub_images_resized: usize },
    /// 全部子图未超标，文件未改动
    Skipped,
}
```
改为：
```rust
pub enum ResizeOutcome {
    /// 文件已重写；`sub_images_resized` 为被缩小的子图数，
    /// `sub_images_converted` 为被转为 JPEG 的非 JPEG 子图数（未勾选转 JPEG 时恒为 0）
    Resized {
        sub_images_resized: usize,
        sub_images_converted: usize,
    },
    /// 全部子图未超标且无需转码，文件未改动
    Skipped,
}
```

- [ ] **Step 4: 在 `with_png_extension`（第 640-646 行）之后新增 `with_jpg_extension`**

```rust
/// 将文件名的扩展名替换为 `.jpg`（用于子图转 JPEG 后的命名）。
fn with_jpg_extension(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    format!("{}.jpg", stem)
}
```

- [ ] **Step 5: 编译确认（会有 `resize_ims_file`/`start_batch_resize` 因签名/字段未同步而报错，属预期——下一任务修复；本步仅确认类型本身无误）**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | head -30`
Expected: 报错指出 `ResizeOutcome::Resized` 字段不匹配 / `resize_ims_file` 调用处问题（这些是 Task 2/3 要改的）。确认报错**仅**来自这些未同步点，而非新加类型本身的语法错误。若新类型有语法错误先修复。

注：此步不提交（编译未通过），直接进入 Task 2 一起完成后编译。或先注释掉报错处的调用也可，但更简单是直接推进 Task 2/3 后统一编译。本任务仅做类型与常量、辅助函数定义。

- [ ] **Step 6: 提交（在 Task 2/3 完成后编译通过再一起提交，或此处先提交类型定义）**

建议：本任务与 Task 2 一起完成后提交。此处仅准备类型定义。

---

## Task 2: 重构 `resize_ims_file` 支持转 JPEG + 更新 `encode_resized`（`ims_pack.rs`）

**Files:**
- Modify: `src-tauri/src/ims_pack.rs:561-637`（`resize_ims_file`）、`ims_pack.rs:648-685`（`encode_resized`）

**Interfaces:**
- Consumes: `CONVERT_JPEG_QUALITY`、`RESIZE_JPEG_QUALITY`、`with_jpg_extension`、`with_png_extension`、`decode_avif`、`flatten_alpha_to_rgb`、`ResizeOutcome`（Task 1）
- Produces: `pub fn resize_ims_file(filepath: &str, max_edge: u32, convert_to_jpeg: bool) -> Result<ResizeOutcome, String>`
- Produces: `fn encode_resized(image: &image::DynamicImage, out_format: u8, out_filename: &str, jpeg_quality: u8) -> Result<(Vec<u8>, u8, String), String>`

- [ ] **Step 1: 重构 `resize_ims_file`（第 561-637 行）为支持转 JPEG 的版本**

完整替换为：
```rust
/// 就地调整 IMS 文件中子图的分辨率，可选将非 JPEG 子图转为 JPEG。
///
/// 逐子图判断：若子图原始最长边 `> max_edge`，按保持长宽比缩小到最长边等于
/// `max_edge`（`Lanczos3`）并重新编码；未超标子图默认保留原始字节。
/// 当 `convert_to_jpeg` 为 true 时，所有非 JPEG 子图（PNG/WebP/AVIF）无论是否
/// 超标都重新编码为 JPEG（q90）；已是 JPEG 且未超标的子图保留原字节不动。
/// AVIF 子图用 `avif` 解码；未勾选转 JPEG 时 AVIF 转 PNG，勾选时转 JPEG。
/// 只要任一子图被缩小或转码，整个 IMS 文件会通过临时文件 + rename 原子重写；
/// 若全部子图未超标且无需转码返回 `Skipped`。
/// 任一子图解码/编码失败则整文件跳过（返回 `Err`，原文件保持完好）。
pub fn resize_ims_file(
    filepath: &str,
    max_edge: u32,
    convert_to_jpeg: bool,
) -> Result<ResizeOutcome, String> {
    use crate::ims::{parse_ims, read_sub_image};

    let ims_info = parse_ims(filepath)?;

    let mut entries: Vec<UpEntry> = Vec::with_capacity(ims_info.images.len());
    let mut resized_count: usize = 0;
    let mut converted_count: usize = 0;

    for img in &ims_info.images {
        let bytes = read_sub_image(filepath, img.data_offset, img.data_length)?;
        let is_jpeg = img.format == 0;

        // 解码：AVIF 走 avif crate，其余走 image crate
        let decoded = if img.format == 3 {
            decode_avif(&bytes)
                .map_err(|e| format!("解码 AVIF 子图 {} 失败: {}", img.filename, e))?
        } else {
            image::load_from_memory(&bytes)
                .map_err(|e| format!("解码子图 {} 失败: {}", img.filename, e))?
        };

        let longest = decoded.width().max(decoded.height());
        let oversized = longest > max_edge;

        // 未超标且（已是 JPEG 或未勾选转码）→ 保留原字节
        if !oversized && (is_jpeg || !convert_to_jpeg) {
            entries.push(UpEntry {
                filename: img.filename.clone(),
                format: img.format,
                bytes,
                is_cover: img.is_cover,
            });
            continue;
        }

        // 需要重编码：确定目标格式
        // 勾选转 JPEG → 统一 JPEG(0)；未勾选 → AVIF 转 PNG(1)，其余保持原格式
        let target_format: u8 = if convert_to_jpeg {
            0
        } else if img.format == 3 {
            1
        } else {
            img.format
        };

        // 确定待编码的图（超标则先缩小，未超标转码则图不变尺寸）
        let image_to_encode = if oversized {
            let scale = max_edge as f64 / longest as f64;
            let new_w = std::cmp::max(1, (decoded.width() as f64 * scale).round() as u32);
            let new_h = std::cmp::max(1, (decoded.height() as f64 * scale).round() as u32);
            decoded.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3)
        } else {
            decoded
        };

        // JPEG 质量：原 JPEG 子图用 88，非 JPEG 转 JPEG 用 90
        let jpeg_quality = if is_jpeg {
            RESIZE_JPEG_QUALITY
        } else {
            CONVERT_JPEG_QUALITY
        };

        let (final_bytes, final_format, final_filename) =
            encode_resized(&image_to_encode, target_format, &img.filename, jpeg_quality)?;

        entries.push(UpEntry {
            filename: final_filename,
            format: final_format,
            bytes: final_bytes,
            is_cover: img.is_cover,
        });
        if oversized {
            resized_count += 1;
        }
        if convert_to_jpeg && !is_jpeg {
            converted_count += 1;
        }
    }

    if resized_count == 0 && converted_count == 0 {
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
        sub_images_converted: converted_count,
    })
}
```

注：原实现里 AVIF 解码后 `out_filename` 用 `with_png_extension` 预改名的逻辑已移除——扩展名统一由 `encode_resized` 根据 `target_format` 决定（JPEG→`.jpg`，PNG→`.png`），避免重复改名。

- [ ] **Step 2: 重构 `encode_resized`（第 648-685 行）加 `jpeg_quality` 参数并处理扩展名**

完整替换为：
```rust
/// 将 `DynamicImage` 按目标格式编码。
///
/// - JPEG(0)：若带 alpha 先合成到白底，再用 `JpegEncoder` 以 `jpeg_quality` 编码；
///   扩展名改为 `.jpg`
/// - PNG(1)：直接 `write_to` PNG（保留 alpha）；扩展名改为 `.png`
/// - 其余 `out_format` 值统一按 PNG 编码（与现有 WebP/AVIF→PNG 惯例一致）
fn encode_resized(
    image: &image::DynamicImage,
    out_format: u8,
    out_filename: &str,
    jpeg_quality: u8,
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
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, jpeg_quality)
                .encode(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
                .map_err(|e| format!("JPEG 编码 {} 失败: {}", out_filename, e))?;
            Ok((bytes, 0u8, with_jpg_extension(out_filename)))
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

注：JPEG 分支扩展名从 `out_filename.to_string()` 改为 `with_jpg_extension(out_filename)`——对原本就是 `.jpg` 的 JPEG 子图，`with_jpg_extension` 幂等（stem 不变），行为一致；对转 JPEG 的 PNG/WebP/AVIF 子图，扩展名正确变为 `.jpg`。

- [ ] **Step 3: 编译确认**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | head -30`
Expected: 报错仅来自 `start_batch_resize`（`lib.rs` 调用 `resize_ims_file` 少一个参数 + `ResizeOutcome::Resized` 字段不匹配）——这是 Task 3 要改的。`ims_pack.rs` 内部应无错。若 `ims_pack.rs` 内部有错先修复。

- [ ] **Step 4: 提交（与 Task 3 一起，或先提交 ims_pack 部分）**

建议与 Task 3 一起编译通过后提交。

---

## Task 3: 更新 `start_batch_resize` 传参与进度消息（`lib.rs`）

**Files:**
- Modify: `src-tauri/src/lib.rs:1314-1473`（`start_batch_resize`）

**Interfaces:**
- Consumes: `resize_ims_file(filepath, max_edge, convert_to_jpeg)`、`ResizeOutcome::Resized { sub_images_resized, sub_images_converted }`（Task 2）
- Produces: 更新后的 `start_batch_resize`（签名不变）

- [ ] **Step 1: 在 `start_batch_resize` 中提取 `convert_to_jpeg` 并传给 `resize_ims_file`**

在第 1351 行 `let max_edge = config.max_edge;` 之后加：
```rust
    let convert_to_jpeg = config.convert_to_jpeg;
```

- [ ] **Step 2: 修改 rayon 多线程分支（第 1367-1374 行）的调用与消息**

将：
```rust
                let outcome = resize_ims_file(fp, max_edge);
                let message = match &outcome {
                    Ok(ResizeOutcome::Resized { sub_images_resized }) => {
                        format!("已调整 {} (缩小 {} 张子图)", fname, sub_images_resized)
                    }
                    Ok(ResizeOutcome::Skipped) => format!("跳过 {} (无子图超标)", fname),
                    Err(e) => format!("失败 {}: {}", fname, e),
                };
```
改为：
```rust
                let outcome = resize_ims_file(fp, max_edge, convert_to_jpeg);
                let message = match &outcome {
                    Ok(ResizeOutcome::Resized {
                        sub_images_resized,
                        sub_images_converted,
                    }) => format!(
                        "已调整 {} ({})",
                        fname,
                        resize_summary(*sub_images_resized, *sub_images_converted)
                    ),
                    Ok(ResizeOutcome::Skipped) => format!("跳过 {} (无需处理)", fname),
                    Err(e) => format!("失败 {}: {}", fname, e),
                };
```

- [ ] **Step 3: 修改串行分支（第 1411-1418 行）的调用与消息（同上）**

将串行分支里相同的 `let outcome = resize_ims_file(fp, max_edge);` 与 `match &outcome { ... }` 块改为与 Step 2 相同的内容（调用加 `convert_to_jpeg`，消息用 `resize_summary`，Skipped 文案改"无需处理"）。

- [ ] **Step 4: 在 `start_batch_resize` 函数之前（第 1314 行 `#[tauri::command]` 之前）新增 `resize_summary` 辅助函数**

```rust
/// 生成单文件调整结果的进度摘要（缩小 N 张 / 转 JPEG M 张）。
fn resize_summary(resized: usize, converted: usize) -> String {
    match (resized, converted) {
        (r, 0) if r > 0 => format!("缩小 {} 张子图", r),
        (0, c) if c > 0 => format!("转 JPEG {} 张子图", c),
        (r, c) if r > 0 && c > 0 => format!("缩小 {} 张, 转 JPEG {} 张", r, c),
        _ => "无变化".to_string(),
    }
}
```

- [ ] **Step 5: 编译确认**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | tail -15`
Expected: 编译通过（无错误；可能有预先存在的 dead-code 警告，与本次无关）

- [ ] **Step 6: 运行全部后端测试确认无回归**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib 2>&1 | tail -15`
Expected: 全部通过（现有 resize 测试的 `ResizeOutcome::Resized` 构造需同步——见 Task 4；若现有测试因字段增加而编译失败，先推进 Task 4 再统一跑）

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/ims_pack.rs src-tauri/src/lib.rs
git commit -m "feat: add convert-to-JPEG support in resize_ims_file and start_batch_resize"
```

---

## Task 4: 更新/新增 `ims_pack.rs` 测试（适配新字段 + 覆盖转 JPEG 路径）

**Files:**
- Modify: `src-tauri/src/ims_pack.rs`（`#[cfg(test)] mod tests`，现有 resize 测试适配 + 新增转 JPEG 测试）

**Interfaces:**
- Consumes: `resize_ims_file(filepath, max_edge, convert_to_jpeg)`、`ResizeOutcome::Resized { sub_images_resized, sub_images_converted }`（Task 2）

- [ ] **Step 1: 适配现有 4 个 resize 测试的 `resize_ims_file` 调用（加第三参数 `false`）与 `ResizeOutcome::Resized` 匹配**

现有 4 个测试（`resize_skips_*`、`resize_resizes_oversized_jpeg_*`、`resize_preserves_undersized_*`、`resize_returns_err_*`、`resize_preserves_alpha_*`）中：
- 所有 `resize_ims_file(&ims_path, N)` 调用改为 `resize_ims_file(&ims_path, N, false)`
- 所有 `ResizeOutcome::Resized { sub_images_resized }` 匹配改为 `ResizeOutcome::Resized { sub_images_resized, .. }`（用 `..` 忽略新字段）

例如 `resize_resizes_oversized_jpeg_and_preserves_aspect` 中：
```rust
        let outcome = resize_ims_file(&ims_path, 400, false).unwrap();
        match outcome {
            ResizeOutcome::Resized { sub_images_resized, .. } => assert_eq!(sub_images_resized, 1),
            _ => panic!("expected Resized"),
        }
```

- [ ] **Step 2: 运行现有 resize 测试确认适配后通过**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib ims_pack::tests::resize 2>&1 | tail -12`
Expected: 5 个现有 resize 测试全部通过（行为未变，`convert_to_jpeg=false`）

- [ ] **Step 3: 新增转 JPEG 测试（在 `resize_preserves_alpha_for_png_sub_image` 之后追加）**

```rust
    /// 构造一个含单张指定尺寸/格式/是否带 alpha 子图的 IMS，返回 (临时目录, ims 路径)。
    fn make_ims_single(format: u8, w: u32, h: u32, rgba: bool) -> (tempfile::TempDir, String) {
        let dir = tempdir().unwrap();
        let (bytes, ext) = match (format, rgba) {
            (0, _) => {
                let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    w, h, image::Rgb([100, 120, 140]),
                ));
                let mut b = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Jpeg)
                    .unwrap();
                (b, "jpg")
            }
            (1, true) => {
                let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                    w, h, image::Rgba([100, 120, 140, 128]),
                ));
                let mut b = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png)
                    .unwrap();
                (b, "png")
            }
            (1, false) => {
                let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    w, h, image::Rgb([100, 120, 140]),
                ));
                let mut b = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png)
                    .unwrap();
                (b, "png")
            }
            _ => panic!("unsupported test format"),
        };
        let entries = vec![UpEntry {
            filename: format!("cover.{}", ext),
            format,
            bytes,
            is_cover: true,
        }];
        let ims_path = dir
            .path()
            .join(format!("test.ims.{}", ext))
            .to_string_lossy()
            .to_string();
        write_ims_entries(&ims_path, &entries).unwrap();
        (dir, ims_path)
    }

    #[test]
    fn convert_to_jpeg_converts_undersized_png() {
        // 未超标 PNG + convert_to_jpeg=true → 转 JPEG（不缩小）
        let (_dir, ims_path) = make_ims_single(1, 100, 100, false);
        let outcome = resize_ims_file(&ims_path, 400, true).unwrap();
        match outcome {
            ResizeOutcome::Resized {
                sub_images_resized,
                sub_images_converted,
            } => {
                assert_eq!(sub_images_resized, 0);
                assert_eq!(sub_images_converted, 1);
            }
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        assert_eq!(info.images[0].format, 0, "应转为 JPEG");
        assert!(info.images[0].filename.ends_with(".jpg"), "扩展名应为 .jpg");
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (100, 100), "未超标不缩小");
    }

    #[test]
    fn convert_to_jpeg_preserves_undersized_jpeg() {
        // 未超标 JPEG + convert_to_jpeg=true → Skipped（已是 JPEG 不二次编码）
        let (_dir, ims_path) = make_ims_single(0, 100, 100, false);
        let original = std::fs::read(&ims_path).unwrap();
        let outcome = resize_ims_file(&ims_path, 400, true).unwrap();
        assert!(matches!(outcome, ResizeOutcome::Skipped));
        assert_eq!(std::fs::read(&ims_path).unwrap(), original, "文件不应改动");
    }

    #[test]
    fn convert_to_jpeg_oversized_png_resizes_and_converts() {
        // 超标 PNG + convert_to_jpeg=true → 缩小 + 转 JPEG
        let (_dir, ims_path) = make_ims_single(1, 800, 600, false);
        let outcome = resize_ims_file(&ims_path, 400, true).unwrap();
        match outcome {
            ResizeOutcome::Resized {
                sub_images_resized,
                sub_images_converted,
            } => {
                assert_eq!(sub_images_resized, 1);
                assert_eq!(sub_images_converted, 1);
            }
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        assert_eq!(info.images[0].format, 0);
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (400, 300));
    }

    #[test]
    fn no_convert_keeps_undersized_png() {
        // 未超标 PNG + convert_to_jpeg=false → Skipped（回归：未勾选行为不变）
        let (_dir, ims_path) = make_ims_single(1, 100, 100, false);
        let original = std::fs::read(&ims_path).unwrap();
        let outcome = resize_ims_file(&ims_path, 400, false).unwrap();
        assert!(matches!(outcome, ResizeOutcome::Skipped));
        assert_eq!(std::fs::read(&ims_path).unwrap(), original);
    }

    #[test]
    fn convert_to_jpeg_flattens_alpha() {
        // 带 alpha 的 PNG + convert_to_jpeg=true → 转 JPEG 后无 alpha
        let (_dir, ims_path) = make_ims_single(1, 800, 600, true);
        let outcome = resize_ims_file(&ims_path, 400, true).unwrap();
        assert!(matches!(outcome, ResizeOutcome::Resized { .. }));
        let info = parse_ims(&ims_path).unwrap();
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_ne!(
            decoded.color(),
            image::ColorType::Rgba8,
            "转 JPEG 后 alpha 应被合成"
        );
    }
```

- [ ] **Step 4: 运行全部 resize 测试**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib ims_pack::tests:: 2>&1 | tail -20`
Expected: 全部通过（原有 8 + 新增 5 = 13 个 ims_pack 测试）

- [ ] **Step 5: 运行完整后端套件确认无回归**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib 2>&1 | tail -6`
Expected: 全部通过（原 34 + 新增 5 = 39 个）

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/ims_pack.rs
git commit -m "test: add convert-to-JPEG tests and adapt existing resize tests"
```

---

## Task 5: 前端 — modal 加复选框 + `collectResizeConfig` 加字段（`index.html` + `app.js`）

**Files:**
- Modify: `index.html:404`（`.modal-options`）、`src/app.js:1213-1226`（`collectResizeConfig`）

**Interfaces:**
- Produces: `#chk-resize-to-jpeg` 复选框；`collectResizeConfig` 返回 `convert_to_jpeg: bool`

- [ ] **Step 1: 在 `#resize-modal` 的 `.modal-options` 中（`index.html` 第 404 行 `chk-resize-multi` 之后）加复选框**

将：
```html
        <div class="modal-options">
          <label><input type="checkbox" id="chk-resize-multi"> 多线程</label>
        </div>
```
改为：
```html
        <div class="modal-options">
          <label><input type="checkbox" id="chk-resize-multi"> 多线程</label>
          <label><input type="checkbox" id="chk-resize-to-jpeg"> 同时转为 JPEG (q90，显著减小体积)</label>
        </div>
```

- [ ] **Step 2: 在 `app.js` 的 `collectResizeConfig`（第 1213-1226 行）加 `convert_to_jpeg` 字段**

将：
```js
  const maxEdge = parseInt($('#resize-max-edge').value) || 0;
  return {
    folders,
    max_edge: maxEdge,
    multi_threaded: $('#chk-resize-multi').checked,
  };
```
改为：
```js
  const maxEdge = parseInt($('#resize-max-edge').value) || 0;
  return {
    folders,
    max_edge: maxEdge,
    multi_threaded: $('#chk-resize-multi').checked,
    convert_to_jpeg: $('#chk-resize-to-jpeg').checked,
  };
```

- [ ] **Step 3: 前端构建验证**

Run: `cd f:/Workspace/imsViewer && npm run build 2>&1 | tail -8`
Expected: Vite 构建成功，无 JS 错误

- [ ] **Step 4: 提交**

```bash
git add index.html src/app.js
git commit -m "feat: add convert-to-JPEG checkbox in batch resize modal"
```

---

## Task 6: 集成构建与验证

**Files:**
- 无代码改动；本任务为构建与验证

- [ ] **Step 1: 后端完整测试**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib 2>&1 | tail -6`
Expected: 全部通过（39 个）

- [ ] **Step 2: cargo check 警告检查**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | tail -8`
Expected: 编译通过，无新增警告（仅预先存在的 5 个 dead-code 警告）

- [ ] **Step 3: 前端构建**

Run: `cd f:/Workspace/imsViewer && npm run build 2>&1 | tail -8`
Expected: Vite 构建成功

- [ ] **Step 4: 手工验收（若可交互）**

- 工具 → 批量调整分辨率 → 添加文件夹 → 勾选"同时转为 JPEG" → 开始调整
- 验证：PNG 子图转 JPEG（format=0、扩展名 `.jpg`）、体积显著下降、未超标 JPEG 子图未动
- 验证：不勾选时行为与之前一致（保持原格式）

- [ ] **Step 5: 最终提交（若有验收修复）**

无问题则无需提交。

---

## Self-Review 已完成

- **Spec coverage**：设计 §2 决策表逐条对应（作用范围→Task2 循环逻辑、独立于缩放→Task2 `!oversized && convert_to_jpeg` 分支、q90→Task1 `CONVERT_JPEG_QUALITY`、WebP/AVIF→JPEG+`.jpg`→Task2 `target_format=0`+`with_jpg_extension`、未超标 JPEG 保留→Task2 `is_jpeg || !convert_to_jpeg` 保留分支、复用 `encode_resized`→Task2 Step2）。§3 行为变化（Skipped 判定）→Task2 `resized_count==0 && converted_count==0`。§4.2 进度消息→Task3 `resize_summary`。§4.3/4.4 前端→Task5。§7 测试→Task4 全部 5 个新测试。
- **Placeholder scan**：无 TBD/TODO；所有代码步骤给出完整代码；测试用例给出完整断言与辅助函数。
- **Type consistency**：`resize_ims_file(filepath, max_edge, convert_to_jpeg)`（Task2 定义、Task3 调用、Task4 测试一致）；`ResizeOutcome::Resized { sub_images_resized, sub_images_converted }`（Task1 定义、Task2 返回、Task3 match、Task4 测试一致）；`encode_resized(image, out_format, out_filename, jpeg_quality)`（Task2 定义与调用一致）；`resize_summary(resized, converted)`（Task3 定义与调用一致）；`convert_to_jpeg`（Task1 config 字段、Task3 提取、Task5 前端字段一致）。
