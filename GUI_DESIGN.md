# IMS Viewer — GUI 设计方案

## 1. 框架选型：Tauri v2

### 1.1 为什么选 Tauri

经过 PySide6 / Electron / Tauri 三方对比，最终选择 **Tauri v2**：

| 维度 | PySide6 | Electron | **Tauri** |
|------|---------|----------|-----------|
| 打包 .exe | ~70MB | ~150MB | **~10MB** |
| 内存占用 | ~80MB | ~200MB | **~50MB** |
| 启动速度 | 快 | 慢 | **快** |
| UI 表现力 | ★★★☆☆ | ★★★★★ | ★★★★★ |
| 图片显示 | QGraphicsView | Canvas API | **Canvas API** |
| 暗色主题 | QSS | CSS | **CSS** |
| 开发语言 | Python | JS+Node | **Rust+JS** |
| IMS 解析 | 已有 Python | 需 Port | 需 Port（Rust 天然擅长二进制） |
| 社区热度 | ★★★★★ | ★★★★★ | ★★★★☆ |

**决策依据**：Tauri 在体积/性能/UI 表现力三个维度同时最优。唯一代价是用 Rust port IMS 解析（~100 行），但 Rust 处理二进制格式是其长板，不是负担。

### 1.2 两语言分工

```
┌──────────────────────────────────────────────┐
│              Tauri Shell (Rust)              │
│  ┌────────────────────────────────────────┐  │
│  │   Frontend (HTML + CSS + Vanilla JS)   │  │
│  │   - 三栏布局 (CSS Grid)                │  │
│  │   - Canvas 图片显示/缩放/平移           │  │
│  │   - 缩略图列表                          │  │
│  │   - 暗色主题 (CSS Variables)           │  │
│  │   - 键盘/鼠标事件                      │  │
│  └────────────────┬───────────────────────┘  │
│                   │ invoke() IPC              │
│  ┌────────────────▼───────────────────────┐  │
│  │   Rust Backend                         │  │
│  │   - IMS 二进制解析 (byteorder crate)    │  │
│  │   - 文件夹扫描 (std::fs)                │  │
│  │   - 图片数据切片 (零拷贝)               │  │
│  │   - 原生文件对话框 (tauri-plugin-dialog) │  │
│  │   - 图片导出                            │  │
│  └────────────────────────────────────────┘  │
└──────────────────────────────────────────────┘
```

---

## 2. 核心交互模型：双层导航

```
文件夹层 (← → 左右键切换)
─────────────────────────────────────────────────────
  ab.jpg    ab.ims.jpg    xxx.png    xxx.ims.png   ...
    │           │            │           │
    │        IMS 子图层      │        IMS 子图层
    │        (↑ ↓ 上下键)    │        (↑ ↓ 上下键)
    │           │            │           │
    │      ┌────┼────┐       │      ┌────┼────┐
    │     子图0 子图1 子图2   │     子图0 子图1 子图2
    │     (封面)             │     (封面)
```

- **← →**：切换到文件夹中上一个/下一个图片文件（IMS 或普通图片）
- **↑ ↓**：IMS 文件时切换内部子图；普通图片时无操作

### 2.1 文件类型自适应

| 当前文件类型 | 左侧栏 | ↑↓ 键 | 右侧栏 |
|-------------|--------|-------|--------|
| `.ims.jpg` / `.ims.png` | 全部子图缩略图 | 切换子图 | IMS 文件信息 + 当前子图信息 |
| `.jpg` / `.png` (普通) | 单张缩略图 | 无操作 | 当前文件信息 |

---

## 3. 界面布局

```
┌──────────────────────────────────────────────────────┐
│  标题栏: imsViewer - xxx.ims.jpg                      │
├──────────┬────────────────────────┬──────────────────┤
│          │                        │                  │
│ 左侧栏    │      中央显示区          │    右侧栏         │
│          │                        │                  │
│ ◀ 3/10 ▶│                        │  ┌──────────────┐ │
│ ┌──────┐ │                        │  │ 当前文件信息   │ │
│ │缩略图1│ │    Canvas 图片显示       │  │ 文件名/类型   │ │
│ │⭐封面 │ │    - 自适应缩放          │  │ 大小/版本     │ │
│ └──────┘ │    - Ctrl+滚轮缩放       │  │ 图片总数      │ │
│ ┌──────┐ │    - 拖拽平移            │  └──────────────┘ │
│ │缩略图2│ │                        │                  │
│ └──────┘ │                        │  ┌──────────────┐ │
│ ┌──────┐ │                        │  │ 当前图片信息   │ │
│ │缩略图3│ │                        │  │ 文件名/格式   │ │
│ └──────┘ │                        │  │ 分辨率/大小   │ │
│          │                        │  │ 偏移          │ │
│          │                        │  └──────────────┘ │
├──────────┴────────────────────────┴──────────────────┤
│  状态栏: 文件 3/10 | ab.ims.jpg 子图 2/4 | 100%      │
└──────────────────────────────────────────────────────┘
```

### 3.1 三栏比例

- 左侧栏：~220px (`min-width: 160px`，CSS Grid `1fr 4fr 1.2fr`)
- 中央区：弹性填充
- 右侧栏：~260px (`min-width: 200px`)

---

## 4. 组件设计

### 4.1 前端组件 (HTML/CSS/JS)

```
index.html
├── #menu-bar        — 顶部菜单栏
│   ├── 文件: 打开 / 导出当前 / 导出全部
│   ├── 视图: 适应 / 100% / 放大 / 缩小
│   └── 编辑: 进入/退出编辑模式 (打开 IMS 文件后显示)
│
├── #main-container  — CSS Grid 三栏
│   ├── #left-panel
│   │   ├── #folder-bar       — ◀ 文件名(3/10) ▶
│   │   └── #thumbnail-list   — 子图缩略图 (CSS Grid 2列)
│   │       └── .thumb-item   — 缩略图卡片 (Canvas 生成)
│   │           封面项加 .cover-badge 星标
│   │
│   ├── #center-panel
│   │   ├── #edit-mode-bar    — 编辑模式橙色提示条
│   │   └── #image-canvas     — <canvas> 显示当前图片
│   │
│   └── #right-panel
│       ├── #file-info        — 当前文件元信息
│       └── #image-info       — 当前子图信息
│
├── #status-bar               — 状态栏
│
├── #batch-modal              — 批量转换模态框
├── #upgrade-modal            — 批量升级模态框
└── #thumb-context-menu       — 缩略图右键菜单 (编辑模式)
```

### 4.2 Rust 后端 (Tauri Commands)

```rust
// 暴露给前端的 10 个命令
#[tauri::command] fn open_file(path: String) -> Result<FilePayload, String>
#[tauri::command] fn pick_file(app: AppHandle) -> Result<Option<String>, String>
#[tauri::command] fn pick_folder(app: AppHandle) -> Result<Option<String>, String>
#[tauri::command] fn get_sub_image(path: String, index: usize) -> Result<SubImagePayload, String>
#[tauri::command] fn extract_image(app: AppHandle, path: String, index: usize) -> Result<String, String>
#[tauri::command] fn extract_all(app: AppHandle, path: String) -> Result<Vec<String>, String>
#[tauri::command] fn start_batch_convert(app: AppHandle, config: BatchConfig) -> Result<String, String>
#[tauri::command] fn start_batch_upgrade(app: AppHandle, path: String, depth: u32, multi_threaded: bool) -> Result<String, String>
#[tauri::command] fn register_file_associations() -> Result<String, String>
#[tauri::command] fn get_launch_file(state: State<LaunchFile>) -> Result<Option<String>, String>
```

### 4.3 核心数据流

```
用户打开文件 (Ctrl+O)
  → Rust: tauri_plugin_dialog 弹出原生对话框
  → Rust: scan_folder(dir) → Vec<FolderEntry>
  → Rust: parse_ims(path) → ImsFileInfo
  → Rust: 返回 FilePayload { folder_list, ims_info, sub_images[0].bytes }
  → JS: 更新 thumbnail-list、canvas、info-panel、status-bar

用户按 →
  → JS: current_file_idx += 1
  → JS: invoke('open_file', { path: folder_list[idx] })
  → Rust → JS: 新文件数据
  → JS: 刷新全部 UI

用户按 ↓ (IMS 模式)
  → JS: current_sub_idx += 1
  → JS: invoke('get_sub_image', { path, index: sub_idx })
  → Rust → JS: ImagePayload { bytes, format }
  → JS: canvas 重绘 + thumbnail-list 高亮 + info-panel 刷新
```

---

## 5. 图片显示方案

使用 `<canvas>` + 自建简易变换矩阵：

```javascript
class ImageViewer {
    constructor(canvas) {
        this.ctx = canvas.getContext('2d');
        this.scale = 1.0;
        this.offsetX = 0;
        this.offsetY = 0;
    }

    loadImage(bytes, format) {
        const blob = new Blob([bytes], { type: format === 0 ? 'image/jpeg' : 'image/png' });
        const url = URL.createObjectURL(blob);
        this.img = new Image();
        this.img.onload = () => { this.fitToWindow(); this.draw(); };
        this.img.src = url;
    }

    draw() {
        this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
        this.ctx.save();
        this.ctx.translate(this.offsetX, this.offsetY);
        this.ctx.scale(this.scale, this.scale);
        this.ctx.drawImage(this.img, 0, 0);
        this.ctx.restore();
    }

    fitToWindow() {
        const sx = this.canvas.width / this.img.width;
        const sy = this.canvas.height / this.img.height;
        this.scale = Math.min(sx, sy);
        this.offsetX = (this.canvas.width - this.img.width * this.scale) / 2;
        this.offsetY = (this.canvas.height - this.img.height * this.scale) / 2;
    }
}
```

缩略图生成同样用 Canvas：
```javascript
function createThumbnail(bytes, format, w, h) {
    // 创建离屏 canvas → drawImage → 设置到缩略图 <img> 的 src
    const offscreen = document.createElement('canvas');
    offscreen.width = w; offscreen.height = h;
    const ctx = offscreen.getContext('2d');
    // ... draw scaled image
    return offscreen.toDataURL();
}
```

---

## 6. Rust IMS 解析

### 6.1 文件结构

```
src-tauri/src/
├── main.rs          # Tauri 入口
├── lib.rs           # 模块导出 + Tauri 配置
└── ims.rs           # IMS 格式解析
```

### 6.2 核心类型

```rust
// ims.rs
use serde::Serialize;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

pub const MAGIC: &[u8; 8] = b"IMSIMAGE";

#[derive(Debug, Clone, Serialize)]
pub struct ImsImage {
    pub index: usize,
    pub filename: String,
    pub format: u8,       // 0=JPEG, 1=PNG, 2=WebP, 3=AVIF
    pub data_offset: u32,
    pub data_length: u32,
    pub is_cover: bool,
    /// SHA-256 内容哈希 (hex, v2+)
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImsFileInfo {
    pub filepath: String,
    pub file_size: u64,
    pub version: u16,
    pub image_count: u32,
    pub data_start: u32,
    pub index_offset: u32,
    pub images: Vec<ImsImage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderEntry {
    pub filepath: String,
    pub filename: String,
    pub is_ims: bool,
    pub file_size: u64,
}
```

### 6.3 解析函数

```rust
pub fn parse_ims(filepath: &str) -> Result<ImsFileInfo, String> {
    let data = std::fs::read(filepath).map_err(|e| e.to_string())?;
    let file_size = data.len() as u64;
    
    // 末尾 4 字节 → index_offset
    let index_offset = {
        let mut cursor = Cursor::new(&data[file_size as usize - 4..]);
        cursor.read_u32::<LittleEndian>().unwrap()
    };
    
    let mut cursor = Cursor::new(&data[index_offset as usize..]);
    
    // 验证 magic
    let mut magic_buf = [0u8; 8];
    cursor.read_exact(&mut magic_buf).map_err(|e| e.to_string())?;
    if &magic_buf != MAGIC {
        return Err("无效的 IMS 文件: magic 不匹配".into());
    }
    
    // 读取 header
    let version = cursor.read_u16::<LittleEndian>().unwrap();
    let image_count = cursor.read_u32::<LittleEndian>().unwrap();
    let data_start = cursor.read_u32::<LittleEndian>().unwrap();
    
    // 遍历 entries
    let mut images = Vec::with_capacity(image_count as usize);
    for i in 0..image_count as usize {
        let name_len = cursor.read_u16::<LittleEndian>().unwrap() as usize;
        let mut name_buf = vec![0u8; name_len];
        cursor.read_exact(&mut name_buf).map_err(|e| e.to_string())?;
        let filename = String::from_utf8_lossy(&name_buf).to_string();
        
        let format = cursor.read_u8().unwrap();
        let data_offset = cursor.read_u32::<LittleEndian>().unwrap();
        let data_length = cursor.read_u32::<LittleEndian>().unwrap();
        let is_cover = cursor.read_u8().unwrap() != 0;
        
        images.push(ImsImage {
            index: i,
            filename,
            format,
            data_offset,
            data_length,
            is_cover,
        });
    }
    
    Ok(ImsFileInfo {
        filepath: filepath.into(),
        file_size,
        version,
        image_count,
        data_start,
        index_offset,
        images,
    })
}
```

---

## 7. 快捷键设计

### 7.1 双层导航

| 快捷键 | 层级 | 功能 |
|--------|------|------|
| `←` | 文件夹层 | 上一个文件（编辑模式下锁定） |
| `→` | 文件夹层 | 下一个文件（编辑模式下锁定） |
| `Home` | 文件夹层 | 跳到第一个文件（编辑模式下锁定） |
| `End` | 文件夹层 | 跳到最后一个文件（编辑模式下锁定） |
| `↑` | IMS 子图层 | 上一张子图 |
| `↓` | IMS 子图层 | 下一张子图 |

### 7.2 全局快捷键

| 快捷键 | 功能 |
|--------|------|
| `Home` / `End` | 跳到文件夹第一个/最后一个文件 |
| `Ctrl+O` | 打开文件（原生对话框） |
| `Ctrl+E` | 导出当前图片 |
| `Ctrl+Shift+E` | 导出当前 IMS 全部子图 |
| `+` / `-` | 放大 / 缩小 |
| `0` | 适应窗口 |
| `1` | 原始大小 (100%) |

### 7.3 编辑模式快捷键

| 快捷键 | 功能 |
|--------|------|
| `Esc` | 退出编辑模式 |
| `Del` | 删除选中的子图（封面除外） |

### 7.4 鼠标交互

| 操作 | 功能 |
|------|------|
| 点击缩略图 | 切换子图（编辑模式下同时更新选择） |
| `Ctrl+点击` 缩略图 | 多选 — 切换选中状态 |
| `Shift+点击` 缩略图 | 多选 — 范围选中 |
| 右键缩略图 | 编辑模式：弹出上下文菜单（插入/替换/删除） |
| 滚轮 | 切换子图 |
| Shift+滚轮 | 切换文件（编辑模式下锁定） |
| Ctrl+滚轮 | 缩放图片 |
| 拖拽画布 | 平移（任意缩放级别均可） |
| 拖拽缩略图 | 编辑模式：排序（封面锁定不可拖动） |
| 拖放文件到窗口 | 浏览模式：打开文件；编辑模式：插入图片到末尾 |
| 双击中央区 | 适应窗口 ↔ 100% |

---

## 8. 界面视觉风格

暗色主题（CSS Variables）：

```css
:root {
    --bg-primary:    #2d2d2d;
    --bg-secondary:  #252525;
    --bg-tertiary:   #1e1e1e;
    --text-primary:  #e0e0e0;
    --text-secondary:#a0a0a0;
    --accent:        #0078d4;
    --accent-hover:  #1a8ad4;
    --border:        #3d3d3d;
    --cover-star:    #ffd700;
}
```

---

## 9. 实现计划

### Phase 1: 项目骨架 + Rust 后端（~200 行 Rust）✅
- [x] `cargo init` + Tauri 配置
- [x] `ims.rs` — IMS 解析 + 文件夹扫描
- [x] `lib.rs` — Tauri commands (open_file, scan_folder, get_sub_image)
- [x] `package.json` + Vite 配置

### Phase 2: 前端布局 + 图片显示（~250 行 JS）✅
- [x] `index.html` — 三栏 CSS Grid
- [x] `styles.css` — 暗色主题
- [x] `app.js` — ImageViewer 类 (Canvas 加载/缩放/平移)
- [x] 缩略图生成函数

### Phase 3: 双层导航 + 键盘交互（~200 行 JS）✅
- [x] ← → 文件切换
- [x] ↑ ↓ 子图切换
- [x] 缩略图点击切换
- [x] 全局快捷键
- [x] FolderBar 导航条

### Phase 4: 信息面板 + 导出（~150 行）✅
- [x] 右侧文件/图片信息动态更新
- [x] 导出当前图片
- [x] 导出全部子图
- [x] 状态栏

### Phase 5: 打包 + 打磨（~100 行）✅
- [x] `cargo tauri build` 生成 .exe
- [x] 窗口标题更新
- [x] 拖放文件打开
- [x] 关于对话框

**预估总代码量**：~500 行 Rust + ~400 行 JS + ~150 行 HTML/CSS

---

## 10. 打包为 .exe

Tauri 原生编译为平台可执行文件，无需第三方打包工具：

```bash
npm run tauri build
```

产物：

```
src-tauri/target/release/bundle/
├── msi/imsViewer_1.0.0_x64.msi    # Windows 安装包
├── nsis/imsViewer_1.0.0_x64.exe   # NSIS 安装程序
└── imsViewer.exe                   # 独立可执行文件 (~10MB)
```

| 组成部分 | 大小 |
|----------|------|
| Rust 编译产物（IMS 解析 + Tauri 框架） | ~4MB |
| WebView2 运行时 | 系统自带，0 |
| HTML/CSS/JS 资源 | ~50KB |
| **合计** | **~5-10MB** |

> WebView2 在 Windows 10+ 上已预装，用户无需额外安装任何运行时。

---

## 11. 项目文件清单

```
imsViewer/
├── DESIGN.md              # IMS 格式 + CLI 工具设计
├── GUI_DESIGN.md           # 本文件 — GUI 设计方案
│
├── ims_pack.py             # CLI 打包工具 (不变)
├── ims_unpack.py           # CLI 解包工具 (不变)
├── ims_batch.py            # CLI 批量打包 (不变)
├── verify_ims.py           # 格式验证 (不变)
├── requirements.txt        # Python 依赖
│
├── src/                    # 🆕 前端
│   ├── index.html
│   ├── styles.css
│   └── app.js
│
├── src-tauri/              # 🆕 Rust 后端
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── build.rs
│   ├── capabilities/
│   │   └── default.json
│   ├── icons/
│   └── src/
│       ├── main.rs
│       ├── lib.rs
│       ├── ims.rs
│       └── ims_pack.rs        # 批量转换 + IMS 打包
│
├── package.json            # 🆕 npm 依赖 (tauri-cli)
├── vite.config.js          # 🆕 Vite 构建配置
└── index.html              # 🆕 入口 HTML
```

---

## 12. 依赖

### Python (CLI 工具，不变)
```
Pillow>=9.0
```

### Node.js (前端 + Tauri CLI)
```json
{
  "devDependencies": {
    "@tauri-apps/cli": "^2.0",
    "vite": "^5.0"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.0",
    "@tauri-apps/plugin-dialog": "^2.0"
  }
}
```

### Rust (后端)
```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-dialog = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
byteorder = "1"
base64 = "0.22"
walkdir = "2"
rayon = "1"
regex = "1"
```

---

## 13. 批量转换工具

### 13.1 入口

菜单栏 **工具 → 批量转换** 打开模态框。

### 13.2 功能

| 功能 | 说明 |
|------|------|
| 多文件夹 | 动态添加/删除，每行可设遍历深度 (0-100，默认0=无限) |
| 输出目录 | 留空=原地转换；填写后默认镜像输入目录结构 |
| 保持目录结构 | 勾选后输出到 `Output/输入文件夹名/子路径/`；不勾选则平铺 |
| 自动跳过 IMS | 扫描时跳过 stem 含 `.ims` 的已打包文件 |
| 删除源文件 | 勾选后转换成功即删除原始图片 |
| 多线程 | 勾选后用 rayon 并行转换 |
| 实时进度 | 进度条 + `[当前/总数] 文件名` 文字 |
| 格式支持 | JPEG / PNG / WebP / AVIF（WebP/AVIF 自动转 PNG） |

### 13.3 深度值含义

| 深度 | 行为 |
|------|------|
| 0 | 无限遍历所有子目录 |
| 1 | 仅当前目录 |
| N | 遍历 N 层 |

### 13.4 镜像输出示例

```
输出目录: Output，输入目录: Input0, Input1

勾选"保持目录结构"时:
  Input1/tex0/image0.jpg + image0_00001_.jpg + image0_00002_.jpg
    → Output/Input1/tex0/image0.ims.jpg

不勾选时:
  Input1/tex0/image0.jpg + image0_00001_.jpg + image0_00002_.jpg
    → Output/image0.ims.jpg
```

### 13.5 文件分组规则

- 封面: `{base}.{ext}`
- 子图: `{base}_XXXXX_.{ext}`（5 位数字序号）
- 输出: `{base}.ims.{cover_ext}`

### 13.6 IMS v2 — 内容哈希

自 v2 起，每个条目追加 SHA-256 内容哈希（33 字节）：

| 字段 | 大小 | 说明 |
|------|------|------|
| hash_algorithm | 1B | `1` = SHA-256 |
| hash | 32B | SHA-256 原始字节 |

- 打包时自动计算
- 前端信息面板显示（前 20 字符 + 悬浮完整）
- 解析器向后兼容 v1（无 hash 字段）

### 13.7 架构

```
前端模态框                    Rust 后端                      文件系统
─────────────────────────────────────────────────────────────────
invoke('start_batch_convert', config)
                              ↓
                              scan_and_group()  × N 文件夹    → walkdir 遍历
                              ↓
                              pack_ims()        × M 组       → 写入 IMS 文件
                              ↓ 每完成一个
                              emit("batch-progress")          ← 进度
listen("batch-progress")  ←
更新进度条
```

---

## 14. 批量升级工具

### 14.1 入口

菜单栏 **工具 → 批量升级 IMS** 打开升级模态框。

### 14.2 功能

将旧版 IMS 文件（v1）就地升级到 v2，追加 SHA-256 内容哈希。

| 功能 | 说明 |
|------|------|
| 目录扫描 | 选择目录递归扫描所有 IMS 文件 |
| 深度控制 | 同批量转换，0=无限遍历 |
| 版本检测 | 自动跳过已是 v2+ 的文件 |
| 多线程 | 可选 rayon 并行升级 |
| 实时进度 | 进度条 + 文件名反馈 |

### 14.3 升级流程

1. 解析旧 IMS，读取全部子图数据
2. 以 v2 格式重新写入（封面排第一，每张子图追加 SHA-256 hash）
3. 写入临时文件后替换原文件（保证原子性）

---

## 15. 右键菜单注册

### 15.1 入口

菜单栏 **工具 → 注册右键菜单**，或通过安装包自动注册。

### 15.2 注册的扩展名

| 组 | 扩展名 |
|----|--------|
| 普通图片 | `.jpg` `.jpeg` `.png` `.webp` `.avif` |
| IMS 容器 | `.ims.jpg` `.ims.png` |

### 15.3 实现

通过 `reg add` 命令在 `HKCU\Software\Classes\SystemFileAssociations` 下为每个扩展名注册 shell 命令。右键图片文件即出现"使用 imsViewer 打开"菜单项。

### 15.4 启动文件

`get_launch_file` 命令读取启动时传入的文件路径（来自右键"打开方式"或命令行参数），前端初始化后自动打开该文件。

---

## 16. IMS 按需加载策略

### 16.1 问题

打开 IMS 文件时一次性读取全部子图 + base64 编码 → 大文件非常慢。

### 16.2 优化

| 阶段 | 行为 |
|------|------|
| 打开 IMS | 只加载封面图片数据，其余子图仅返回元数据 |
| 后台预加载 | 逐个异步加载剩余子图，加载一张更新一张缩略图 |
| 切换子图 | 已缓存 → 直接显示；未缓存 → 即时调用 `get_sub_image` |
| 切换文件 | 取消旧的后台任务，开始新的预加载 |
| 加载占位 | 未加载的缩略图显示流光动画

---

## 17. 编辑模式（实验性）

基于 [EDIT_MODE_DESIGN.md](EDIT_MODE_DESIGN.md) 设计方案实现，已完成 Phase 1-3。

### 17.1 实现状态

| Phase | 内容 | 状态 |
|-------|------|------|
| Phase 1 | 基础编辑框架（菜单、切换、导航锁、脏状态） | 已完成 |
| Phase 2 | 核心编辑操作（多选、删除、替换、封面保护、右键菜单） | 已完成 |
| Phase 3 | 插入 + 排序（文件对话框、拖放、拖拽排序、IMS 提取） | 已完成 |
| Phase 4 | Undo/Redo（命令模式、Ctrl+Z/Y、栈深度限制 30 步） | ✅ 已完成 |
| Phase 5 | 保存（save_ims Rust 命令、原子写入、Ctrl+S） | ✅ 已完成 |

### 17.2 入口与状态管理

- **进入方式**: 打开 IMS 文件后菜单栏显示 `编辑` 菜单，点击进入
- **退出方式**: 再次点击菜单或按 `Esc`
- **导航锁定**: 进入编辑模式后 `←` `→` / `Shift+滚轮` 文件导航被锁定。`↑` `↓` 子图切换保持可用
- **脏状态**: 任何修改（删除/插入/替换/排序）标记 `state.dirty = true`。退出时若有未保存修改弹出 `confirm` 确认提示
- **编辑指示**: 中栏顶部显示橙色 `✏ 编辑模式` 提示条（`#edit-mode-bar`）

### 17.3 多选交互

- `Ctrl+点击` 缩略图：切换该项的选中状态（toggle）
- `Shift+点击` 缩略图：选中从已选最小索引到当前索引的范围
- 普通点击：清除多选，单选当前项
- 选中项显示蓝色高亮边框 + 发光阴影（`.thumb-item.selected`）
- 多选状态存储在 `state.selectedIndices`（Set 类型）

### 17.4 上下文菜单

右键点击缩略图弹出固定定位菜单（`#thumb-context-menu`），提供三个操作：

| 菜单项 | 行为 |
|--------|------|
| 插入图片... | 打开文件对话框，插入到选中项之后（或列表末尾）。拖入 IMS 文件时提取全部子图 |
| 替换图片... | 打开文件对话框，替换当前唯一选中的子图。封面被替换后保持 `isCover = true` |
| 删除选中 | 删除所有选中的子图（封面自动跳过，不可删除） |

### 17.5 拖拽排序

- **触发**: 在缩略图上 `mousedown` 并移动超过 5px 阈值
- **视觉**: 原位置变半透明（`.dragging`），创建固定定位的克隆节点跟随鼠标（带旋转 + 缩放效果）
- **插入指示**: 鼠标悬停位置的缩略图显示绿色边框（`border-top` 或 `border-bottom`，3px solid #2ecc71）
- **封面锁定**: 索引 0 的封面不可拖动
- **完成**: `mouseup` 时执行数组 `splice` 移动，重建缩略图列表

### 17.6 拖放插入

- 复用 Tauri 原生拖放事件（`onDragDropEvent`）
- 编辑模式下的 `drop` 调用 `insertImageFiles(paths)` 而非 `openFile`
- 支持普通图片文件和 IMS 文件
- 拖入 IMS 文件时自动提取**全部子图**（通过 `get_sub_image` 加载延迟数据），全部标记 `isCover = false`

### 17.7 删除与封面保护

- `Del` 键调用 `deleteSelectedImages()`
- 自动过滤 `index === 0`（封面）
- 从后往前删除避免索引错乱
- 删除后更新当前显示索引（若超出范围则定位到最后一张）

### 17.8 实现说明

- 所有编辑逻辑位于 `src/app.js`（未拆分独立 `edit.js`），约 300 行编辑相关代码
- 编辑状态集中管理在 `state` 对象：`editMode`、`dirty`、`selectedIndices`（Set）、`dragState`
- 新增 Rust 命令：`save_ims`（原子写入，WebP/AVIF 自动转 PNG，仅剩封面时存为普通图片）
