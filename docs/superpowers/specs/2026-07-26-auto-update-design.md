# imsViewer 自动更新功能设计

## 概述

为 imsViewer 添加自动更新功能：应用启动时自动检查 GitHub Release 最新版本，用户确认后自动下载并安装更新。支持"跳过此版本"功能。

## 需求确认

| 需求项 | 决策 |
|--------|------|
| 发布物形式 | Tauri bundle 生成的 NSIS/MSI 安装包 |
| 检查时机 | 启动时自动检查 + 手动检查按钮 |
| 下载方式 | 用户确认后下载，下载完成后自动重启安装 |
| 跳过版本 | 支持"跳过此版本"，跳过版本不再自动提示 |
| 发版流程 | 本地手动构建 + 手动上传 GitHub Release |

## 技术方案

使用 **Tauri 官方 Updater 插件** (`tauri-plugin-updater`)，原因：
- 与 Tauri bundle (NSIS/MSI) 完美兼容
- 官方维护，处理 Windows 安装/重启边缘情况
- 内置签名验证，安全性高
- 支持差分更新（未来可扩展）

## 架构

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│   前端 (JS)      │────▶│  Tauri Updater   │────▶│  GitHub Release │
│  - 检查更新按钮   │     │  Plugin (Rust)   │     │  - latest.json  │
│  - 更新提示弹窗   │◄────│  - 版本检查       │◄────│  - 安装包 + .sig │
│  - 下载进度显示   │     │  - 下载/验证/安装  │     │                 │
│  - 跳过版本管理   │     │                  │     │                 │
└─────────────────┘     └──────────────────┘     └─────────────────┘
```

## 核心组件

### 1. 依赖与配置

**前端依赖 (`package.json`)：**
```json
{
  "dependencies": {
    "@tauri-apps/plugin-updater": "^2.0",
    "@tauri-apps/plugin-process": "^2.0"
  }
}
```

**后端依赖 (`src-tauri/Cargo.toml`)：**
```toml
[dependencies]
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
```

**Tauri 配置 (`src-tauri/tauri.conf.json`)：**
```json
{
  "plugins": {
    "updater": {
      "active": true,
      "endpoints": [
        "https://github.com/TanXianliang/imsViewer/releases/latest/download/latest.json"
      ],
      "dialog": false,
      "pubkey": "<硬编码的公钥内容>"
    }
  }
}
```

### 2. 更新检查流程

**启动自动检查：**
- App 启动后延迟 3 秒执行，避免阻塞启动
- 静默检查，失败不打扰用户
- 发现新版本且未被跳过时，弹出更新提示

**手动检查：**
- 菜单栏 "帮助" → "检查更新"
- 无论是否跳过，都正常提示检查结果
- 无更新时显示"当前已是最新版本"

**版本比较逻辑：**
```javascript
// 语义化版本比较
function compareVersions(current, latest) {
  const c = current.split('.').map(Number);
  const l = latest.split('.').map(Number);
  for (let i = 0; i < 3; i++) {
    if (l[i] > c[i]) return 1;   // 有更新
    if (l[i] < c[i]) return -1;  // 当前更新
  }
  return 0; // 相同
}
```

### 3. 跳过此版本功能

**存储：** `localStorage` 键 `imsviewer:skip-version`

**行为：**
- 点击"跳过此版本" → 记录版本号到 localStorage
- 启动检查时：`if (latestVersion === skippedVersion) → 静默忽略`
- 手动检查时：忽略跳过列表，正常提示
- 发布更新版本后（如跳过 1.3.0 后发布 1.3.1）→ 恢复自动提示

**清除跳过：**
- 在"关于"对话框中显示当前跳过的版本（如有）
- 提供"清除跳过记录"按钮（可选，低优先级）

### 4. 用户交互流程

**更新提示弹窗：**
```
┌─────────────────────────────┐
│  发现新版本 v1.3.0            │
│  当前版本 v1.2.1              │
│                             │
│  更新内容:                   │
│  - 新增 xxx 功能             │
│  - 修复 yyy 问题             │
│                             │
│  [跳过此版本] [稍后提醒] [立即更新] │
└─────────────────────────────┘
```

**下载进度弹窗：**
```
┌─────────────────────────────┐
│  正在下载更新 v1.3.0...       │
│                             │
│  ████████████████░░░░  80%   │
│  (92.3 MB / 115.4 MB)        │
│                             │
│  [取消下载]                   │
└─────────────────────────────┘
```

**下载完成弹窗：**
```
┌─────────────────────────────┐
│  更新下载完成                 │
│                             │
│   imsViewer 将重启以完成安装   │
│                             │
│  [立即重启安装]               │
└─────────────────────────────┘
```

### 5. 发布流程（本地手动）

**步骤：**
1. 构建：`npm run tauri build`
2. 签名：`tauri signer sign -k ~/.tauri/imsviewer.key -p <密码> <安装包路径>`
3. 生成 `latest.json`（脚本辅助）
4. 上传 GitHub Release：安装包 + `.sig` + `latest.json`

**`latest.json` 格式：**
```json
{
  "version": "1.2.1",
  "notes": "更新说明",
  "pub_date": "2026-07-26T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<签名内容>",
      "url": "https://github.com/TanXianliang/imsViewer/releases/download/v1.2.1/imsViewer_1.2.1_x64-setup.exe"
    }
  }
}
```

**签名密钥管理：**
- 私钥：`~/.tauri/imsviewer.key`（本地保存，不提交 git）
- 公钥：硬编码到 `tauri.conf.json` 或从 `src-tauri/resources/updater-pubkey.pem` 读取
- `.gitignore` 添加 `*.key` 忽略

### 6. 错误处理

| 场景 | 处理方式 |
|------|---------|
| 网络不可用 | 启动检查静默失败；手动检查提示"网络连接失败" |
| GitHub API 限流/超时 | 启动检查静默失败；手动检查提示"检查更新失败，请稍后重试" |
| 下载中断 | 支持断点续传；用户可取消后重试 |
| 签名验证失败 | 提示"更新包校验失败，请从官网下载"，中止更新 |
| 安装失败 | 提示错误信息，保留当前版本 |
| 无更新 | 手动检查时提示"当前已是最新版本" |

### 7. 需要新增/修改的文件

| 文件 | 操作 | 说明 |
|------|------|------|
| `package.json` | 修改 | 添加 `@tauri-apps/plugin-updater`, `@tauri-apps/plugin-process` |
| `src-tauri/Cargo.toml` | 修改 | 添加 `tauri-plugin-updater`, `tauri-plugin-process` |
| `src-tauri/tauri.conf.json` | 修改 | 配置 updater 插件 |
| `src-tauri/src/lib.rs` | 修改 | 注册 updater 和 process 插件 |
| `src-tauri/capabilities/default.json` | 修改 | 添加 updater 权限 |
| `src/app.js` | 修改 | 添加更新检查逻辑、弹窗交互、跳过版本管理 |
| `src/styles.css` | 修改 | 添加更新弹窗样式 |
| `index.html` | 修改 | 添加更新弹窗 HTML、帮助菜单"检查更新"项 |
| `scripts/release-helper.bat` | 新增 | 发版辅助脚本（签名+生成 latest.json） |
| `.gitignore` | 修改 | 添加 `*.key` 忽略 |

## 安全考虑

- **签名验证：** 所有更新包必须通过公钥验证，防止中间人攻击
- **HTTPS：** 所有网络请求强制 HTTPS
- **私钥保护：** 私钥文件不提交到 git，`.gitignore` 忽略

## 未来扩展

- **差分更新：** Tauri updater 支持差分更新，可显著减少下载体积
- **更新渠道：** 支持 stable/beta 渠道切换
- **自动下载：** 可选的后台静默下载（用户设置开启）
