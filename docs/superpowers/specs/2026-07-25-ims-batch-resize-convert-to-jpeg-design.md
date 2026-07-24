# IMS 批量调分辨率工具 — "同时转为 JPEG" 选项 设计文档

- 日期：2026-07-25
- 状态：已确认决策，待编写实现计划
- 作者：TanXianliang
- 关联代码：`src-tauri/src/ims_pack.rs`、`src-tauri/src/lib.rs`、`index.html`、`src/app.js`
- 前置设计：`docs/superpowers/specs/2026-07-24-ims-batch-resize-design.md`

## 1. 目标与范围

在已有的 IMS 批量调分辨率工具（`resize_ims_file` + `start_batch_resize` + `#resize-modal`）中新增一个"同时转为 JPEG"开关。勾选后，除按最长边缩小超标子图外，还将 IMS 文件中的**所有非 JPEG 子图**（PNG/WebP/AVIF）重新编码为 JPEG，以大幅减小文件体积（实测 PNG→JPEG q90 可省约 85%）。

这是对现有工具的就地扩展，不改变 IMS 容器结构，不新增独立工具。

## 2. 决策清单

| 决策点 | 选择 |
|---|---|
| 转 JPEG 作用范围 | 所有非 JPEG 子图（PNG/WebP/AVIF）勾选后统一转 JPEG |
| 与最长边关系 | 转 JPEG 独立于缩放：勾选后所有非 JPEG 子图都转（无论是否超标） |
| JPEG 质量 | 固定 q90，不加质量控件 |
| WebP/AVIF 处理 | 都转 JPEG，扩展名改 `.jpg` |
| 未超标 JPEG 子图 | 保留原字节不动（已是目标格式，避免二次 JPEG 损失） |
| 编码函数复用 | 方案 X：复用 `encode_resized`（其职责是"编码一张图到目标格式"，与是否 resize 无关），不新增函数 |

## 3. 核心逻辑

勾选"转 JPEG"（`convert_to_jpeg = true`）后，每个子图的处理路径：

| 子图格式 | 是否超标 | 处理 |
|---|---|---|
| JPEG(0) | 超标 | 缩小 + 重编码 JPEG q88（保持格式缩小，用 `RESIZE_JPEG_QUALITY`） |
| JPEG(0) | 未超标 | **保留原字节**（不二次编码） |
| PNG(1) | 超标 | 缩小 + 转 JPEG q90（alpha-flatten 到白底，用 `CONVERT_JPEG_QUALITY`） |
| PNG(1) | 未超标 | 转 JPEG q90（不缩小，alpha-flatten 到白底，用 `CONVERT_JPEG_QUALITY`） |
| WebP(2) | 超标 | 缩小 + 转 JPEG q90 |
| WebP(2) | 未超标 | 转 JPEG q90 |
| AVIF(3) | 超标 | 缩小 + 转 JPEG q90（`decode_avif` 解码） |
| AVIF(3) | 未超标 | 转 JPEG q90（`decode_avif` 解码） |

质量规则：**已是 JPEG 的子图**缩小重编码用 `RESIZE_JPEG_QUALITY`（88，保持现有行为）；**非 JPEG 子图转 JPEG**用 `CONVERT_JPEG_QUALITY`（90）。即 `jpeg_quality` 参数按"该子图是否原为 JPEG"选择：原 JPEG → 88，非 JPEG 转 JPEG → 90。

未勾选（`convert_to_jpeg = false`，默认）时，行为与现有工具完全一致：保持原格式，AVIF→PNG，未超标保留原字节。

### 3.1 关键行为变化

勾选转 JPEG 后，**未超标的非 JPEG 子图也要重编码**（转 JPEG）——这打破了原"未超标保留原字节"原则，但这是用户明确选择的（转格式本身就是目的）。`Skipped` 判定相应调整：

- 未勾选：`Skipped` 当且仅当所有子图最长边 ≤ max_edge（现有行为）
- 勾选：`Skipped` 当且仅当（所有子图最长边 ≤ max_edge）且（所有子图都已是 JPEG）。只要文件里有非 JPEG 子图，即使全部未超标，也需重写

### 3.2 文件名扩展名

转 JPEG 后子图扩展名改为 `.jpg`（新增 `with_jpg_extension` 辅助，与现有 `with_png_extension` 平行）。AVIF 子图在未勾选时仍转 PNG（`.png`），勾选时转 JPEG（`.jpg`）。

## 4. 组件设计

### 4.1 `ims_pack.rs` 改动

#### 4.1.1 `BatchResizeConfig` 加字段

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResizeConfig {
    pub folders: Vec<FolderConfig>,
    pub max_edge: u32,
    pub multi_threaded: bool,
    /// 是否将所有非 JPEG 子图转为 JPEG（q90）
    #[serde(default)]
    pub convert_to_jpeg: bool,
}
```

`#[serde(default)]` 保证旧前端（无此字段）反序列化时为 `false`，向后兼容。

#### 4.1.2 新增常量

```rust
const CONVERT_JPEG_QUALITY: u8 = 90;
```

（现有 `RESIZE_JPEG_QUALITY = 88` 用于"保持 JPEG 格式"的缩小重编码；`CONVERT_JPEG_QUALITY = 90` 用于"转 JPEG"。两者语义不同，分开定义。）

#### 4.1.3 `resize_ims_file` 签名与逻辑

签名加参数：
```rust
pub fn resize_ims_file(
    filepath: &str,
    max_edge: u32,
    convert_to_jpeg: bool,
) -> Result<ResizeOutcome, String>
```

`ResizeOutcome` 扩展以区分"缩小"与"转码"计数（用于进度消息）：
```rust
pub enum ResizeOutcome {
    Resized { sub_images_resized: usize, sub_images_converted: usize },
    Skipped,
}
```
（`sub_images_resized` = 被缩小的子图数；`sub_images_converted` = 被转为 JPEG 的非 JPEG 子图数。未勾选时 `sub_images_converted` 恒为 0。）

核心循环逻辑（伪代码）：
```
for img in &ims_info.images:
    bytes = read_sub_image(...)
    decoded = decode(bytes, img.format)   // AVIF 走 decode_avif，其余 image crate
    longest = max(w, h)
    is_jpeg = (img.format == 0)
    oversized = longest > max_edge

    if !oversized && (is_jpeg || !convert_to_jpeg):
        // 未超标且（已是 JPEG 或未勾选转码）→ 保留原字节
        entries.push(原 bytes, 原 format, 原 filename)
        continue

    // 需要重编码：确定目标格式
    target_format = if convert_to_jpeg { 0 } else if img.format == 3 { 1 } else { img.format }
    // 注：未勾选时 AVIF→PNG(1)，其余保持原格式；勾选时统一 JPEG(0)

    // 确定待编码的图（超标则先缩小）
    image_to_encode = if oversized {
        resize_exact(decoded, new_w, new_h, Lanczos3)
    } else {
        decoded   // 未超标转码：图不变尺寸
    }

    // JPEG 质量：原 JPEG 子图用 88，非 JPEG 转 JPEG 用 90
    jpeg_quality = if is_jpeg { RESIZE_JPEG_QUALITY } else { CONVERT_JPEG_QUALITY }

    (final_bytes, final_format, final_filename) =
        encode_resized(&image_to_encode, target_format, &img.filename, jpeg_quality)?
    entries.push(final_bytes, final_format, final_filename)
    if oversized { resized_count += 1 }
    if convert_to_jpeg && !is_jpeg { converted_count += 1 }

if resized_count == 0 && converted_count == 0:
    return Ok(Skipped)
// 否则原子重写（同现有逻辑）
```

#### 4.1.4 `encode_resized` 调整

现有 `encode_resized(image, out_format, out_filename)` 的 JPEG 分支固定用 `RESIZE_JPEG_QUALITY`（88）。为支持"原 JPEG 用 88、转 JPEG 用 90"，给 `encode_resized` 加一个 `jpeg_quality: u8` 参数，由调用方按"该子图是否原为 JPEG"传入（原 JPEG → 88，非 JPEG 转 JPEG → 90）。

签名变为：
```rust
fn encode_resized(
    image: &image::DynamicImage,
    out_format: u8,
    out_filename: &str,
    jpeg_quality: u8,
) -> Result<(Vec<u8>, u8, String), String>
```

JPEG 分支的扩展名处理：当 `out_format == 0` 且原文件名不是 `.jpg/.jpeg` 时，扩展名改为 `.jpg`（新增 `with_jpg_extension`）。PNG 分支逻辑不变。

#### 4.1.5 新增 `with_jpg_extension`

```rust
fn with_jpg_extension(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    format!("{}.jpg", stem)
}
```

### 4.2 `lib.rs` 改动

`start_batch_resize` 把 `config.convert_to_jpeg` 传给 `resize_ims_file`：
```rust
let convert_to_jpeg = config.convert_to_jpeg;
// ...
let outcome = resize_ims_file(fp, max_edge, convert_to_jpeg);
```

进度消息适配新的 `ResizeOutcome::Resized { sub_images_resized, sub_images_converted }`：
- 仅缩小：`已调整 {fname} (缩小 N 张子图)`
- 仅转码：`已调整 {fname} (转 JPEG M 张子图)`
- 两者：`已调整 {fname} (缩小 N 张, 转 JPEG M 张)`

### 4.3 `index.html` 改动

在 `#resize-modal` 的 `.modal-options` 中（`chk-resize-multi` 之后）加复选框：
```html
<label><input type="checkbox" id="chk-resize-to-jpeg"> 同时转为 JPEG (q90，显著减小体积)</label>
```

### 4.4 `app.js` 改动

`collectResizeConfig` 加字段：
```js
return {
  folders,
  max_edge: maxEdge,
  multi_threaded: $('#chk-resize-multi').checked,
  convert_to_jpeg: $('#chk-resize-to-jpeg').checked,
};
```

## 5. 关键实现细节与约束

- **向后兼容**：`convert_to_jpeg` 用 `#[serde(default)]`，旧前端调用时默认为 `false`，行为与现有工具一致
- **alpha 处理**：转 JPEG 时 PNG/WebP/AVIF 的 alpha 通道用 `flatten_alpha_to_rgb` 合成到白底（复用现有函数）
- **不放大**：转 JPEG 不改变"不放大"原则——未超标子图转 JPEG 时图尺寸不变
- **原子性**：沿用现有 `.tmp_resize` + rename 原子替换；任一子图失败整文件跳过
- **质量分离**：保持 JPEG 缩小用 q88（`RESIZE_JPEG_QUALITY`），转 JPEG 用 q90（`CONVERT_JPEG_QUALITY`），语义清晰
- **不新增依赖**：全部复用现有 `image`/`avif` crate

## 6. 错误处理

沿用现有错误处理：任一子图解码/编码失败 → 整文件 `Err`，原文件保持完好，进度消息显示失败原因。`Skipped` 判定见 §3.1。

## 7. 测试策略

在 `ims_pack.rs` 的 `#[cfg(test)] mod tests` 新增：

- `resize_convert_to_jpeg_converts_undersized_png`：单张未超标 PNG 子图，`convert_to_jpeg=true` → `Resized{resized:0, converted:1}`，重新解析后子图 format=0、扩展名 `.jpg`、可解码为 JPEG
- `resize_convert_to_jpeg_preserves_undersized_jpeg`：单张未超标 JPEG 子图，`convert_to_jpeg=true` → `Skipped`，文件字节不变（5a：已是 JPEG 不二次编码）
- `resize_convert_to_jpeg_oversized_png_resizes_and_converts`：超标 PNG，`convert_to_jpeg=true` → `Resized{resized:1, converted:1}`，尺寸缩小且 format=0
- `resize_no_convert_keeps_png_when_undersized`：未超标 PNG，`convert_to_jpeg=false` → `Skipped`，文件字节不变（回归：未勾选行为不变）
- `resize_convert_to_jpeg_flattens_alpha`：带 alpha 的 PNG，`convert_to_jpeg=true` → 转 JPEG 后 `color() != Rgba8`（alpha 已合成）

复用现有 `make_ims_with_sub_images` 辅助（需扩展支持 RGBA，或参考 `resize_preserves_alpha_for_png_sub_image` 的内联构造方式）。

## 8. 不在范围内（YAGNI）

- 不加 JPEG 质量控件（固定 q90）
- 不改变"保持原格式"的默认行为（未勾选时与现有工具一致）
- 不支持转 WebP/AVIF（仅转 JPEG）
- 不改变 AVIF 未勾选时转 PNG 的现有行为
- 不新增独立"批量转 JPEG"工具（合并在调分辨率工具中）

## 9. 改动文件清单

| 文件 | 改动 |
|---|---|
| `src-tauri/src/ims_pack.rs` | `BatchResizeConfig` 加字段、`ResizeOutcome` 加 `sub_images_converted`、`resize_ims_file` 加参数与转码逻辑、`encode_resized` 加 `jpeg_quality` 参数、新增 `CONVERT_JPEG_QUALITY`/`with_jpg_extension`、新增测试 |
| `src-tauri/src/lib.rs` | `start_batch_resize` 传 `convert_to_jpeg`、进度消息适配 |
| `index.html` | `#resize-modal` 加 `chk-resize-to-jpeg` 复选框 |
| `src/app.js` | `collectResizeConfig` 加 `convert_to_jpeg` 字段 |

## 10. 开放问题

无。所有决策已在第 2 节确认。
