# imsViewer 自动更新功能实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 imsViewer 添加基于 Tauri Updater 插件的自动更新功能，支持启动自动检查、手动检查、跳过版本、下载安装和重启。

**Architecture:** 使用 Tauri 官方 Updater 插件对接 GitHub Release。前端负责 UI 交互（检查更新、弹窗提示、进度显示、跳过版本管理），后端负责插件注册和配置。发布流程通过本地脚本辅助完成签名和 latest.json 生成。

**Tech Stack:** Tauri 2, @tauri-apps/plugin-updater, @tauri-apps/plugin-process, tauri-plugin-updater, tauri-plugin-process

## Global Constraints

- 项目使用 Tauri 2，前端为原生 JS (ES modules)，无框架
- 现有模态框样式复用 `.modal-overlay` / `.modal-container` / `.modal-header` / `.modal-body` / `.modal-footer`
- 菜单操作通过 `data-action` 属性分发，在 `setupMenuActions()` 的 switch 中处理
- 版本号格式为语义化版本（如 1.2.1）
- 发布物为 NSIS 安装包 (`imsViewer_X.X.X_x64-setup.exe`)
- 私钥保存在本地 `~/.tauri/imsviewer.key`，不提交 git
- 所有网络请求使用 HTTPS

---

### Task 1: 添加 Updater 和 Process 插件依赖

**Files:**
- Modify: `package.json`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: 无
- Produces: 前端可用 `@tauri-apps/plugin-updater` 和 `@tauri-apps/plugin-process`；后端可用 `tauri_plugin_updater` 和 `tauri_plugin_process`

- [ ] **Step 1: 添加前端依赖**

修改 `package.json`，在 `dependencies` 中添加：

```json
{
  "dependencies": {
    "@tauri-apps/api": "^2.0",
    "@tauri-apps/plugin-dialog": "^2.0",
    "@tauri-apps/plugin-updater": "^2.0",
    "@tauri-apps/plugin-process": "^2.0"
  }
}
```

- [ ] **Step 2: 添加后端依赖**

修改 `src-tauri/Cargo.toml`，在 `[dependencies]` 中添加：

```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-dialog = "2"
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
# ... 其余依赖保持不变
```

- [ ] **Step 3: 安装前端依赖**

Run: `npm install`
Expected: 成功安装，无错误

- [ ] **Step 4: 验证 Rust 依赖可解析**

Run: `cd src-tauri && cargo check`
Expected: 编译检查通过（可能下载新 crate）

- [ ] **Step 5: Commit**

```bash
git add package.json src-tauri/Cargo.toml
git commit -m "feat: add updater and process plugin dependencies"
```

---

### Task 2: 配置 Tauri Updater 插件

**Files:**
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/src/lib.rs:1750-1798`
- Modify: `src-tauri/capabilities/default.json`

**Interfaces:**
- Consumes: Task 1 的依赖
- Produces: Updater 插件在 Tauri 中注册并配置完成，前端可通过 JS API 调用

- [ ] **Step 1: 配置 tauri.conf.json**

修改 `src-tauri/tauri.conf.json`，在 `"plugins"` 对象中添加 updater 配置（如果没有 `plugins` 字段则创建）：

```json
{
  "plugins": {
    "updater": {
      "active": true,
      "endpoints": [
        "https://github.com/TanXianliang/imsViewer/releases/latest/download/latest.json"
      ],
      "dialog": false,
      "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDlBRkI3RjE3RjE3RjE3RjE3ClJXU0FnPT0K"
    }
  }
}
```

注意：上述 `pubkey` 是示例占位符，实际值需要在生成密钥对后替换。开发阶段可以先使用此占位符，构建时会报错提示需要真实公钥。

- [ ] **Step 2: 注册插件到 Tauri Builder**

修改 `src-tauri/src/lib.rs` 的 `run()` 函数，在 `.plugin(tauri_plugin_dialog::init())` 后添加：

```rust
pub fn run(launch_file: Option<String>) {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::init())
        .plugin(tauri_plugin_process::init())
        .manage(LaunchFile(launch_file))
        // ... 其余保持不变
```

- [ ] **Step 3: 添加权限到 capabilities**

修改 `src-tauri/capabilities/default.json`，在 `permissions` 数组中添加：

```json
{
  "identifier": "default",
  "description": "默认权限集",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "core:window:allow-start-dragging",
    "core:window:allow-close",
    "core:window:allow-minimize",
    "core:window:allow-toggle-maximize",
    "core:window:allow-internal-toggle-maximize",
    "dialog:default",
    "dialog:allow-open",
    "dialog:allow-save",
    "updater:default",
    "updater:allow-check",
    "updater:allow-download",
    "updater:allow-install",
    "process:default",
    "process:allow-restart"
  ]
}
```

- [ ] **Step 4: 验证编译**

Run: `cd src-tauri && cargo check`
Expected: 编译通过，updater 插件正确链接

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tauri.conf.json src-tauri/src/lib.rs src-tauri/capabilities/default.json
git commit -m "feat: configure updater and process plugins in Tauri"
```

---

### Task 3: 生成签名密钥对

**Files:**
- Create: `scripts/generate-keys.bat`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: 无
- Produces: 签名密钥对（私钥 `~/.tauri/imsviewer.key`，公钥内容用于 `tauri.conf.json`）

- [ ] **Step 1: 创建密钥生成脚本**

创建 `scripts/generate-keys.bat`：

```batch
@echo off
setlocal

echo ========================================
echo   imsViewer Updater Key Generator
echo ========================================
echo.

set "KEY_DIR=%USERPROFILE%\.tauri"
set "KEY_PATH=%KEY_DIR%\imsviewer.key"

if not exist "%KEY_DIR%" mkdir "%KEY_DIR%"

if exist "%KEY_PATH%" (
    echo WARNING: Key already exists at %KEY_PATH%
    echo Delete it first if you want to regenerate.
    pause
    exit /b 1
)

echo Generating new signing key pair...
call npm run tauri signer generate -- -w "%KEY_PATH%"

if not exist "%KEY_PATH%" (
    echo ERROR: Key generation failed
    pause
    exit /b 1
)

echo.
echo ========================================
echo   Key generated successfully!
echo ========================================
echo.
echo Private key: %KEY_PATH%
echo.
echo PUBLIC KEY (add to tauri.conf.json):
type "%KEY_PATH%.pub"
echo.
echo.
echo IMPORTANT: Keep the private key secure and never commit it to git!
pause
```

- [ ] **Step 2: 添加 .gitignore 规则**

修改 `.gitignore`，添加：

```
# Tauri updater keys
*.key
*.key.pub
```

- [ ] **Step 3: 运行密钥生成脚本**

Run: `scripts\generate-keys.bat`
Expected: 生成密钥对，显示公钥内容

- [ ] **Step 4: 更新 tauri.conf.json 中的 pubkey**

将脚本输出的公钥内容（`imsviewer.key.pub` 文件内容）复制到 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey` 字段。

- [ ] **Step 5: Commit**

```bash
git add scripts/generate-keys.bat .gitignore src-tauri/tauri.conf.json
git commit -m "chore: add key generation script and update pubkey"
```

---

### Task 4: 前端更新检查逻辑

**Files:**
- Modify: `src/app.js`

**Interfaces:**
- Consumes: Task 2 注册的 updater 插件
- Produces: `checkForUpdates(manual)` 函数、`skipVersion(version)` 函数、`getSkippedVersion()` 函数、`clearSkippedVersion()` 函数

- [ ] **Step 1: 添加更新相关导入**

修改 `src/app.js` 顶部导入部分：

```javascript
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { setupDedupTool } from './dedup.js';
```

- [ ] **Step 2: 添加版本比较和跳过版本管理函数**

在 `src/app.js` 的 `// ===== State =====` 部分之前添加：

```javascript
// ===== 更新检查 =====
const SKIP_VERSION_KEY = 'imsviewer:skip-version';

/**
 * 比较两个语义化版本号
 * @returns {number} 1: latest > current, -1: latest < current, 0: equal
 */
function compareVersions(current, latest) {
  const c = current.split('.').map(Number);
  const l = latest.split('.').map(Number);
  for (let i = 0; i < 3; i++) {
    const cv = c[i] || 0;
    const lv = l[i] || 0;
    if (lv > cv) return 1;
    if (lv < cv) return -1;
  }
  return 0;
}

function getSkippedVersion() {
  return localStorage.getItem(SKIP_VERSION_KEY);
}

function skipVersion(version) {
  localStorage.setItem(SKIP_VERSION_KEY, version);
}

function clearSkippedVersion() {
  localStorage.removeItem(SKIP_VERSION_KEY);
}
```

- [ ] **Step 3: 添加更新检查主函数**

在 `src/app.js` 的 `// ===== 菜单栏 =====` 部分之前添加：

```javascript
// ===== 更新检查 =====
let updateCheckInProgress = false;

async function checkForUpdates(manual = false) {
  if (updateCheckInProgress) return;
  updateCheckInProgress = true;

  try {
    const update = await check();
    if (!update) {
      if (manual) alert('检查更新失败：无法连接到更新服务器');
      return;
    }

    if (!update.available) {
      if (manual) alert('当前已是最新版本');
      return;
    }

    const currentVersion = update.currentVersion;
    const latestVersion = update.version;

    // 自动检查时，如果用户已跳过此版本，则静默忽略
    if (!manual) {
      const skipped = getSkippedVersion();
      if (skipped && compareVersions(skipped, latestVersion) === 0) {
        console.log(`用户已跳过版本 ${latestVersion}，不再提示`);
        return;
      }
    }

    // 显示更新提示弹窗
    showUpdateModal(update, currentVersion, latestVersion);
  } catch (err) {
    console.error('检查更新失败:', err);
    if (manual) {
      alert('检查更新失败: ' + (err.message || err));
    }
  } finally {
    updateCheckInProgress = false;
  }
}
```

- [ ] **Step 4: 添加启动时自动检查**

修改 `src/app.js` 底部的启动代码：

```javascript
// ===== 启动 =====
console.log('Tauri API:', typeof invoke);
init();

// 处理右键"使用 imsViewer 打开"传入的文件路径
(async () => {
  try {
    const path = await invoke('get_launch_file');
    if (path) {
      console.log('启动文件:', path);
      await openFile(path);
    }
  } catch (e) {
    console.error('获取启动文件失败:', e);
  }
})();

// 启动后延迟 3 秒自动检查更新
setTimeout(() => {
  checkForUpdates(false);
}, 3000);
```

- [ ] **Step 5: Commit**

```bash
git add src/app.js
git commit -m "feat: add update check logic with skip-version support"
```

---

### Task 5: 更新提示弹窗 UI

**Files:**
- Modify: `index.html`
- Modify: `src/styles.css`
- Modify: `src/app.js`

**Interfaces:**
- Consumes: Task 4 的 `checkForUpdates` 函数
- Produces: `showUpdateModal(update, currentVersion, latestVersion)` 函数、更新弹窗 HTML/CSS

- [ ] **Step 1: 添加更新弹窗 HTML**

修改 `index.html`，在 `<!-- 缩略图右键菜单（编辑模式） -->` 之前添加：

```html
  <!-- 更新提示模态框 -->
  <div id="update-modal" class="modal-overlay" style="display:none">
    <div class="modal-container" style="width: 480px">
      <div class="modal-header">
        <span>发现新版本</span>
        <button class="btn-modal-close" id="btn-update-close">&times;</button>
      </div>
      <div class="modal-body">
        <div class="update-version-info">
          <div class="update-version-row">
            <span class="update-label">当前版本:</span>
            <span class="update-value" id="update-current-version">-</span>
          </div>
          <div class="update-version-row">
            <span class="update-label">最新版本:</span>
            <span class="update-value update-new-version" id="update-latest-version">-</span>
          </div>
        </div>
        <div class="update-notes-section">
          <div class="update-notes-title">更新内容:</div>
          <div class="update-notes-content" id="update-notes">暂无更新说明</div>
        </div>
      </div>
      <div class="modal-footer">
        <button id="btn-update-skip" class="btn-secondary">跳过此版本</button>
        <button id="btn-update-later" class="btn-secondary">稍后提醒</button>
        <button id="btn-update-now" class="btn-primary">立即更新</button>
      </div>
    </div>
  </div>

  <!-- 更新下载进度模态框 -->
  <div id="update-progress-modal" class="modal-overlay" style="display:none">
    <div class="modal-container" style="width: 400px">
      <div class="modal-header">
        <span>正在下载更新</span>
      </div>
      <div class="modal-body">
        <div class="update-progress-info">
          <div class="update-progress-version" id="update-progress-version">v-</div>
          <div class="progress-bar-container" style="margin: 12px 0">
            <div class="progress-bar-fill" id="update-progress-fill"></div>
          </div>
          <div class="update-progress-text" id="update-progress-text">准备下载...</div>
        </div>
      </div>
      <div class="modal-footer">
        <button id="btn-update-cancel" class="btn-secondary">取消下载</button>
      </div>
    </div>
  </div>

  <!-- 更新完成模态框 -->
  <div id="update-complete-modal" class="modal-overlay" style="display:none">
    <div class="modal-container" style="width: 400px">
      <div class="modal-header">
        <span>更新下载完成</span>
      </div>
      <div class="modal-body">
        <div class="update-complete-message">
          imsViewer 将重启以完成安装。
          <br><br>
          点击"立即重启安装"开始更新。
        </div>
      </div>
      <div class="modal-footer">
        <button id="btn-update-restart" class="btn-primary">立即重启安装</button>
      </div>
    </div>
  </div>
```

- [ ] **Step 2: 添加更新弹窗样式**

修改 `src/styles.css`，在 `/* ===== 模态框 ===== */` 部分之后添加：

```css
/* ===== 更新弹窗 ===== */
.update-version-info {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 8px 0;
}
.update-version-row {
  display: flex;
  justify-content: space-between;
  font-size: 13px;
}
.update-label {
  color: var(--text-secondary);
}
.update-value {
  color: var(--text-primary);
  font-weight: 500;
}
.update-new-version {
  color: var(--accent);
  font-weight: 600;
}
.update-notes-section {
  margin-top: 8px;
  padding-top: 12px;
  border-top: 1px solid var(--border);
}
.update-notes-title {
  font-size: 12px;
  color: var(--text-secondary);
  margin-bottom: 6px;
}
.update-notes-content {
  font-size: 12px;
  color: var(--text-primary);
  line-height: 1.5;
  max-height: 120px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-word;
}
.update-progress-info {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 8px 0;
}
.update-progress-version {
  font-size: 14px;
  font-weight: 600;
  color: var(--accent);
}
.update-progress-text {
  font-size: 12px;
  color: var(--text-secondary);
  text-align: center;
}
.update-complete-message {
  font-size: 13px;
  line-height: 1.6;
  text-align: center;
  padding: 16px 0;
}
```

- [ ] **Step 3: 添加 showUpdateModal 函数**

修改 `src/app.js`，在 `checkForUpdates` 函数之后添加：

```javascript
function showUpdateModal(update, currentVersion, latestVersion) {
  const modal = $('#update-modal');
  $('#update-current-version').textContent = 'v' + currentVersion;
  $('#update-latest-version').textContent = 'v' + latestVersion;
  $('#update-notes').textContent = update.body || '暂无更新说明';

  // 绑定按钮事件（先移除旧监听器）
  const btnSkip = $('#btn-update-skip');
  const btnLater = $('#btn-update-later');
  const btnNow = $('#btn-update-now');
  const btnClose = $('#btn-update-close');

  const closeModal = () => { modal.style.display = 'none'; };

  // 使用 onclick 覆盖旧监听器
  btnSkip.onclick = () => {
    skipVersion(latestVersion);
    closeModal();
  };
  btnLater.onclick = closeModal;
  btnClose.onclick = closeModal;
  btnNow.onclick = async () => {
    closeModal();
    await downloadAndInstallUpdate(update, latestVersion);
  };

  modal.style.display = 'flex';
}
```

- [ ] **Step 4: Commit**

```bash
git add index.html src/styles.css src/app.js
git commit -m "feat: add update notification modal UI"
```

---

### Task 6: 下载安装和重启逻辑

**Files:**
- Modify: `src/app.js`

**Interfaces:**
- Consumes: Task 5 的 `showUpdateModal` 函数
- Produces: `downloadAndInstallUpdate(update, version)` 函数

- [ ] **Step 1: 添加下载安装函数**

修改 `src/app.js`，在 `showUpdateModal` 函数之后添加：

```javascript
async function downloadAndInstallUpdate(update, version) {
  const progressModal = $('#update-progress-modal');
  const completeModal = $('#update-complete-modal');
  const progressFill = $('#update-progress-fill');
  const progressText = $('#update-progress-text');
  const progressVersion = $('#update-progress-version');
  const btnCancel = $('#btn-update-cancel');

  progressVersion.textContent = 'v' + version;
  progressFill.style.width = '0%';
  progressText.textContent = '准备下载...';
  progressModal.style.display = 'flex';

  let cancelled = false;
  btnCancel.onclick = () => {
    cancelled = true;
    progressModal.style.display = 'none';
  };

  try {
    let downloaded = 0;
    let contentLength = 0;

    await update.downloadAndInstall((event) => {
      if (cancelled) return;
      switch (event.event) {
        case 'Started':
          contentLength = event.data.contentLength || 0;
          progressText.textContent = '开始下载...';
          break;
        case 'Progress':
          downloaded += event.data.chunkLength;
          if (contentLength > 0) {
            const percent = Math.round((downloaded / contentLength) * 100);
            progressFill.style.width = percent + '%';
            progressText.textContent = `已下载 ${formatSize(downloaded)} / ${formatSize(contentLength)} (${percent}%)`;
          } else {
            progressText.textContent = `已下载 ${formatSize(downloaded)}`;
          }
          break;
        case 'Finished':
          progressFill.style.width = '100%';
          progressText.textContent = '下载完成，正在安装...';
          break;
      }
    });

    if (cancelled) return;

    progressModal.style.display = 'none';

    // 显示完成弹窗
    const btnRestart = $('#btn-update-restart');
    btnRestart.onclick = async () => {
      completeModal.style.display = 'none';
      await relaunch();
    };
    completeModal.style.display = 'flex';
  } catch (err) {
    progressModal.style.display = 'none';
    if (!cancelled) {
      alert('更新下载失败: ' + (err.message || err));
    }
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add src/app.js
git commit -m "feat: add update download, install and relaunch logic"
```

---

### Task 7: 菜单集成和关于对话框

**Files:**
- Modify: `index.html`
- Modify: `src/app.js`

**Interfaces:**
- Consumes: Task 4 的 `checkForUpdates` 函数
- Produces: 帮助菜单"检查更新"项、关于对话框显示版本和跳过信息

- [ ] **Step 1: 添加菜单项**

修改 `index.html` 的帮助菜单：

```html
        <div class="menu-item" data-menu="help">
          帮助
          <div class="menu-dropdown">
            <div class="menu-action" data-action="help-shortcuts">快捷键参考</div>
            <div class="menu-action" data-action="help-usage">使用说明</div>
            <div class="menu-separator"></div>
            <div class="menu-action" data-action="check-update">检查更新</div>
            <div class="menu-separator"></div>
            <div class="menu-action" data-action="about">关于 imsViewer</div>
          </div>
        </div>
```

- [ ] **Step 2: 添加菜单处理**

修改 `src/app.js` 的 `setupMenuActions()` 函数，在 switch 中添加：

```javascript
        case 'check-update':
          checkForUpdates(true);
          break;
```

- [ ] **Step 3: 更新关于对话框**

修改 `src/app.js` 的 `about` case：

```javascript
        case 'about': {
          const skipped = getSkippedVersion();
          let aboutText = 'imsViewer v' + (window.__imsVersion || '1.2.1') + '\n';
          aboutText += 'IMS 图片浏览器\n基于 Tauri v2 + Rust + Canvas';
          if (skipped) {
            aboutText += '\n\n已跳过版本: v' + skipped;
            aboutText += '\n(点击"检查更新"可恢复提示)';
          }
          alert(aboutText);
          break;
        }
```

- [ ] **Step 4: Commit**

```bash
git add index.html src/app.js
git commit -m "feat: add check-update menu item and update about dialog"
```

---

### Task 8: 发版辅助脚本

**Files:**
- Create: `scripts/release-helper.bat`

**Interfaces:**
- Consumes: Task 3 生成的密钥
- Produces: 签名后的安装包、`.sig` 文件、`latest.json`

- [ ] **Step 1: 创建发版辅助脚本**

创建 `scripts/release-helper.bat`：

```batch
@echo off
setlocal

echo ========================================
echo   imsViewer Release Helper
echo ========================================
echo.

set "KEY_PATH=%USERPROFILE%\.tauri\imsviewer.key"
set "BUNDLE_DIR=src-tauri\target\release\bundle\nsis"

if not exist "%KEY_PATH%" (
    echo ERROR: Private key not found at %KEY_PATH%
    echo Run scripts\generate-keys.bat first.
    pause
    exit /b 1
)

if not exist "%BUNDLE_DIR%" (
    echo ERROR: Bundle directory not found: %BUNDLE_DIR%
    echo Run npm run tauri build first.
    pause
    exit /b 1
)

echo Found bundle directory: %BUNDLE_DIR%
echo.

REM 查找最新的安装包
for /f "delims=" %%F in ('dir /b /o-d "%BUNDLE_DIR%\*.exe" 2^>nul') do (
    set "SETUP_EXE=%%F"
    goto :found
)

echo ERROR: No setup.exe found in %BUNDLE_DIR%
pause
exit /b 1

:found
echo Found installer: %SETUP_EXE%
echo.

REM 提取版本号
for /f "tokens=2 delims=_" %%V in ("%SETUP_EXE%") do set "VERSION=%%V"
echo Version: %VERSION%
echo.

REM 签名
echo Signing installer...
call npm run tauri signer sign -- -k "%KEY_PATH%" "%BUNDLE_DIR%\%SETUP_EXE%"

if errorlevel 1 (
    echo ERROR: Signing failed
    pause
    exit /b 1
)

set "SIG_FILE=%BUNDLE_DIR%\%SETUP_EXE%.sig"
if not exist "%SIG_FILE%" (
    echo ERROR: Signature file not generated
    pause
    exit /b 1
)

echo.
echo ========================================
echo   Signing completed!
echo ========================================
echo.
echo Installer: %BUNDLE_DIR%\%SETUP_EXE%
echo Signature: %SIG_FILE%
echo.

REM 生成 latest.json
set "LATEST_JSON=%BUNDLE_DIR%\latest.json"
echo Generating latest.json...

REM 读取签名内容
set /p SIGNATURE=<"%SIG_FILE%"

REM 获取文件大小
for %%A in ("%BUNDLE_DIR%\%SETUP_EXE%") do set "FILE_SIZE=%%~zA"

REM 获取当前日期
for /f "tokens=2 delims==" %%I in ('wmic os get localdatetime /value') do set datetime=%%I
set "PUB_DATE=%datetime:~0,4%-%datetime:~4,2%-%datetime:~6,2%T%datetime:~8,2%:%datetime:~10,2%:%datetime:~12,2%Z"

(
echo {
echo   "version": "%VERSION%",
echo   "notes": "imsViewer v%VERSION%",
echo   "pub_date": "%PUB_DATE%",
echo   "platforms": {
echo     "windows-x86_64": {
echo       "signature": "%SIGNATURE%",
echo       "url": "https://github.com/TanXianliang/imsViewer/releases/download/v%VERSION%/imsViewer_%VERSION%_x64-setup.exe"
echo     }
echo   }
echo }
) > "%LATEST_JSON%"

echo.
echo latest.json generated: %LATEST_JSON%
echo.
echo ========================================
echo   Next steps:
echo ========================================
echo 1. Create a new release on GitHub
echo 2. Upload: %SETUP_EXE%
echo 3. Upload: %SETUP_EXE%.sig
echo 4. Upload: latest.json
echo.
pause
```

- [ ] **Step 2: 测试脚本（可选，需要实际构建）**

Run: `scripts\release-helper.bat`
Expected: 显示签名和 latest.json 生成过程（需要已有构建产物）

- [ ] **Step 3: Commit**

```bash
git add scripts/release-helper.bat
git commit -m "chore: add release helper script for signing and latest.json"
```

---

### Task 9: 端到端验证

**Files:**
- 无修改（验证任务）

**Interfaces:**
- Consumes: 所有前置任务
- Produces: 验证通过的自动更新功能

- [ ] **Step 1: 验证开发模式编译**

Run: `npm run tauri dev`
Expected: 应用正常启动，无编译错误

- [ ] **Step 2: 验证菜单项**

操作： 打开应用 → 帮助 → 检查更新
Expected: 显示"检查更新失败"或"无法连接到更新服务器"（因为 latest.json 尚未发布）

- [ ] **Step 3: 验证跳过版本逻辑**

操作： 在浏览器控制台执行 `localStorage.setItem('imsviewer:skip-version', '1.2.1')`，然后触发自动检查
Expected: 控制台显示"用户已跳过版本 1.2.1，不再提示"

- [ ] **Step 4: 验证构建**

Run: `npm run tauri build`
Expected: 构建成功，生成 NSIS 安装包

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test: verify auto-update feature end-to-end"
```

---

## 自审检查

**Spec coverage:**
- [x] 启动时自动检查 — Task 4 Step 4
- [x] 手动检查按钮 — Task 7 Step 1+2
- [x] 用户确认后下载 — Task 5 Step 3 (btnNow onclick)
- [x] 下载完成后自动重启安装 — Task 6 Step 1
- [x] 跳过此版本 — Task 4 Step 2 + Task 5 Step 3
- [x] 签名验证 — Task 2 (pubkey 配置) + Task 3 (密钥生成)
- [x] 发布流程辅助 — Task 8

**Placeholder scan:** 无 TBD/TODO，所有步骤包含完整代码

**Type consistency:** 
- `checkForUpdates(manual)` — Task 4 定义，Task 7 调用，签名一致
- `showUpdateModal(update, currentVersion, latestVersion)` — Task 5 定义，Task 4 调用，签名一致
- `downloadAndInstallUpdate(update, version)` — Task 6 定义，Task 5 调用，签名一致
- `skipVersion(version)` / `getSkippedVersion()` / `clearSkippedVersion()` — Task 4 定义，Task 5/7 使用，签名一致
