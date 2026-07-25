# IMS 批量转换工具 — "子图片转为 JPEG" 选项 设计文档

- 日期：2026-07-25
- 状态：已确认决策，待实现
- 作者：TanXianliang
- 关联代码：`src-tauri/src/ims_pack.rs`、`src-tauri/src/lib.rs`、`src-tauri/src/dedup/tests.rs`、`index.html`、`src/app.js`
- 关联设计：`docs/superpowers/specs/2026-07-25-ims-batch-resize-convert-to-jpeg-design.md`（调分辨率工具的转 JPEG，语义一致）

## 1. 目标与范围

在"批量转换"工具（散图按命名规范分组打包成 IMS）中新增"子图片转为 JPEG"开关。勾选后，打包/合并时将所有**非 JPEG 子图**（PNG/WebP/AVIF）重新编码为 JPEG q90，封面保持原格式，已是 JPEG 的子图原样保留，以大幅减小 IMS 体积。

这是对现有批量转换工具的就地扩展，覆盖两条路径：
- **pack 路径**（`pack_ims`）：目标 IMS 不存在时，整组打包
- **merge 路径**（`merge_into_existing_ims`）：目标 IMS 已存在时按 MD5 合并子图

## 2. 决策清单

| 决策点 | 选择 |
|---|---|
| 作用范围 | 仅子图转 JPEG，封面保持原格式（封面不受开关影响） |
| 已是 JPEG 的子图 | 原样保留不重编码（避免二次 JPEG 损失） |
| JPEG 质量 | 固定 q90（复用 `CONVERT_JPEG_QUALITY`） |
| WebP/AVIF 子图 | 直接转 JPEG（解码后编码 JPEG q90，扩展名 `.jpg`） |
| 封面 WebP/AVIF | 仍按现有逻辑转 PNG（不受开关影响） |
| merge 路径 | 支持，但仅 `insert_candidate == true` 的图（真正要插入的子图）转 JPEG；用于 MD5 匹配的封面保持原字节以保证去重正确 |

## 3. 核心逻辑

### 3.1 pack 路径（`pack_ims`）

`pack_ims` 逐文件处理，按 `i == cover_index` 区分封面/子图：

| 文件角色 | 源格式 | 处理 |
|---|---|---|
| 封面 | JPEG/PNG | 原样读字节（现有行为） |
| 封面 | WebP/AVIF | 解码转 PNG（现有行为，不受开关影响） |
| 子图 | JPEG | 原样读字节（不二次编码） |
| 子图 | PNG | 解码 → 转 JPEG q90（alpha-flatten），扩展名 `.jpg` |
| 子图 | WebP | 解码 → 转 JPEG q90，扩展名 `.jpg` |
| 子图 | AVIF | 解码（`decode_avif`）→ 转 JPEG q90，扩展名 `.jpg` |

未勾选时，行为与现有完全一致（JPEG/PNG 原样、WebP/AVIF 转 PNG）。

### 3.2 merge 路径（`merge_into_existing_ims`）

merge 路径里所有 `match_paths`（含封面+子图）都经 `load_image_as_entry` 加载，用于两个目的：MD5 匹配（判断是否已存在）+ 插入候选。

**关键约束**：若把用于 MD5 匹配的封面也转 JPEG，其 MD5 会变，导致与已有 IMS 里同内容封面匹配失败，被当成新图重复插入。

**处理**：在 `merge_into_existing_ims` 的循环里，仅 `insert_keys.contains(source_path_key(path))` 为 true 的图（即 `insert_paths`，真正要插入的子图）调用 `load_image_as_entry(path, false, true)`（`to_jpeg=true`）；其余（仅用于 MD5 匹配的封面）调用 `load_image_as_entry(path, false, false)`（`to_jpeg=false`，保持原字节）。

这样：MD5 匹配用原字节（去重正确），只有真正插入的子图才转 JPEG。

## 4. 组件设计

### 4.1 `ims_pack.rs` 改动

#### 4.1.1 `BatchConfig` 加字段

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

`#[serde(default)]` 保证旧前端反序列化时为 `false`，向后兼容。

#### 4.1.2 `pack_ims` 加参数

```rust
pub fn pack_ims(
    image_paths: &[String],
    cover_index: usize,
    output_path: &str,
    sub_images_to_jpeg: bool,
) -> Result<(), String>
```

内部逐文件处理逻辑调整（在第 273-289 行"WebP/AVIF → 自动转 PNG"块处扩展）：

```rust
let is_cover = i == cover_index;
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
    let (bytes, fmt, name) =
        encode_resized(&decoded, 0, &filename, CONVERT_JPEG_QUALITY)?;
    (bytes, fmt, name)
} else if format == 2 || format == 3 {
    // 现有逻辑：WebP/AVIF → 转 PNG（封面，或未勾选转 JPEG 的子图）
    let img = image::load_from_memory(&raw_bytes).map_err(|e| {
        format!("解码 {} 失败: {} (格式: {})", path, e, if format == 2 { "WebP" } else { "AVIF" })
    })?;
    let mut png_bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败 {}: {}", path, e))?;
    let new_name = format!("{}.png", Path::new(&filename).file_stem().and_then(|s| s.to_str()).unwrap_or("image"));
    (png_bytes, 1u8, new_name)
} else {
    (raw_bytes, format, filename)
};
```

注：AVIF 子图转 JPEG 用 `decode_avif`（`image` crate 无 avif 特性）；WebP 子图转 JPEG 用 `image::load_from_memory`（webp 特性已启用）。

#### 4.1.3 `load_image_as_entry` 加参数

```rust
pub fn load_image_as_entry(path: &Path, is_cover: bool, to_jpeg: bool) -> Result<UpEntry, String>
```

在现有"WebP/AVIF → 转 PNG"逻辑前插入"to_jpeg 且非 JPEG → 转 JPEG"分支：

```rust
let (bytes, final_format, final_filename) = if to_jpeg && format != 0 {
    let decoded = if format == 3 {
        decode_avif(&raw_bytes)
            .map_err(|e| format!("Failed to decode AVIF {}: {}", path.display(), e))?
    } else {
        image::load_from_memory(&raw_bytes).map_err(|e| {
            format!("Failed to decode {}: {}", path.display(), e)
        })?
    };
    let (b, f, n) = encode_resized(&decoded, 0, &filename, CONVERT_JPEG_QUALITY)?;
    (b, f, n)
} else if format == 2 || format == 3 {
    // 现有 WebP/AVIF → PNG 逻辑（不变）
    ...
} else {
    (raw_bytes, format, filename)
};
```

### 4.2 `lib.rs` 改动

#### 4.2.1 `merge_into_existing_ims` 加参数

```rust
fn merge_into_existing_ims(
    output_path: &str,
    match_paths: &[String],
    insert_paths: &[String],
    sub_images_to_jpeg: bool,
) -> Result<MergeExistingResult, String>
```

在加载循环（第 878-890 行）里，按 `insert_keys.contains` 决定 `to_jpeg`：

```rust
for path in match_paths {
    if !seen_sources.insert(source_path_key(path)) {
        continue;
    }
    let is_insert = insert_keys.contains(&source_path_key(path));
    // 仅真正要插入的子图转 JPEG；用于 MD5 匹配的封面保持原字节
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

#### 4.2.2 `start_batch_convert` 传参

- 从 `config.sub_images_to_jpeg` 取值
- 调 `pack_ims(&image_paths, 0, &task.output_path, config.sub_images_to_jpeg)`（第 1613 行）
- 调 `merge_into_existing_ims(&task.output_path, &task.match_paths, &task.sub_image_paths, config.sub_images_to_jpeg)`（第 1585-1588 行）

### 4.3 `dedup/tests.rs` 改动

`pack_ims` 调用（第 53 行）加第四参数 `false`（测试不需要转 JPEG）。

### 4.4 前端改动

- `index.html` 批量转换 modal（`#batch-modal`）的 `.modal-options` 加复选框：
  ```html
  <label><input type="checkbox" id="chk-sub-to-jpeg"> 子图片转为 JPEG (q90，减小体积)</label>
  ```
- `app.js` `collectBatchConfig` 加字段：
  ```js
  sub_images_to_jpeg: $('#chk-sub-to-jpeg').checked,
  ```

## 5. 关键实现细节与约束

- **向后兼容**：`sub_images_to_jpeg` 用 `#[serde(default)]`，旧前端默认为 `false`，行为与现有工具一致
- **alpha 处理**：转 JPEG 时 PNG/WebP/AVIF 的 alpha 通道用 `flatten_alpha_to_rgb` 合成到白底（复用现有函数，经 `encode_resized`）
- **复用现有工具**：`encode_resized`（JPEG 编码 + alpha-flatten + `.jpg` 扩展名）、`decode_avif`、`CONVERT_JPEG_QUALITY=90`、`with_jpg_extension` 均已存在，直接复用
- **MD5 去重正确性**：merge 路径仅插入候选转 JPEG，匹配用图保持原字节（§3.2）
- **不新增依赖**：全部复用现有 `image`/`avif` crate
- **封面不受影响**：封面维持现有"JPEG/PNG 原样、WebP/AVIF 转 PNG"行为

## 6. 错误处理

沿用现有错误处理：任一文件解码/编码失败 → 该组 `Err`，进度消息显示失败原因。merge 路径的锁（`IMS_WRITE_LOCK`）与原子写（`save_ims_entries_atomic`）不变。

## 7. 测试策略

在 `ims_pack.rs` 的 `#[cfg(test)] mod tests` 新增：

- `pack_ims_converts_sub_image_png_to_jpeg`：一组（JPEG 封面 + PNG 子图），`sub_images_to_jpeg=true` → 子图 format=0、扩展名 `.jpg`、封面 format 不变
- `pack_ims_preserves_jpeg_sub_image_when_converting`：一组（JPEG 封面 + JPEG 子图），`sub_images_to_jpeg=true` → 子图字节与原图一致（不二次编码）
- `pack_ims_no_convert_keeps_png_sub_image`：一组（JPEG 封面 + PNG 子图），`sub_images_to_jpeg=false` → 子图 format=1（回归：未勾选行为不变）
- `pack_ims_converts_sub_image_flattens_alpha`：带 alpha 的 PNG 子图，`sub_images_to_jpeg=true` → 转 JPEG 后 `color() != Rgba8`

merge 路径的 `to_jpeg` 逻辑可经 `load_image_as_entry` 直接测：
- `load_image_as_entry_to_jpeg_converts_png`：PNG 文件 `to_jpeg=true` → format=0、扩展名 `.jpg`
- `load_image_as_entry_to_jpeg_preserves_jpeg`：JPEG 文件 `to_jpeg=true` → 字节不变

## 8. 不在范围内（YAGNI）

- 不加 JPEG 质量控件（固定 q90）
- 封面不转 JPEG（保持原格式）
- 不改变未勾选时的现有行为
- 不支持转 WebP/AVIF（仅转 JPEG）
- 不动调分辨率工具（已实现独立的转 JPEG）

## 9. 改动文件清单

| 文件 | 改动 |
|---|---|
| `src-tauri/src/ims_pack.rs` | `BatchConfig` 加字段、`pack_ims` 加参数与子图转 JPEG 逻辑、`load_image_as_entry` 加 `to_jpeg` 参数、新增测试 |
| `src-tauri/src/lib.rs` | `merge_into_existing_ims` 加参数与 `to_jpeg` 区分逻辑、`start_batch_convert` 传参 |
| `src-tauri/src/dedup/tests.rs` | `pack_ims` 调用加 `false` 参数 |
| `index.html` | 批量转换 modal 加 `chk-sub-to-jpeg` 复选框 |
| `src/app.js` | `collectBatchConfig` 加 `sub_images_to_jpeg` 字段 |

## 10. 开放问题

无。所有决策已在第 2 节确认。
