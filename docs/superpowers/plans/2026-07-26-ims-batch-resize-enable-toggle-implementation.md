# IMS 批量调分辨率工具 "调分辨率可开关" 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把批量调分辨率工具的"调分辨率"功能改为可开关：新增"调整分辨率"复选框（默认勾选），与"同时转为 JPEG"配合支持三种模式（只调分辨率 / 只转 JPEG / 两者都做），都不勾时前端禁止。

**Architecture:** `BatchResizeConfig` 加 `enable_resize: bool`（`#[serde(default = "default_true")]` 向后兼容默认 true，加 `fn default_true()` 辅助）；`resize_ims_file` 加 `enable_resize` 参数，超标判断改为 `enable_resize && longest > max_edge`；`start_batch_resize` 校验"至少勾选一个"、`max_edge` 校验仅在 `enable_resize=true` 时；前端 modal 加复选框（默认 checked）、`collectResizeConfig` 加字段、校验"至少勾一个"、复选框 change 联动禁用最长边输入框。

**Tech Stack:** Rust 2021（`image` 0.25、`avif-rs`）、Tauri v2、原生 JS。

## Global Constraints

- **不新增** Cargo 依赖；**不新增** CSS；**不改动** `ims.rs`、`Cargo.toml`、`styles.css`
- 格式码：JPEG=0, PNG=1, WebP=2, AVIF=3
- `enable_resize` 用 `#[serde(default = "default_true")]`（旧前端不传时默认 true，保持现有"总是调分辨率"行为），需加 `fn default_true() -> bool { true }`
- `resize_ims_file` 新签名：`(filepath: &str, max_edge: u32, convert_to_jpeg: bool, enable_resize: bool) -> Result<ResizeOutcome, String>`
- 超标判断改为 `let oversized = enable_resize && longest > max_edge;`
- `Skipped` 判定不变：`resized_count == 0 && converted_count == 0`
- `start_batch_resize` 校验：`!enable_resize && !convert_to_jpeg` → `Err("请至少勾选一个操作：调整分辨率或转为 JPEG")`；`max_edge < 64` 校验仅在 `enable_resize=true` 时
- 前端：复选框 `#chk-resize-enable` 默认 checked；都不勾 alert"请至少勾选一个操作：调整分辨率或转为 JPEG"；取消勾选禁用 `#resize-max-edge`（保留值）
- 命令名 `start_batch_resize` 不变；事件复用 `batch-progress`
- 文件结构码与提交信息沿用现有中文风格

---

## File Structure

| 文件 | 责任 | 改动类型 |
|---|---|---|
| `src-tauri/src/ims_pack.rs` | 核心：default_true 辅助、config 字段、resize_ims_file 参数与超标判断、测试适配+新增 | 修改 |
| `src-tauri/src/lib.rs` | start_batch_resize 校验调整 + 传参 | 修改 |
| `index.html` | resize modal 加复选框 | 修改 |
| `src/app.js` | collectResizeConfig 加字段、校验、复选框联动 | 修改 |

---

## Task 1: `default_true` 辅助 + `BatchResizeConfig` 加字段 + `resize_ims_file` 加参数（`ims_pack.rs`）

**Files:**
- Modify: `src-tauri/src/ims_pack.rs:175-184`（BatchResizeConfig）、`ims_pack.rs:591-595`（resize_ims_file 签名）、`ims_pack.rs:618`（oversized 判断）

**Interfaces:**
- Produces: `fn default_true() -> bool`
- Produces: `BatchResizeConfig { ..., enable_resize: bool }`（`#[serde(default = "default_true")]`）
- Produces: `pub fn resize_ims_file(filepath: &str, max_edge: u32, convert_to_jpeg: bool, enable_resize: bool) -> Result<ResizeOutcome, String>`

- [ ] **Step 1: 在 `BatchResizeConfig` 之前（第 175 行 `#[derive(...)]` 之前）加 `default_true` 辅助函数**

```rust
/// serde 默认值辅助：enable_resize 默认 true（向后兼容，保持"总是调分辨率"行为）
fn default_true() -> bool {
    true
}
```

- [ ] **Step 2: `BatchResizeConfig` 加 `enable_resize` 字段（第 183 行 `convert_to_jpeg` 之后）**

将第 176-184 行：
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
    /// 是否调整分辨率（默认 true；false 时仅转 JPEG 不缩放）
    #[serde(default = "default_true")]
    pub enable_resize: bool,
}
```

- [ ] **Step 3: `resize_ims_file` 签名加 `enable_resize` 参数（第 591-595 行）**

将：
```rust
pub fn resize_ims_file(
    filepath: &str,
    max_edge: u32,
    convert_to_jpeg: bool,
) -> Result<ResizeOutcome, String> {
```
改为：
```rust
pub fn resize_ims_file(
    filepath: &str,
    max_edge: u32,
    convert_to_jpeg: bool,
    enable_resize: bool,
) -> Result<ResizeOutcome, String> {
```

- [ ] **Step 4: 超标判断改为 `enable_resize && longest > max_edge`（第 618 行）**

将：
```rust
        let oversized = longest > max_edge;
```
改为：
```rust
        let oversized = enable_resize && longest > max_edge;
```

- [ ] **Step 5: 编译确认（会有 lib.rs 调用点和现有测试因签名变化报错，属预期——Task 2/3 修复）**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | grep -E "ims_pack|error\[" | head -20`
Expected: `ims_pack.rs` 非测试代码无错；报错来自 `lib.rs`（`resize_ims_file` 少参数）和 `ims_pack.rs` 测试模块（现有测试少参数）。若 `ims_pack.rs` 非测试代码有错先修复。

- [ ] **Step 6: 提交（与 Task 2/3 一起编译通过后提交）**

---

## Task 2: `start_batch_resize` 校验调整 + 传参（`lib.rs`）

**Files:**
- Modify: `src-tauri/src/lib.rs:1332`（校验）、`lib.rs:1364`（提取）、`lib.rs:1381/1430`（两处调用）

**Interfaces:**
- Consumes: `resize_ims_file(filepath, max_edge, convert_to_jpeg, enable_resize)`（Task 1）、`BatchResizeConfig.enable_resize`（Task 1）

- [ ] **Step 1: 校验调整（第 1332 行）**

将：
```rust
    if config.max_edge < 64 {
        return Err("最长边分辨率不能小于 64".into());
    }
```
改为：
```rust
    if !config.enable_resize && !config.convert_to_jpeg {
        return Err("请至少勾选一个操作：调整分辨率或转为 JPEG".into());
    }
    if config.enable_resize && config.max_edge < 64 {
        return Err("最长边分辨率不能小于 64".into());
    }
```

- [ ] **Step 2: 提取 `enable_resize`（第 1364 行 `let max_edge = config.max_edge;` 之后）**

将：
```rust
    let max_edge = config.max_edge;
    let convert_to_jpeg = config.convert_to_jpeg;
```
改为：
```rust
    let max_edge = config.max_edge;
    let convert_to_jpeg = config.convert_to_jpeg;
    let enable_resize = config.enable_resize;
```

- [ ] **Step 3: 两处 `resize_ims_file` 调用加第四参数（第 1381 行 rayon 分支、第 1430 行串行分支）**

将两处：
```rust
                let outcome = resize_ims_file(fp, max_edge, convert_to_jpeg);
```
改为：
```rust
                let outcome = resize_ims_file(fp, max_edge, convert_to_jpeg, enable_resize);
```

- [ ] **Step 4: 编译确认**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | grep -E "error" | head -10`
Expected: 仅 `ims_pack.rs` 测试模块报错（现有测试少参数，Task 3 修复）；`lib.rs` 和 `ims_pack.rs` 非测试代码无错。

- [ ] **Step 5: 提交（与 Task 1/3 一起）**

---

## Task 3: 适配现有测试 + 新增 enable_resize 测试（`ims_pack.rs`）

**Files:**
- Modify: `src-tauri/src/ims_pack.rs`（`#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `resize_ims_file(filepath, max_edge, convert_to_jpeg, enable_resize)`（Task 1）

- [ ] **Step 1: 适配现有 11 个 `resize_ims_file` 测试调用（加第四参数）**

现有测试调用（`ims_pack.rs` 第 1168/1177/1199/1218/1251/1323/1348/1357/1381/1390/1472/1527 行附近）都是 `resize_ims_file(&ims_path, N, BOOL)` 三参数形式。全部加第四参数 `true`（保持现有"总是调分辨率"行为）：
- `resize_ims_file(&ims_path, N, false)` → `resize_ims_file(&ims_path, N, false, true)`
- `resize_ims_file(&ims_path, N, true)` → `resize_ims_file(&ims_path, N, true, true)`

用编辑器批量替换：把 `, false)` 结尾的 resize_ims_file 调用改为 `, false, true)`，`, true)` 结尾的改为 `, true, true)`。注意只改 `resize_ims_file` 调用，不要误改其他函数。

- [ ] **Step 2: 运行现有 resize 测试确认适配后通过**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib ims_pack::tests:: 2>&1 | tail -8`
Expected: 全部通过（21 个 ims_pack 测试）

- [ ] **Step 3: 新增 enable_resize 测试（在 `mod tests` 末尾追加）**

```rust
    #[test]
    fn enable_resize_false_only_converts_jpeg() {
        // 未超标 PNG + enable_resize=false, convert_to_jpeg=true → 转 JPEG（不缩放）
        let (_dir, ims_path) = make_ims_single(1, 100, 100, false);
        let outcome = resize_ims_file(&ims_path, 400, true, false).unwrap();
        match outcome {
            ResizeOutcome::Resized {
                sub_images_resized,
                sub_images_converted,
            } => {
                assert_eq!(sub_images_resized, 0, "enable_resize=false 不应缩放");
                assert_eq!(sub_images_converted, 1);
            }
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        assert_eq!(info.images[0].format, 0, "应转为 JPEG");
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (100, 100), "尺寸不应改变");
    }

    #[test]
    fn enable_resize_false_oversized_png_not_resized() {
        // 超标 PNG（800×600）+ enable_resize=false, convert_to_jpeg=true → 转 JPEG 但不缩放
        let (_dir, ims_path) = make_ims_single(1, 800, 600, false);
        let outcome = resize_ims_file(&ims_path, 400, true, false).unwrap();
        match outcome {
            ResizeOutcome::Resized {
                sub_images_resized,
                sub_images_converted,
            } => {
                assert_eq!(sub_images_resized, 0, "enable_resize=false 不应缩放");
                assert_eq!(sub_images_converted, 1);
            }
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (800, 600),
            "enable_resize=false 时尺寸不应改变（不缩放）"
        );
    }

    #[test]
    fn enable_resize_false_all_jpeg_skipped() {
        // 单张 JPEG + enable_resize=false, convert_to_jpeg=true → Skipped（已是 JPEG 不二次编码）
        let (_dir, ims_path) = make_ims_single(0, 100, 100, false);
        let original = std::fs::read(&ims_path).unwrap();
        let outcome = resize_ims_file(&ims_path, 400, true, false).unwrap();
        assert!(matches!(outcome, ResizeOutcome::Skipped));
        assert_eq!(std::fs::read(&ims_path).unwrap(), original, "文件不应改动");
    }

    #[test]
    fn enable_resize_true_unchanged_behavior() {
        // 超标 PNG + enable_resize=true, convert_to_jpeg=false → 缩小（回归：现有行为不变）
        let (_dir, ims_path) = make_ims_single(1, 800, 600, false);
        let outcome = resize_ims_file(&ims_path, 400, false, true).unwrap();
        match outcome {
            ResizeOutcome::Resized {
                sub_images_resized,
                sub_images_converted,
            } => {
                assert_eq!(sub_images_resized, 1);
                assert_eq!(sub_images_converted, 0);
            }
            _ => panic!("expected Resized"),
        }
        let info = parse_ims(&ims_path).unwrap();
        let sub = read_sub_image(&ims_path, info.images[0].data_offset, info.images[0].data_length)
            .unwrap();
        let decoded = image::load_from_memory(&sub).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (400, 300), "应缩小");
    }
```

注：`make_ims_single(format, w, h, rgba)` 辅助已存在（此前为转 JPEG 测试添加）。若不存在，参考现有 `convert_to_jpeg_*` 测试用的辅助。

- [ ] **Step 4: 运行全部 resize 测试**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib ims_pack::tests:: 2>&1 | tail -8`
Expected: 全部通过（21 + 4 = 25 个 ims_pack 测试）

- [ ] **Step 5: 运行完整后端套件**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib 2>&1 | tail -6`
Expected: 全部通过（49 + 4 = 53 个）

- [ ] **Step 6: 提交（Task 1-3 一起）**

```bash
git add src-tauri/src/ims_pack.rs src-tauri/src/lib.rs
git commit -m "feat: make resize optional via enable_resize toggle in batch resize"
```

---

## Task 4: 前端 — modal 加复选框 + 校验 + 联动（`index.html` + `app.js`）

**Files:**
- Modify: `index.html:404-406`（`.modal-options`）、`src/app.js:1214-1226`（collectResizeConfig）、`src/app.js:1228+`（startBatchResize 校验）、`src/app.js:1170-1181`（openResizeModal）、`src/app.js:1271-1277`（setupResizeEvents）

**Interfaces:**
- Produces: `#chk-resize-enable` 复选框（默认 checked）；`collectResizeConfig` 返回 `enable_resize: bool`

- [ ] **Step 1: 在 `#resize-modal` 的 `.modal-options` 中（`index.html` 第 405 行 `chk-resize-multi` 之前）加"调整分辨率"复选框**

将第 404-406 行：
```html
        <div class="modal-options">
          <label><input type="checkbox" id="chk-resize-multi"> 多线程</label>
        </div>
```
改为：
```html
        <div class="modal-options">
          <label><input type="checkbox" id="chk-resize-enable" checked> 调整分辨率</label>
          <label><input type="checkbox" id="chk-resize-multi"> 多线程</label>
        </div>
```

注：转 JPEG 复选框 `#chk-resize-to-jpeg` 已在 `.modal-options` 里（此前添加），确认它在 `chk-resize-enable` 之后。实际当前 `.modal-options` 内容可能是 `chk-resize-to-jpeg` + `chk-resize-multi` 两个，需把 `chk-resize-enable` 加在最前。

- [ ] **Step 2: `collectResizeConfig` 加 `enable_resize` 字段（第 1214-1226 行）**

将：
```js
  const maxEdge = parseInt($('#resize-max-edge').value) || 0;
  return {
    folders,
    max_edge: maxEdge,
    multi_threaded: $('#chk-resize-multi').checked,
    convert_to_jpeg: $('#chk-resize-to-jpeg').checked,
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
    enable_resize: $('#chk-resize-enable').checked,
  };
```

- [ ] **Step 3: `startBatchResize` 校验调整（第 1228 行附近）**

将现有校验：
```js
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
```
改为：
```js
  if (config.folders.length === 0) {
    alert('请至少添加一个文件夹');
    return;
  }
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

- [ ] **Step 4: `openResizeModal` 初始化复选框状态（第 1170-1181 行）**

在 `openResizeModal` 里（`$('#resize-max-edge').value = '2560';` 附近）加：
```js
  $('#chk-resize-enable').checked = true;
  $('#resize-max-edge').disabled = false;
```

- [ ] **Step 5: `setupResizeEvents` 加复选框 change 联动（第 1271-1277 行）**

在 `setupResizeEvents` 里加：
```js
  $('#chk-resize-enable').onchange = () => {
    $('#resize-max-edge').disabled = !$('#chk-resize-enable').checked;
  };
```

- [ ] **Step 6: 前端构建验证**

Run: `cd f:/Workspace/imsViewer && npm run build 2>&1 | tail -6`
Expected: Vite 构建成功，无 JS 错误

- [ ] **Step 7: 提交**

```bash
git add index.html src/app.js
git commit -m "feat: add enable-resize checkbox with validation and max-edge toggle in resize modal"
```

---

## Task 5: 集成构建与验证

**Files:**
- 无代码改动；本任务为构建与验证

- [ ] **Step 1: 后端完整测试**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo test --lib 2>&1 | tail -6`
Expected: 全部通过（53 个）

- [ ] **Step 2: cargo check 警告检查**

Run: `cd src-tauri && LIBCLANG_PATH="<libclang路径>" cargo check 2>&1 | tail -6`
Expected: 编译通过，无新增警告

- [ ] **Step 3: 前端构建**

Run: `cd f:/Workspace/imsViewer && npm run build 2>&1 | tail -6`
Expected: Vite 构建成功

- [ ] **Step 4: 手工验收（若可交互）**

- 工具 → 批量调整分辨率 → 默认勾选"调整分辨率"（行为与之前一致）
- 取消勾选"调整分辨率" → 最长边输入框禁用；勾选"转 JPEG" → 只转 JPEG 不缩放
- 两个都不勾 → alert"请至少勾选一个操作"
- 验证只转 JPEG 模式：PNG 子图转 JPEG 但尺寸不变

- [ ] **Step 5: 最终提交（若有验收修复）**

无问题则无需提交。

---

## Self-Review 已完成

- **Spec coverage**：设计 §2 决策表逐条对应（都不勾禁止→Task2 Step1 + Task4 Step3、默认勾选→Task4 Step1 `checked`、禁用最长边→Task4 Step5、enable_resize 字段→Task1 Step2、default_true→Task1 Step1）。§3 三种模式→Task1 Step4 `enable_resize && longest > max_edge`。§3.1 子图矩阵→Task3 新增测试。§4.2 校验→Task2。§4.3 前端→Task4。§7 测试→Task3 全部 4 个新测试。
- **Placeholder scan**：无 TBD/TODO；所有代码步骤给出完整代码；测试用例给出完整断言。
- **Type consistency**：`resize_ims_file(filepath, max_edge, convert_to_jpeg, enable_resize)`（Task1 定义、Task2 调用、Task3 测试一致）；`BatchResizeConfig.enable_resize`（Task1 定义、Task2 提取、Task4 前端字段一致）；`default_true()`（Task1 定义与 serde 引用一致）。
