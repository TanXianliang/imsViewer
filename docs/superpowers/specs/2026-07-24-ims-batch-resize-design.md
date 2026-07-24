# IMS 子图批量分辨率调整工具 — 设计文档

- 日期：2026-07-24
- 状态：已确认决策，待编写实现计划
- 作者：TanXianliang
- 关联代码：`src-tauri/src/ims_pack.rs`、`src-tauri/src/lib.rs`、`src-tauri/src/ims.rs`、`src-tauri/src/dedup/source.rs`、`index.html`、`src/app.js`

## 1. 目标与范围

在 imsViewer 桌面应用（Tauri v2 + Rust 后端 + 原生 JS 前端）中新增一个批量工具，用于**就地**调整 IMS 容器文件（`.ims.{ext}`）中各子图片的分辨率：

- 用户设定一个"最长边分辨率"阈值 `max_edge`（px）
- 扫描所选目录下的所有 IMS 文件
- 对每个文件解析其子图，逐子图判断：若该子图原始最长边 **大于** `max_edge`，则按保持长宽比的方式缩小至最长边等于 `max_edge`，并重新编码写回；若最长边 **小于或等于** `max_edge`，保留原始字节不做任何改动（**不放大**）
- 只要一个 IMS 文件中有任意一张子图被缩小，该文件就会被原子重写；若所有子图均未超标，该文件完全跳过不写

工具是**降采样专用**，不做放大，不导出独立图片，不改变 IMS 容器结构。

## 2. 决策清单

| 决策点 | 选择 |
|---|---|
| 工作范围 | 就地修改现有 IMS 文件（无输出目录、无备份） |
| 输出格式 | 保持子图原格式；AVIF 子图用 `avif-rs` 解码后转 PNG |
| 缩放滤波器 | `Lanczos3` |
| 未超标子图 | 保留原始字节不重编码 |
| 判定/处理粒度 | 子图级（任一子图超标则重写该文件，未超标子图原样放回） |
| 输入形态 | 多文件夹（动态行，每行路径 + 深度，参照 `batch-convert`） |
| 最长边范围 | 下限 64、无硬上限（超 32768 前端警告）、不放大、默认 2560 |
| 失败处理 | 单子图失败 → 整文件跳过，原文件保持完好；不备份 |
| UI 形态 | 新增 `#resize-modal`，复用 `.modal-*` 共享样式，不写新 CSS |

## 3. 架构与实现路径

采用**方案一**：在 `ims_pack.rs` 新增 `resize_ims_file` 函数 + 在 `lib.rs` 新增 `start_batch_resize` Tauri 命令，完全镜像现有 `upgrade_ims_file` + `start_batch_upgrade` 的实现范式。不重构现有代码、不新建模块、不引入新依赖。

理由：
- `ims_pack.rs` 已具备全部写回工具：`write_ims_entries`、`UpEntry`、`create_embedded_thumbnail`（自动重生成内嵌缩略图和 IMSTHMB trailer）
- `lib.rs` 已具备全部批处理基础设施：`scan_ims_files`、`batch-progress` 事件、rayon/串行双模式、`IMS_WRITE_LOCK`、`ImsCache` 失效
- AVIF 解码可复用 `dedup/source.rs` 已有的 `avif-rs` 调用模式（`avif` crate 已在 `Cargo.toml` 依赖中）
- 与现有三个批量工具（convert/extract/upgrade）的扩展方式一一对应，改动集中、风险最低

### 3.1 数据流

```
用户在 #resize-modal 配置：
  folders[] (path + depth)  +  max_edge (int, 默认2560)  +  multi_threaded (bool)
        │
        ▼
invoke('start_batch_resize', { config: BatchResizeConfig })
        │  (lib.rs::start_batch_resize)
        ▼
对每个 folder 调用 scan_ims_files(path, depth) → 合并去重得到 Vec<String> files
        │
        ▼
emit batch-progress { current:0, total, status:"running" }  →  立即 return Ok(total_msg)
        │
        ▼
std::thread::spawn:
  ├─ multi_threaded=true  → rayon par_iter，AtomicU32 计数 + AtomicBool 错误标志
  └─ multi_threaded=false → 串行 for 循环，u32 计数 + bool 错误标志
  每个文件:
    resize_ims_file(filepath, max_edge)  →  lock IMS_WRITE_LOCK
        │  (ims_pack.rs::resize_ims_file)
        ▼
    parse_ims(filepath)  →  ImsFileInfo { images: Vec<ImsImage> }
        │
        ▼
    对每个子图:
      read_sub_image(filepath, data_offset, data_length)  →  bytes
      decode_image(bytes, format)  →  (image::DynamicImage, 新format, 新ext)
        │   ├─ JPEG/PNG/WebP → image::load_from_memory(bytes)
        │   └─ AVIF          → avif-rs 解码 → DynamicImage（新format=PNG, 新ext=.png）
        ▼
      longest_edge = max(width, height)
      if longest_edge > max_edge:
        计算新尺寸保持比例: longest_edge → max_edge
        resized = img.resize_exact(new_w, new_h, FilterType::Lanczos3)
        若有 alpha → alpha-flatten 到白底（复用 create_embedded_thumbnail 的合成逻辑）
        re-encode 到目标格式（JPEG q? / PNG）→ new_bytes
        entry = UpEntry{ filename, format=新format, bytes=new_bytes, is_cover }
      else:
        entry = UpEntry{ filename, format=原format, bytes=原bytes, is_cover }  // 原样放回
        │
        ▼
    若没有任何子图被缩小 → 整文件跳过，返回 Skipped
    若有子图被缩小 → cover 排前 → write_ims_entries(tmp) → remove+rename 原子替换
        │
        ▼
    失效 ImsCache 该路径
        │
        ▼
  发每文件 batch-progress（成功/跳过/失败）
        │
        ▼
  结束：发 batch-progress { status:"completed" | "error" }
```

## 4. 组件设计

### 4.1 `ims_pack.rs` 新增内容

#### 4.1.1 `BatchResizeConfig` 结构体

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResizeConfig {
    pub folders: Vec<FolderConfig>,   // 复用现有 FolderConfig { path, depth }
    pub max_edge: u32,                // 最长边像素，下限 64
    pub multi_threaded: bool,
}
```

#### 4.1.2 `ResizeOutcome` 枚举（用于驱动进度消息）

```rust
enum ResizeOutcome {
    Resized { sub_images_resized: usize },   // 有子图被缩小，文件已重写
    Skipped,                                  // 全部子图未超标，文件未动
}
```

#### 4.1.3 `resize_ims_file` 函数

```rust
pub fn resize_ims_file(filepath: &str, max_edge: u32) -> Result<ResizeOutcome, String>
```

行为：
1. 调用 `crate::ims::parse_ims(filepath)` 获取 `ImsFileInfo`
2. 对每个 `ImsImage`：
   - `read_sub_image(filepath, img.data_offset, img.data_length)` 取原始字节
   - `decode_sub_image(&bytes, img.format)` → `(DynamicImage, new_format: u8, new_ext: &str)`
     - format 0(JPEG)/1(PNG)/2(WebP)：`image::load_from_memory(&bytes)`
     - format 3(AVIF)：`avif-rs` 解码（复用 `dedup/source.rs` 的调用模式）→ 返回 `new_format=1`(PNG), `new_ext="png"`
   - 计算 `longest = max(img.width(), img.height())`
   - 若 `longest > max_edge`：
     - 按比例算 `new_w`/`new_h`（`longest` 对应边 = `max_edge`，另一边 = round(原边 * max_edge / longest)）
     - `resized = img.resize_exact(new_w, new_h, FilterType::Lanczos3)`
     - 若 `resized.color().has_alpha()` → alpha-flatten 到白底（提取 `create_embedded_thumbnail` 里的合成循环为辅助函数 `flatten_alpha_to_rgb`，复用）
     - 重新编码：JPEG→`JpegEncoder::new_with_quality`（质量沿用现有约定，见 4.1.5）、PNG→`PngEncoder`；AVIF 解出的图按 PNG 编码
     - 构建 `UpEntry { filename, format: new_format, bytes: new_bytes, is_cover }`（AVIF 时把 filename 扩展名改为 `.png`）
     - 标记 `resized_count += 1`
   - 否则：`UpEntry { filename, format: img.format, bytes: 原bytes, is_cover }`（原样放回）
   - 任一子图解码/编码出错 → 立即 `Err`，不写文件（整文件跳过，原文件完好）
3. 若 `resized_count == 0` → 返回 `Ok(ResizeOutcome::Skipped)`
4. `entries.sort_by_key(|e| !e.is_cover)`（封面排前）
5. 临时文件路径 `format!("{}.tmp_resize", filepath)`
6. `write_ims_entries(&tmp_path, &entries)`（自动重生成内嵌缩略图 + IMSTHMB trailer）
7. `std::fs::remove_file(filepath)` 然后 `std::fs::rename(&tmp_path, filepath)`
8. 返回 `Ok(ResizeOutcome::Resized { sub_images_resized: resized_count })`

#### 4.1.4 AVIF 解码辅助

```rust
fn decode_avif(bytes: &[u8]) -> Result<image::DynamicImage, String>
```

复用 `dedup/source.rs` 中 `avif-rs` 的调用模式（`avif::Decoder::new()` → `decode()` → 转 `DynamicImage`）。不新增依赖（`avif` 已在 `Cargo.toml`）。

#### 4.1.5 JPEG 重编码质量

沿用项目现有 JPEG 编码惯例：
- `create_embedded_thumbnail` 用 q60（缩略图）
- `dedup/thumbnail.rs` 用 q72（结果缩略图）

主图缩放是**有损降采样**，应在"体积"与"可见质量"间取平衡。建议 **JPEG q88**（接近无损观感、体积明显小于原始高分辨率）。新增常量：
```rust
const RESIZE_JPEG_QUALITY: u8 = 88;
```
PNG 为无损格式，不涉及质量参数。

### 4.2 `lib.rs` 新增内容

#### 4.2.1 `start_batch_resize` Tauri 命令

```rust
#[tauri::command]
async fn start_batch_resize(
    app: tauri::AppHandle,
    config: BatchResizeConfig,
) -> Result<String, String>
```

完全镜像 `start_batch_upgrade` 的结构：
1. 校验 `config.max_edge >= 64`，否则 `Err("最长边分辨率不能小于 64")`
2. 对每个 folder 调 `scan_ims_files(path, depth)`，合并结果并按路径去重（保留顺序），得 `Vec<String> files`
3. `total = files.len()`；若 0 → `Err("未发现 IMS 文件")`
4. `emit batch-progress { current:0, total, message:"共发现 N 个 IMS 文件", status:"running" }`
5. `std::thread::spawn`（返回 `Ok(total_msg)`）：
   - `multi_threaded=true`：`files.par_iter().for_each`，`AtomicU32` 计数 + `AtomicBool` 错误标志
   - `multi_threaded=false`：串行 `for` + `u32` + `bool`
   - 每个文件：`let _lock = IMS_WRITE_LOCK.lock()`（避免与 `save_ims`/`merge_into_existing_ims` 竞争）→ 调 `resize_ims_file(fp, max_edge)`
     - `Ok(Resized{..})` → 进度消息 `"已调整 {filename} (缩小 N 张子图)"`
     - `Ok(Skipped)` → 进度消息 `"跳过 {filename} (无子图超标)"`，计数 +1
     - `Err(e)` → 进度消息 `"失败 {filename}: {e}"`，置错误标志（不中断）
   - 成功重写后失效 `ImsCache`：通过 `app.try_state::<ImsCache>()` 取出并 `cache.0.lock().remove(fp)`（与 `save_ims` 一致；注意 rayon 线程内需用 `try_state` 而非 `State<'_>`）
   - 结束发 `completed` 或 `error` 终态事件

#### 4.2.2 注册命令

在 `tauri::generate_handler![...]` 列表中，于 `start_batch_upgrade` 之后追加 `start_batch_resize`。

### 4.3 `index.html` 新增内容

#### 4.3.1 工具菜单项

在工具菜单 `data-action="batch-upgrade"` 之后、`menu-separator` 之前追加：
```html
<div class="menu-action" data-action="batch-resize">批量调整分辨率...</div>
```

#### 4.3.2 `#resize-modal` 对话框

在 `#upgrade-modal` 之后追加新 modal，复用 `.modal-*` 共享样式，结构：
```
#resize-modal (modal-overlay, display:none)
 └─ modal-container
    ├─ modal-header: "批量调整 IMS 子图分辨率" + btn-resize-close (×)
    └─ modal-body
       ├─ #resize-folder-list             (动态多文件夹行，复用 addResizeFolderRow)
       ├─ button #btn-resize-add-folder    "+ 添加文件夹"
       ├─ output-dir-row: label "最长边分辨率" + input #resize-max-edge (number, min=64, value=2560)
       ├─ modal-options: <label><input type="checkbox" id="chk-resize-multi"> 多线程</label>
       └─ progress-section: #resize-progress-fill + #resize-progress-text
    └─ modal-footer: #btn-start-resize (btn-primary, "开始调整") + #btn-cancel-resize (btn-secondary, "关闭")
```

注：复用 `#folder-list` 的 `.folder-row` 样式（`.path-input` + `.btn-browse` + `.depth-input` + `.btn-remove`），但用独立 JS 函数 `addResizeFolderRow` 绑定独立的浏览/移除逻辑，避免与 convert 的 `#folder-list` ID 冲突。

### 4.4 `src/app.js` 新增内容

#### 4.4.1 菜单分发

在 `.menu-action` click handler 的 switch 中追加：
```js
case 'batch-resize':
  openResizeModal();
  break;
```

#### 4.4.2 Modal 控制函数

镜像 `openBatchModal/closeBatchModal/collectBatchConfig/startBatchConvert/setupBatchEvents`：
- `let resizeUnlisten = null;`
- `openResizeModal()`：显示 modal，重置 `#resize-folder-list` 为空并 `addResizeFolderRow()`，重置进度条/文本，禁用按钮恢复，调用 `setupResizeEvents()`
- `closeResizeModal()`：隐藏，detach `resizeUnlisten`
- `addResizeFolderRow(path='', depth=0)`：构造 `.folder-row`，浏览按钮调 `invoke('pick_folder')`，移除按钮删行
- `collectResizeConfig()`：
  ```js
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
  ```
- `startBatchResize()`：
  - 校验 `folders.length > 0`，否则 alert
  - 校验 `max_edge >= 64`，否则 alert
  - 若 `max_edge > 32768` → `confirm("最长边超过 32768，确定继续吗？")`，取消则 return
  - 禁用按钮 + 设文本"调整中..."
  - `resizeUnlisten = await listen('batch-progress', ...)`，更新 `#resize-progress-fill`/`#resize-progress-text`，`completed`/`error` 时恢复按钮并加 ✅/⚠️ 前缀
  - `await invoke('start_batch_resize', { config })`
- `setupResizeEvents()`：绑定添加文件夹、关闭、开始、浏览按钮

### 4.5 不改动的部分

- `src/styles.css`：全部复用 `.modal-*` / `.output-dir-row` / `.depth-input` / `.modal-options` / `.progress-section` / `.folder-row`，**不写新 CSS**
- `Cargo.toml`：不新增依赖（`image`、`avif`、`rayon`、`walkdir` 均已存在）
- `ims.rs`：不改

## 5. 关键实现细节与约束

### 5.1 IMS_WRITE_LOCK 与 ImsCache 失效

- `resize_ims_file` 内部**不**加锁（保持与 `upgrade_ims_file` 一致——它也不加锁）；锁由 `start_batch_resize` 的调用方在 `lib.rs` 中获取：每个文件处理前 `let _guard = IMS_WRITE_LOCK.lock()`，处理完释放。这与 `merge_into_existing_ims` 的加锁位置一致。
- 成功重写后失效 `ImsCache`：rayon 工作线程内用 `app.try_state::<ImsCache>()`（不能用 `State<'_>`），取到后 `cache.0.lock().remove(fp)`。与 `save_ims` 的失效逻辑一致，避免下次 `open_file` 读到旧缓存。

### 5.2 AVIF 子图处理

- `image` crate **未启用 avif 特性**，`image::load_from_memory` 无法解码 AVIF 字节。
- format==3 的子图走 `decode_avif`（`avif-rs`），解码后统一转 PNG 重编码，`UpEntry.format=1`，并把 `filename` 的扩展名改为 `.png`（与 `pack_ims`/`save_ims` 遇 WebP/AVIF 源时转 PNG 的惯例一致）。
- 解码失败 → 整文件 `Err` 跳过（符合 6a=i）。

### 5.3 Alpha 通道处理

- JPEG 不支持 alpha；若待缩小的子图解码后带 alpha 通道（PNG/AVIF 来源），先 alpha-flatten 到白底再编码 JPEG，复用 `create_embedded_thumbnail` 中的 `src*alpha + 255*(255-alpha) + 127) / 255` 合成公式。
- PNG 支持 alpha，保留 alpha 直接编码。
- 提取一个辅助函数 `flatten_alpha_to_rgb(rgba: RgbaImage) -> RgbImage` 供 `create_embedded_thumbnail` 和 `resize_ims_file` 共用（小幅重构，不改变现有缩略图行为）。

### 5.4 尺寸计算

- `longest = max(w, h)`；若 `longest > max_edge`：
  - `scale = max_edge as f64 / longest as f64`
  - `new_w = max(1, round(w * scale))`、`new_h = max(1, round(h * scale))`
  - 用 `image::image_dimensions` 或 `DynamicImage::width()/height()` 取原始尺寸
  - `resize_exact(new_w, new_h, Lanczos3)` 精确缩放到目标尺寸（`thumbnail` 是"不超过"语义，不适合这里因为我们要精确等于 max_edge）

### 5.5 临时文件与原子替换

- 临时后缀 `.tmp_resize`（与 `upgrade` 的 `.tmp_upgrade` 区分，避免并发冲突）
- `write_ims_entries(tmp)` 成功后才 `remove_file(原)` + `rename(tmp, 原)`；`write_ims_entries` 失败则清理 tmp 并返回 Err，原文件不动
- 单子图 IMS（仅封面）也走 `write_ims_entries`（不调用 `save_ims_entries_atomic`，因为 resize 场景下即使只剩一张子图也仍是 IMS 容器，应保留 IMS 格式而非降级为裸图）

### 5.6 范围校验

- 前端 `#resize-max-edge`：`min="64"`，JS 校验 `< 64` alert、`> 32768` confirm
- 后端 `start_batch_resize`：`max_edge < 64` → `Err`，双重校验

## 6. 错误处理

| 情况 | 行为 |
|---|---|
| 目录无 IMS 文件 | `start_batch_resize` 同步返回 `Err("未发现 IMS 文件")`，不启动线程 |
| `max_edge < 64` | 前端 alert + 后端 `Err`，双重校验 |
| 单子图解码失败（含 AVIF） | `resize_ims_file` 返回 `Err`，该文件整文件跳过，原文件完好，进度消息 `"失败 {filename}: {e}"`，错误标志置位 |
| `write_ims_entries` 失败 | 清理 tmp，返回 `Err`，原文件完好 |
| `remove`/`rename` 失败 | 返回 `Err`，进度消息带原因；若 rename 失败 tmp 可能残留，下次同路径处理会覆盖 |
| 线程结束时有失败 | 发 `error` 终态事件，消息 "有文件处理失败" |
| 线程结束全部成功/跳过 | 发 `completed` 终态事件 |

## 7. 测试策略

### 7.1 单元测试（`ims_pack.rs` 内 `#[cfg(test)]`）

- `resize_ims_file_skips_when_all_sub_images_within_max_edge`：构造一张 100×100 子图的 IMS，`max_edge=200` → `Ok(Skipped)`，文件字节不变
- `resize_ims_file_resizes_oversized_sub_image`：构造一张 800×600 JPEG 子图的 IMS，`max_edge=400` → `Ok(Resized{1})`，重新解析后子图尺寸为 400×300
- `resize_ims_file_preserves_undersized_sub_image_bytes`：IMS 含两张子图（800×600 超标 + 100×100 未超标），`max_edge=400` → 仅第一张重编码，第二张字节与原始 `read_sub_image` 结果逐字节相等
- `resize_ims_file_maintains_aspect_ratio`：4000×3000 → `max_edge=2000` → 新尺寸 2000×1500
- `resize_ims_file_handles_alpha_png`：带 alpha 的 PNG 子图超标缩小后重新解码无 alpha 通道（JPEG）或保留 alpha（PNG）
- `resize_ims_file_returns_err_on_corrupt_sub_image`：损坏的子图字节 → `Err`，原文件不动
- 复用 `tempfile` crate（已在 dev-dependencies）

### 7.2 手工验收

- 准备含混合尺寸子图（含超标/未超标、JPEG/PNG、可选 AVIF）的 IMS 文件若干
- 用 UI 对单文件夹运行，验证进度消息、跳过/成功/失败计数、终态
- 验证处理后 IMS 仍可被 `open_file` 正常解析、子图可显示、缩略图刷新
- 验证多线程与单线程结果一致
- 验证 `max_edge=64` 边界与 `max_edge>32768` 警告

## 8. 不在范围内（YAGNI）

- 不做放大（仅降采样）
- 不导出独立图片文件
- 不支持输出目录（就地修改）
- 不做原文件备份
- 不改 IMS 容器版本/格式
- 不对封面单独特殊处理（封面也按子图级判断和缩放）
- 不新增独立 Rust 模块（集中在 `ims_pack.rs`/`lib.rs`）
- 不新增 CSS
- 不新增 Cargo 依赖

## 9. 改动文件清单

| 文件 | 改动 |
|---|---|
| `src-tauri/src/ims_pack.rs` | 新增 `BatchResizeConfig`、`ResizeOutcome`、`resize_ims_file`、`decode_avif`、`flatten_alpha_to_rgb`（提取自 `create_embedded_thumbnail`，后者改为调用它）；新增常量 `RESIZE_JPEG_QUALITY` |
| `src-tauri/src/lib.rs` | 新增 `start_batch_resize` 命令；`generate_handler!` 列表追加 `start_batch_resize` |
| `index.html` | 工具菜单新增 `batch-resize` 项；新增 `#resize-modal` 对话框 |
| `src/app.js` | 新增 `resizeUnlisten`、`openResizeModal`、`closeResizeModal`、`addResizeFolderRow`、`collectResizeConfig`、`startBatchResize`、`setupResizeEvents`；switch 新增 `batch-resize` 分支 |
| `src-tauri/src/ims.rs` | 不改 |
| `src-tauri/Cargo.toml` | 不改 |
| `src/styles.css` | 不改 |

## 10. 开放问题

无。所有决策已在第 2 节确认。
