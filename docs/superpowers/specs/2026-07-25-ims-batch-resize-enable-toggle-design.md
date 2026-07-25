# IMS 批量调分辨率工具 — "调分辨率" 可开关 设计文档

- 日期：2026-07-25
- 状态：已确认决策，待实现
- 作者：TanXianliang
- 关联代码：`src-tauri/src/ims_pack.rs`、`src-tauri/src/lib.rs`、`index.html`、`src/app.js`
- 前置设计：`docs/superpowers/specs/2026-07-24-ims-batch-resize-design.md`、`docs/superpowers/specs/2026-07-25-ims-batch-resize-convert-to-jpeg-design.md`

## 1. 目标与范围

把批量调分辨率工具中的"调分辨率"功能从"必做"改为可开关。新增"调整分辨率"复选框（默认勾选），与已有的"同时转为 JPEG"复选框配合，支持三种模式：

- **只调分辨率**（勾调分辨率，不勾转 JPEG）——现有默认行为
- **只转 JPEG**（不勾调分辨率，勾转 JPEG）——不缩放，仅把非 JPEG 子图转 JPEG 省空间
- **两者都做**（都勾）——现有勾选转 JPEG 行为

这是对现有工具的就地扩展，不改变 IMS 容器结构。

## 2. 决策清单

| 决策点 | 选择 |
|---|---|
| 都不勾选 | 前端禁止 + alert"请至少勾选一个操作" |
| 调分辨率默认勾选 | 默认勾选（向后兼容，现有行为不变） |
| 取消勾选后最长边输入框 | 禁用（灰掉）但保留值 |
| 后端实现 | `BatchResizeConfig` 加 `enable_resize: bool` 字段，`resize_ims_file` 加参数 |
| serde 默认值 | `#[serde(default = "default_true")]`（旧前端不传时默认 true，保持现有"总是调分辨率"行为），加 `fn default_true() -> bool { true }` 辅助 |

## 3. 核心逻辑

三种模式（`resize_ims_file` 内）：

| enable_resize | convert_to_jpeg | 行为 |
|---|---|---|
| true | false | 只调分辨率 |
| false | true | 只转 JPEG（所有非 JPEG 子图转，不缩放） |
| true | true | 调分辨率 + 转 JPEG |

`resize_ims_file` 的超标判断改为：
```rust
let oversized = enable_resize && longest > max_edge;
```

- `enable_resize=false` 时，`oversized` 恒为 false → 所有子图不缩放，只按 `convert_to_jpeg` 决定是否转码
- `Skipped` 判定不变：`resized_count == 0 && converted_count == 0`

### 3.1 子图处理矩阵（enable_resize=false, convert_to_jpeg=true 时）

| 子图格式 | 处理 |
|---|---|
| JPEG | 保留原字节（is_jpeg，不二次编码） |
| PNG/WebP/AVIF | 转 JPEG q90（不缩放，图尺寸不变） |

## 4. 组件设计

### 4.1 `ims_pack.rs` 改动

#### 4.1.1 `default_true` 辅助函数 + `BatchResizeConfig` 加字段

```rust
/// serde 默认值辅助：enable_resize 默认 true（向后兼容，保持"总是调分辨率"行为）
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResizeConfig {
    pub folders: Vec<FolderConfig>,
    /// 最长边像素下限 64
    pub max_edge: u32,
    pub multi_threaded: bool,
    /// 是否将所有非 JPEG 子图转为 JPEG（q90）
    #[serde(default)]
    pub convert_to_jpeg: bool,
    /// 是否调整分辨率（默认 true；false 时仅转 JPEG 不缩放）
    #[serde(default = "default_true")]
    pub enable_resize: bool,
}
```

#### 4.1.2 `resize_ims_file` 加参数

```rust
pub fn resize_ims_file(
    filepath: &str,
    max_edge: u32,
    convert_to_jpeg: bool,
    enable_resize: bool,
) -> Result<ResizeOutcome, String>
```

超标判断改为：
```rust
let longest = decoded.width().max(decoded.height());
let oversized = enable_resize && longest > max_edge;
```

其余逻辑不变（保留原字节分支、target_format、jpeg_quality、Skipped 判定、原子重写）。

### 4.2 `lib.rs` 改动

`start_batch_resize`：
- 校验改为："`enable_resize` 和 `convert_to_jpeg` 至少一个为 true"，否则 `Err("请至少勾选一个操作：调整分辨率或转为 JPEG")`
- `max_edge < 64` 校验仅在 `enable_resize=true` 时进行
- 传 `config.enable_resize` 给 `resize_ims_file`

```rust
if !config.enable_resize && !config.convert_to_jpeg {
    return Err("请至少勾选一个操作：调整分辨率或转为 JPEG".into());
}
if config.enable_resize && config.max_edge < 64 {
    return Err("最长边分辨率不能小于 64".into());
}
// ...
let enable_resize = config.enable_resize;
// ...
let outcome = resize_ims_file(fp, max_edge, convert_to_jpeg, enable_resize);
```

### 4.3 前端改动

#### 4.3.1 `index.html`

`#resize-modal` 的"最长边分辨率"行附近加"调整分辨率"复选框（默认 checked）：

```html
<div class="modal-options">
  <label><input type="checkbox" id="chk-resize-enable" checked> 调整分辨率</label>
  <label><input type="checkbox" id="chk-resize-to-jpeg"> 同时转为 JPEG (q90，显著减小体积)</label>
  <label><input type="checkbox" id="chk-resize-multi"> 多线程</label>
</div>
```

（"调整分辨率"复选框放在 `.modal-options` 最前，最长边输入框保留在原 `.output-dir-row`。）

#### 4.3.2 `app.js`

- `collectResizeConfig` 加 `enable_resize: $('#chk-resize-enable').checked`
- `startBatchResize` 校验：
  ```js
  if (!config.enable_resize && !config.convert_to_jpeg) {
    alert('请至少勾选一个操作：调整分辨率或转为 JPEG');
    return;
  }
  if (config.enable_resize && config.max_edge < 64) {
    alert('最长边分辨率不能小于 64');
    return;
  }
  if (config.enable_resize && config.max_edge > 32768) {
    if (!confirm('最长边超过 32768，确定继续吗？')) return;
  }
  ```
- 复选框 change 联动（在 `setupResizeEvents` 里）：
  ```js
  $('#chk-resize-enable').onchange = () => {
    $('#resize-max-edge').disabled = !$('#chk-resize-enable').checked;
  };
  ```
  `openResizeModal` 里初始化：`$('#chk-resize-enable').checked = true; $('#resize-max-edge').disabled = false;`

## 5. 关键实现细节与约束

- **向后兼容**：`enable_resize` 用 `#[serde(default = "default_true")]`，旧前端不传时默认 `true`，行为与现有工具一致（总是调分辨率）
- **校验分层**：前端 alert + 后端 Err 双重校验"至少勾选一个"；`max_edge` 校验仅在 `enable_resize=true` 时进行
- **UI 联动**：取消勾选"调整分辨率"时禁用最长边输入框（保留值），勾选时启用
- **不改变现有逻辑**：`enable_resize=true` 时行为与之前完全一致
- **不新增依赖**

## 6. 错误处理

沿用现有错误处理。`Skipped` 判定不变（`resized_count == 0 && converted_count == 0`）。只转 JPEG 模式下，全 JPEG 文件的子图都保留原字节 → `Skipped`。

## 7. 测试策略

在 `ims_pack.rs` 的 `#[cfg(test)] mod tests` 新增/适配：

- 现有测试适配：`resize_ims_file` 调用加第四参数 `true`（保持现有行为）
- `enable_resize_false_only_converts_jpeg`：单张未超标 PNG，`enable_resize=false, convert_to_jpeg=true` → `Resized{resized:0, converted:1}`，尺寸不变、format=0
- `enable_resize_false_oversized_png_not_resized`：单张超标 PNG（800×600），`enable_resize=false, convert_to_jpeg=true` → `Resized{resized:0, converted:1}`，**尺寸不变**（不缩放）、format=0
- `enable_resize_false_all_jpeg_skipped`：单张 JPEG，`enable_resize=false, convert_to_jpeg=true` → `Skipped`（已是 JPEG 不二次编码）
- `enable_resize_true_unchanged_behavior`：单张超标 PNG，`enable_resize=true, convert_to_jpeg=false` → `Resized{resized:1, converted:0}`，尺寸缩小（回归：现有行为不变）

## 8. 不在范围内（YAGNI）

- 不改变"转 JPEG"开关的现有行为
- 不加 JPEG 质量控件（固定 q90）
- 不改动批量转换工具（它有独立的 sub_images_to_jpeg）
- 不支持"只调分辨率但不转 JPEG 时禁用转 JPEG 复选框"（两开关独立，无联动除"至少勾一个"校验）

## 9. 改动文件清单

| 文件 | 改动 |
|---|---|
| `src-tauri/src/ims_pack.rs` | `default_true` 辅助、`BatchResizeConfig` 加 `enable_resize`、`resize_ims_file` 加参数与超标判断、测试适配+新增 |
| `src-tauri/src/lib.rs` | `start_batch_resize` 校验调整 + 传 `enable_resize` |
| `index.html` | `#resize-modal` 加 `chk-resize-enable` 复选框 |
| `src/app.js` | `collectResizeConfig` 加字段、`startBatchResize` 校验、复选框联动禁用最长边输入框 |

## 10. 开放问题

无。所有决策已在第 2 节确认。
