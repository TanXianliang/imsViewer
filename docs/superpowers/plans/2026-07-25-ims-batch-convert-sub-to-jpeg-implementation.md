# IMS 批量转换工具 "子图片转为 JPEG" 选项 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在批量转换工具中新增"子图片转为 JPEG"开关：勾选后打包/合并时将所有非 JPEG 子图（PNG/WebP/AVIF）重新编码为 JPEG q90，封面保持原格式，已是 JPEG 的子图原样保留，覆盖 pack 和 merge 两条路径。

**Architecture:** `BatchConfig` 加 `sub_images_to_jpeg` 字段（`#[serde(default)]` 向后兼容）；`pack_ims` 加参数，子图（`i != cover_index`）非 JPEG 时解码（AVIF 用 `decode_avif`）→ `encode_resized` 转 JPEG q90；`load_image_as_entry` 加 `to_jpeg` 参数；`merge_into_existing_ims` 加参数，仅 `insert_candidate` 的子图转 JPEG（MD5 匹配用图保持原字节保证去重正确）；`start_batch_convert` 传参；前端 modal 加复选框。复用现有 `encode_resized`/`decode_avif`/`CONVERT_JPEG_QUALITY`/`with_jpg_extension`。

**Tech Stack:** Rust 2021（`image` 0.25 jpeg/png/webp、`avif-rs`）、Tauri v2、原生 JS。

## Global Constraints

- **不新增** Cargo 依赖（`image`/`avif` 均已在 `Cargo.toml`）
- **不新增** CSS（复用 `.modal-options` checkbox label）
- **不改动** `ims.rs`、`Cargo.toml`、`styles.css`
- 格式码：JPEG=0, PNG=1, WebP=2, AVIF=3
- `image` crate 未启用 avif 特性，AVIF 字节用 `decode_avif`（已存在于 `ims_pack.rs:52`）
- `CONVERT_JPEG_QUALITY: u8 = 90`（已存在于 `ims_pack.rs:23`）
- `sub_images_to_jpeg` 用 `#[serde(default)]`，默认 `false`（向后兼容，未勾选行为与现有工具完全一致）
- 转 JPEG 后子图扩展名改 `.jpg`（复用 `with_jpg_extension`，已存在于 `ims_pack.rs:690`）
- 仅子图转 JPEG，封面保持原格式（封面 WebP/AVIF 仍转 PNG，不受开关影响）
- 已是 JPEG 的子图原样保留不重编码
- merge 路径仅 `insert_candidate == true` 的图转 JPEG；MD5 匹配用图保持原字节
- 复用 `encode_resized(image, out_format, out_filename, jpeg_quality)`（已存在于 `ims_pack.rs:704`）
- 命令名 `start_batch_convert` 不变；事件复用 `batch-progress`
- 文件结构码与提交信息沿用现有中文风格

---

## File Structure

| 文件 | 责任 | 改动类型 |
|---|---|---|
| `src-tauri/src/ims_pack.rs` | 核心：config 字段、pack_ims/load_image_as_entry 转 JPEG 逻辑、测试 | 修改 |
| `src-tauri/src/lib.rs` | merge_into_existing_ims 加参数与 to_jpeg 区分、start_batch_convert 传参 | 修改 |
| `src-tauri/src/dedup/tests.rs` | pack_ims 调用加 false 参数 | 修改 |
| `index.html` | 批量转换 modal 加复选框 | 修改 |
| `src/app.js` | collectBatchConfig 加字段 | 修改 |

---

## Task 1: `BatchConfig` 加字段 + `pack_ims` 支持子图转 JPEG（`ims_pack.rs`）

**Files:**
- Modify: `src-tauri/src/ims_pack.rs:151-159`（BatchConfig）、`ims_pack.rs:214-218`（pack_ims 签名）、`ims_pack.rs:271-299`（pack_ims 逐文件处理逻辑）

**Interfaces:**
- Consumes: `decode_avif`（`ims_pack.rs:52`）、`encode_resized`（`ims_pack.rs:704`）、`CONVERT_JPEG_QUALITY`（`ims_pack.rs:23`）
- Produces: `BatchConfig { ..., sub_images_to_jpeg: bool }`（`#[serde(default)]`）
- Produces: `pub fn pack_ims(image_paths: &[String], cover_index: usize, output_path: &str, sub_images_to_jpeg: bool) -> Result<(), String>`

- [ ] **Step 1: `BatchConfig` 加 `sub_images_to_jpeg` 字段（第 158 行 `merge_existing_ims` 之后）**

将第 151-159 行：
```rust
pub struct BatchConfig {
    pub folders: Vec<FolderConfig>,
    pub delete_source: bool,
    pub multi_threaded: bool,
    pub output_dir: Option<String>,
    pub preserve_structure: bool,
    #[serde(default)]
    pub merge_existing_ims: bool,
}
```
改为：
```rust
pub struct BatchConfig {
    pub folders: Vec<FolderConfig>,
    pub delete_source: bool,
    pub multi_threaded: bool,
    pub output_dir: Option<String>,
    pub preserve_structure: bool,
    #[serde(default)]
    pub merge_existing_ims: bool,
    /// 是否将所有非 JPEG 子图转为 JPEG（q90）
    #[serde(default)]
    pub sub_images_to_jpeg: bool,
}
```

- [ ] **Step 2: `pack_ims` 签名加 `sub_images_to_jpeg` 参数（第 214-218 行）**

将：
```rust
pub fn pack_ims(
    image_paths: &[String],
    cover_index: usize,
    output_path: &str,
) -> Result<(), String> {
```
改为：
```rust
pub fn pack_ims(
    image_paths: &[String],
    cover_index: usize,
    output_path: &str,
    sub_images_to_jpeg: bool,
) -> Result<(), String> {
```

- [ ] **Step 3: `pack_ims` 逐文件处理逻辑加子图转 JPEG 分支（第 273-299 行）**

将第 273-299 行（`// WebP/AVIF → 自动转 PNG` 块到 `entries.push` 前）：
```rust
        // WebP/AVIF → 自动转 PNG
        let (final_bytes, final_format, final_filename) = if format == 2 || format == 3 {
            let img = image::load_from_memory(&raw_bytes).map_err(|e| {
                format!(
                    "解码 {} 失败: {} (格式: {})",
                    path,
                    e,
                    if format == 2 { "WebP" } else { "AVIF" }
                )
            })?;
            let mut png_bytes = Vec::new();
            img.write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .map_err(|e| format!("PNG 编码失败 {}: {}", path, e))?;
            let new_name = format!(
                "{}.png",
                Path::new(&filename)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("image")
            );
            (png_bytes, 1u8, new_name)
        } else {
            (raw_bytes, format, filename)
        };
```
改为：
```rust
        let is_cover = i == cover_index;
        // 子图转 JPEG（勾选且非 JPEG）；否则 WebP/AVIF → 自动转 PNG
        let (final_bytes, final_format, final_filename) = if !is_cover
            && sub_images_to_jpeg
            && format != 0
        {
            // 子图转 JPEG：解码（AVIF 用 decode_avif）→ encode_resized 转 JPEG q90
            let decoded = if format == 3 {
                decode_avif(&raw_bytes)
                    .map_err(|e| format!("解码 AVIF {} 失败: {}", path, e))?
            } else {
                image::load_from_memory(&raw_bytes)
                    .map_err(|e| format!("解码 {} 失败: {}", path, e))?
            };
            encode_resized(&decoded, 0, &filename, CONVERT_JPEG_QUALITY)?
        } else if format == 2 || format == 3 {
            let img = image::load_from_memory(&raw_bytes).map_err(|e| {
                format!(
                    "解码 {} 失败: {} (格式: {})",
                    path,
                    e,
                    if format == 2 { "WebP" } else { "AVIF" }
                )
            })?;
            let mut png_bytes = Vec::new();
            img.write_to(
                &mut std::io::Cursor::new(&mut png_bytes),
                image::ImageFormat::Png,
            )
            .map_err(|e| format!("PNG 编码失败 {}: {}", path, e))?;
            let new_name = format!(
                "{}.png",
                Path::new(&filename)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("image")
            );
            (png_bytes, 1u8, new_name)
        } else {
            (raw_bytes, format, filename)
        };
```

注：`encode_resized` 返回 `(Vec<u8>, u8, String)`，与 `if/else` 各分支的 `(final_bytes, final_format, final_filename)` 三元组类型一致，可直接用 `?` 解包。

- [ ] **Step 4: 编译确认（会有 lib.rs/dedup/tests.rs 调用点因签名变化报错，属预期——Task 2/3 修复）**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | grep -E "ims_pack|error" | head -20`
Expected: `ims_pack.rs` 内部无错；报错仅来自 `lib.rs`（`pack_ims` 少参数）和 `dedup/tests.rs`（`pack_ims` 少参数）。若 `ims_pack.rs` 内部有错先修复。

- [ ] **Step 5: 提交（与 Task 2/3 一起编译通过后提交，或先提交 ims_pack 部分）**

建议与 Task 2/3 一起完成后提交。

---

## Task 2: `load_image_as_entry` 加 `to_jpeg` 参数（`ims_pack.rs`）

**Files:**
- Modify: `src-tauri/src/ims_pack.rs:858`（load_image_as_entry 签名）、`ims_pack.rs:898-923`（WebP/AVIF 转码逻辑）

**Interfaces:**
- Consumes: `decode_avif`、`encode_resized`、`CONVERT_JPEG_QUALITY`（同 Task 1）
- Produces: `pub fn load_image_as_entry(path: &Path, is_cover: bool, to_jpeg: bool) -> Result<UpEntry, String>`

- [ ] **Step 1: `load_image_as_entry` 签名加 `to_jpeg` 参数（第 858 行）**

将：
```rust
pub fn load_image_as_entry(path: &Path, is_cover: bool) -> Result<UpEntry, String> {
```
改为：
```rust
pub fn load_image_as_entry(path: &Path, is_cover: bool, to_jpeg: bool) -> Result<UpEntry, String> {
```

- [ ] **Step 2: 在 WebP/AVIF 转码逻辑前插入"to_jpeg 且非 JPEG → 转 JPEG"分支（第 898-923 行）**

将第 898-923 行：
```rust
    let (bytes, final_format, final_filename) = if format == 2 || format == 3 {
        let img = image::load_from_memory(&raw_bytes).map_err(|e| {
            format!(
                "Failed to decode {}: {} ({})",
                path.display(),
                e,
                if format == 2 { "WebP" } else { "AVIF" }
            )
        })?;
        let mut png_bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| format!("Failed to encode PNG {}: {}", path.display(), e))?;
        let new_name = format!(
            "{}.png",
            Path::new(&filename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("image")
        );
        (png_bytes, 1u8, new_name)
    } else {
        (raw_bytes, format, filename)
    };
```
改为：
```rust
    let (bytes, final_format, final_filename) = if to_jpeg && format != 0 {
        // 转 JPEG：解码（AVIF 用 decode_avif）→ encode_resized 转 JPEG q90
        let decoded = if format == 3 {
            decode_avif(&raw_bytes)
                .map_err(|e| format!("Failed to decode AVIF {}: {}", path.display(), e))?
        } else {
            image::load_from_memory(&raw_bytes)
                .map_err(|e| format!("Failed to decode {}: {}", path.display(), e))?
        };
        encode_resized(&decoded, 0, &filename, CONVERT_JPEG_QUALITY)?
    } else if format == 2 || format == 3 {
        let img = image::load_from_memory(&raw_bytes).map_err(|e| {
            format!(
                "Failed to decode {}: {} ({})",
                path.display(),
                e,
                if format == 2 { "WebP" } else { "AVIF" }
            )
        })?;
        let mut png_bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| format!("Failed to encode PNG {}: {}", path.display(), e))?;
        let new_name = format!(
            "{}.png",
            Path::new(&filename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("image")
        );
        (png_bytes, 1u8, new_name)
    } else {
        (raw_bytes, format, filename)
    };
```

- [ ] **Step 3: 编译确认（报错仅来自 lib.rs 的 load_image_as_entry 调用点，Task 4 修复）**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | grep -E "ims_pack|error" | head -20`
Expected: `ims_pack.rs` 内部无错；报错仅来自 `lib.rs`（`load_image_as_entry` 少参数 + `pack_ims` 少参数）和 `dedup/tests.rs`。

- [ ] **Step 4: 提交（与 Task 1/3/4 一起）**

---

## Task 3: `dedup/tests.rs` 的 `pack_ims` 调用加参数

**Files:**
- Modify: `src-tauri/src/dedup/tests.rs:53`

**Interfaces:**
- Consumes: `pack_ims(image_paths, cover_index, output_path, sub_images_to_jpeg)`（Task 1）

- [ ] **Step 1: 查看 `dedup/tests.rs:53` 的 `pack_ims` 调用，加第四参数 `false`**

读取第 53 行附近的 `pack_ims(...)` 调用，在 `output_path` 参数后加 `, false`。例如若原为：
```rust
    pack_ims(
        &image_paths,
        0,
        &output_path,
    )
```
改为：
```rust
    pack_ims(
        &image_paths,
        0,
        &output_path,
        false,
    )
```

- [ ] **Step 2: 提交（与 Task 4 一起）**

---

## Task 4: `merge_into_existing_ims` 加参数 + `start_batch_convert` 传参（`lib.rs`）

**Files:**
- Modify: `src-tauri/src/lib.rs:848-852`（merge_into_existing_ims 签名）、`lib.rs:878-890`（加载循环）、`lib.rs:1585-1588`（merge 调用）、`lib.rs:1613`（pack_ims 调用）

**Interfaces:**
- Consumes: `load_image_as_entry(path, is_cover, to_jpeg)`（Task 2）、`pack_ims(..., sub_images_to_jpeg)`（Task 1）、`BatchConfig.sub_images_to_jpeg`（Task 1）
- Produces: `fn merge_into_existing_ims(output_path, match_paths, insert_paths, sub_images_to_jpeg) -> Result<MergeExistingResult, String>`

- [ ] **Step 1: `merge_into_existing_ims` 签名加 `sub_images_to_jpeg` 参数（第 848-852 行）**

将：
```rust
fn merge_into_existing_ims(
    output_path: &str,
    match_paths: &[String],
    insert_paths: &[String],
) -> Result<MergeExistingResult, String> {
```
改为：
```rust
fn merge_into_existing_ims(
    output_path: &str,
    match_paths: &[String],
    insert_paths: &[String],
    sub_images_to_jpeg: bool,
) -> Result<MergeExistingResult, String> {
```

- [ ] **Step 2: 加载循环按 `insert_keys.contains` 决定 `to_jpeg`（第 878-890 行）**

将第 878-890 行：
```rust
    for path in match_paths {
        if !seen_sources.insert(source_path_key(path)) {
            continue;
        }
        let entry = load_image_as_entry(Path::new(path), false)?;
        let md5 = md5_hex(&entry.bytes);
        sources.push(MergeSourceEntry {
            path: path.clone(),
            entry,
            md5,
            insert_candidate: insert_keys.contains(&source_path_key(path)),
        });
    }
```
改为：
```rust
    for path in match_paths {
        if !seen_sources.insert(source_path_key(path)) {
            continue;
        }
        let is_insert = insert_keys.contains(&source_path_key(path));
        // 仅真正要插入的子图转 JPEG；用于 MD5 匹配的封面保持原字节（保证去重正确）
        let entry = load_image_as_entry(Path::new(path), false, is_insert && sub_images_to_jpeg)?;
        let md5 = md5_hex(&entry.bytes);
        sources.push(MergeSourceEntry {
            path: path.clone(),
            entry,
            md5,
            insert_candidate: is_insert,
        });
    }
```

- [ ] **Step 3: `start_batch_convert` 的 merge 调用加参数（第 1585-1588 行）**

将：
```rust
                let merge = merge_into_existing_ims(
                    &task.output_path,
                    &task.match_paths,
                    &task.sub_image_paths,
                )?;
```
改为：
```rust
                let merge = merge_into_existing_ims(
                    &task.output_path,
                    &task.match_paths,
                    &task.sub_image_paths,
                    config.sub_images_to_jpeg,
                )?;
```

注：需确认 `config` 在 `convert` 闭包内可用。`start_batch_convert` 的 `convert` 闭包当前捕获了 `delete_source`/`multi`/`merge_existing_ims` 等局部变量（见第 1622 行附近 `let delete_source = config.delete_source;` 等）。需在闭包前加 `let sub_images_to_jpeg = config.sub_images_to_jpeg;` 并在闭包内使用该局部变量（与 `merge_existing_ims` 的处理方式一致）。

- [ ] **Step 4: `start_batch_convert` 的 `pack_ims` 调用加参数（第 1613 行）**

将：
```rust
            pack_ims(&image_paths, 0, &task.output_path)?;
```
改为：
```rust
            pack_ims(&image_paths, 0, &task.output_path, sub_images_to_jpeg)?;
```

- [ ] **Step 5: 编译确认**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | tail -10`
Expected: 编译通过（无错误；仅预先存在的 dead-code 警告）

- [ ] **Step 6: 运行全部后端测试确认无回归**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib 2>&1 | tail -6`
Expected: 全部通过（41 个）

- [ ] **Step 7: 提交（Task 1-4 一起）**

```bash
git add src-tauri/src/ims_pack.rs src-tauri/src/lib.rs src-tauri/src/dedup/tests.rs
git commit -m "feat: add sub-images-to-JPEG option in batch convert (pack + merge paths)"
```

---

## Task 5: `ims_pack.rs` 测试（pack_ims + load_image_as_entry 转 JPEG）

**Files:**
- Modify: `src-tauri/src/ims_pack.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `pack_ims(..., sub_images_to_jpeg)`（Task 1）、`load_image_as_entry(..., to_jpeg)`（Task 2）

- [ ] **Step 1: 新增测试辅助与用例（在 `mod tests` 末尾追加）**

先加辅助（构造散图文件到临时目录）：
```rust
    /// 在临时目录写一张指定尺寸/格式的图片文件，返回 (临时目录, 文件路径)。
    fn write_temp_image(dir: &tempfile::TempDir, name: &str, w: u32, h: u32, rgba: bool) -> String {
        let path = dir.path().join(name);
        if rgba {
            let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                w, h, image::Rgba([100, 120, 140, 128]),
            ));
            img.save(&path).unwrap();
        } else {
            let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                w, h, image::Rgb([100, 120, 140]),
            ));
            img.save(&path).unwrap();
        }
        path.to_string_lossy().to_string()
    }
```

再加测试用例：
```rust
    #[test]
    fn pack_ims_converts_sub_image_png_to_jpeg() {
        let dir = tempdir().unwrap();
        let cover = write_temp_image(&dir, "cover.jpg", 100, 100, false);
        let sub = write_temp_image(&dir, "cover_00001_.png", 200, 200, false);
        let output = dir.path().join("out.ims.jpg").to_string_lossy().to_string();
        pack_ims(&[cover, sub], 0, &output, true).unwrap();
        let info = crate::ims::parse_ims(&output).unwrap();
        // 封面（index 0）保持 JPEG，子图（index 1）转 JPEG
        assert_eq!(info.images[0].format, 0, "封面应保持 JPEG");
        assert_eq!(info.images[1].format, 0, "子图应转为 JPEG");
        assert!(info.images[1].filename.ends_with(".jpg"), "子图扩展名应为 .jpg");
    }

    #[test]
    fn pack_ims_preserves_jpeg_sub_image_when_converting() {
        let dir = tempdir().unwrap();
        let cover = write_temp_image(&dir, "cover.jpg", 100, 100, false);
        let sub = write_temp_image(&dir, "cover_00001_.jpg", 200, 200, false);
        let sub_original = std::fs::read(&sub).unwrap();
        let output = dir.path().join("out.ims.jpg").to_string_lossy().to_string();
        pack_ims(&[cover, sub], 0, &output, true).unwrap();
        let info = crate::ims::parse_ims(&output).unwrap();
        let sub_bytes = crate::ims::read_sub_image(
            &output,
            info.images[1].data_offset,
            info.images[1].data_length,
        )
        .unwrap();
        assert_eq!(sub_bytes, sub_original, "已是 JPEG 的子图不应二次编码");
    }

    #[test]
    fn pack_ims_no_convert_keeps_png_sub_image() {
        let dir = tempdir().unwrap();
        let cover = write_temp_image(&dir, "cover.jpg", 100, 100, false);
        let sub = write_temp_image(&dir, "cover_00001_.png", 200, 200, false);
        let output = dir.path().join("out.ims.png").to_string_lossy().to_string();
        pack_ims(&[cover, sub], 0, &output, false).unwrap();
        let info = crate::ims::parse_ims(&output).unwrap();
        assert_eq!(info.images[1].format, 1, "未勾选时 PNG 子图应保持 PNG");
    }

    #[test]
    fn pack_ims_convert_sub_image_flattens_alpha() {
        let dir = tempdir().unwrap();
        let cover = write_temp_image(&dir, "cover.jpg", 100, 100, false);
        let sub = write_temp_image(&dir, "cover_00001_.png", 200, 200, true);
        let output = dir.path().join("out.ims.jpg").to_string_lossy().to_string();
        pack_ims(&[cover, sub], 0, &output, true).unwrap();
        let info = crate::ims::parse_ims(&output).unwrap();
        let sub_bytes = crate::ims::read_sub_image(
            &output,
            info.images[1].data_offset,
            info.images[1].data_length,
        )
        .unwrap();
        let decoded = image::load_from_memory(&sub_bytes).unwrap();
        assert_ne!(
            decoded.color(),
            image::ColorType::Rgba8,
            "转 JPEG 后 alpha 应被合成"
        );
    }

    #[test]
    fn load_image_as_entry_to_jpeg_converts_png() {
        let dir = tempdir().unwrap();
        let png_path = write_temp_image(&dir, "img.png", 100, 100, false);
        let entry = load_image_as_entry(std::path::Path::new(&png_path), false, true).unwrap();
        assert_eq!(entry.format, 0, "应转为 JPEG");
        assert!(entry.filename.ends_with(".jpg"), "扩展名应为 .jpg");
    }

    #[test]
    fn load_image_as_entry_to_jpeg_preserves_jpeg() {
        let dir = tempdir().unwrap();
        let jpg_path = write_temp_image(&dir, "img.jpg", 100, 100, false);
        let original = std::fs::read(&jpg_path).unwrap();
        let entry = load_image_as_entry(std::path::Path::new(&jpg_path), false, true).unwrap();
        assert_eq!(entry.bytes, original, "已是 JPEG 不应二次编码");
    }
```

- [ ] **Step 2: 运行新增测试**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib ims_pack::tests::pack_ims 2>&1 | tail -10`
Expected: 4 个 pack_ims 测试通过

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib ims_pack::tests::load_image_as_entry 2>&1 | tail -6`
Expected: 2 个 load_image_as_entry 测试通过

- [ ] **Step 3: 运行完整 ims_pack 模块**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib ims_pack 2>&1 | tail -6`
Expected: 全部通过（原 15 + 新增 6 = 21 个）

- [ ] **Step 4: 运行完整后端套件**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib 2>&1 | tail -6`
Expected: 全部通过（原 41 + 新增 6 = 47 个）

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/ims_pack.rs
git commit -m "test: add pack_ims and load_image_as_entry convert-to-JPEG tests"
```

---

## Task 6: 前端 — 批量转换 modal 加复选框 + `collectBatchConfig` 加字段（`index.html` + `app.js`）

**Files:**
- Modify: `index.html:298`（`.modal-options`）、`src/app.js:1094-1110`（`collectBatchConfig`）

**Interfaces:**
- Produces: `#chk-sub-to-jpeg` 复选框；`collectBatchConfig` 返回 `sub_images_to_jpeg: bool`

- [ ] **Step 1: 在批量转换 modal 的 `.modal-options` 中（`index.html` 第 298 行 `chk-merge-existing-ims` 之后）加复选框**

将第 294-299 行：
```html
        <div class="modal-options">
          <label><input type="checkbox" id="chk-delete-source"> 转换成功后删除源文件</label>
          <label><input type="checkbox" id="chk-multi-thread"> 多线程转换</label>
          <label><input type="checkbox" id="chk-preserve-structure" checked> 保持目录结构</label>
          <label><input type="checkbox" id="chk-merge-existing-ims"> 目标 IMS 已存在时按 MD5 合并子图片</label>
        </div>
```
改为：
```html
        <div class="modal-options">
          <label><input type="checkbox" id="chk-delete-source"> 转换成功后删除源文件</label>
          <label><input type="checkbox" id="chk-multi-thread"> 多线程转换</label>
          <label><input type="checkbox" id="chk-preserve-structure" checked> 保持目录结构</label>
          <label><input type="checkbox" id="chk-merge-existing-ims"> 目标 IMS 已存在时按 MD5 合并子图片</label>
          <label><input type="checkbox" id="chk-sub-to-jpeg"> 子图片转为 JPEG (q90，减小体积)</label>
        </div>
```

- [ ] **Step 2: 在 `app.js` 的 `collectBatchConfig`（第 1094-1110 行）加 `sub_images_to_jpeg` 字段**

将第 1099-1109 行：
```js
  return {
    folders,
    delete_source: $('#chk-delete-source').checked,
    multi_threaded: $('#chk-multi-thread').checked,
    output_dir: outputDir || null,
    preserve_structure: $('#chk-preserve-structure').checked,
    merge_existing_ims: $('#chk-merge-existing-ims').checked,
  };
```
改为：
```js
  return {
    folders,
    delete_source: $('#chk-delete-source').checked,
    multi_threaded: $('#chk-multi-thread').checked,
    output_dir: outputDir || null,
    preserve_structure: $('#chk-preserve-structure').checked,
    merge_existing_ims: $('#chk-merge-existing-ims').checked,
    sub_images_to_jpeg: $('#chk-sub-to-jpeg').checked,
  };
```

- [ ] **Step 3: 前端构建验证**

Run: `cd f:/Workspace/imsViewer && npm run build 2>&1 | tail -6`
Expected: Vite 构建成功，无 JS 错误

- [ ] **Step 4: 提交**

```bash
git add index.html src/app.js
git commit -m "feat: add sub-images-to-JPEG checkbox in batch convert modal"
```

---

## Task 7: 集成构建与验证

**Files:**
- 无代码改动；本任务为构建与验证

- [ ] **Step 1: 后端完整测试**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib 2>&1 | tail -6`
Expected: 全部通过（47 个）

- [ ] **Step 2: cargo check 警告检查**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | tail -6`
Expected: 编译通过，无新增警告

- [ ] **Step 3: 前端构建**

Run: `cd f:/Workspace/imsViewer && npm run build 2>&1 | tail -6`
Expected: Vite 构建成功

- [ ] **Step 4: 手工验收（若可交互）**

- 工具 → 批量转换 → 添加文件夹 → 勾选"子图片转为 JPEG" → 开始转换
- 验证：PNG 子图转 JPEG（format=0、扩展名 `.jpg`）、封面保持原格式、已是 JPEG 子图未动
- 验证：不勾选时行为与之前一致

- [ ] **Step 5: 最终提交（若有验收修复）**

无问题则无需提交。

---

## Self-Review 已完成

- **Spec coverage**：设计 §2 决策表逐条对应（作用范围→Task1 pack_ims `!is_cover` 条件、已是 JPEG 不动→Task1/2 `format != 0` 条件、q90→复用 `CONVERT_JPEG_QUALITY`、WebP/AVIF 子图直接转→Task1/2 解码分支、封面不受影响→Task1 `is_cover` 区分、merge 仅插入候选转→Task4 `is_insert && sub_images_to_jpeg`）。§3.2 MD5 约束→Task4 Step2。§4.3 dedup/tests→Task3。§4.4 前端→Task6。§7 测试→Task5 全部 6 个测试。
- **Placeholder scan**：无 TBD/TODO；所有代码步骤给出完整代码；测试用例给出完整断言与辅助函数。
- **Type consistency**：`pack_ims(image_paths, cover_index, output_path, sub_images_to_jpeg)`（Task1 定义、Task3/4 调用、Task5 测试一致）；`load_image_as_entry(path, is_cover, to_jpeg)`（Task2 定义、Task4 调用、Task5 测试一致）；`BatchConfig.sub_images_to_jpeg`（Task1 定义、Task4 提取、Task6 前端字段一致）；`merge_into_existing_ims(..., sub_images_to_jpeg)`（Task4 定义与调用一致）。
