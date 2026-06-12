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
│   └── 视图: 适应 / 100% / 放大 / 缩小
│
├── #main-container  — CSS Grid 三栏
│   ├── #left-panel
│   │   ├── #folder-bar       — ◀ 文件名(3/10) ▶
│   │   └── #thumbnail-list   — 子图缩略图 (CSS Grid 2列)
│   │       └── .thumb-item   — 缩略图卡片 (Canvas 生成)
│   │           封面项加 .cover-badge 星标
│   │
│   ├── #center-panel
│   │   └── #image-canvas     — <canvas> 显示当前图片
│   │
│   └── #right-panel
│       ├── #file-info        — 当前文件元信息
│       └── #image-info       — 当前子图信息
│
└── #status-bar               — 状态栏
```

### 4.2 Rust 后端 (Tauri Commands)

```rust
// 暴露给前端的 6 个命令
#[tauri::command] fn open_file(path: String) -> Result<FilePayload, String>
#[tauri::command] fn scan_folder(path: String) -> Vec<FolderEntry>
#[tauri::command] fn get_sub_image(path: String, index: usize) -> Result<ImagePayload, String>
#[tauri::command] fn extract_image(path: String, index: usize, out_dir: String) -> Result<String, String>
#[tauri::command] fn extract_all(path: String, out_dir: String) -> Result<Vec<String>, String>
#[tauri::command] fn get_supported_files(dir: String) -> Vec<FolderEntry>
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
    pub format: u8,       // 0=JPEG, 1=PNG
    pub data_offset: u32,
    pub data_length: u32,
    pub is_cover: bool,
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
| `←` | 文件夹层 | 上一个文件 |
| `→` | 文件夹层 | 下一个文件 |
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

### 7.3 鼠标交互

| 操作 | 功能 |
|------|------|
| 点击缩略图 | 切换子图 |
| 滚轮 | 滚动缩略图列表 |
| Ctrl+滚轮 | 缩放中央图片 |
| 拖拽 | 平移（放大后） |
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

### Phase 1: 项目骨架 + Rust 后端（~200 行 Rust）
- [ ] `cargo init` + Tauri 配置
- [ ] `ims.rs` — IMS 解析 + 文件夹扫描
- [ ] `lib.rs` — Tauri commands (open_file, scan_folder, get_sub_image)
- [ ] `package.json` + Vite 配置

### Phase 2: 前端布局 + 图片显示（~250 行 JS）
- [ ] `index.html` — 三栏 CSS Grid
- [ ] `styles.css` — 暗色主题
- [ ] `app.js` — ImageViewer 类 (Canvas 加载/缩放/平移)
- [ ] 缩略图生成函数

### Phase 3: 双层导航 + 键盘交互（~200 行 JS）
- [ ] ← → 文件切换
- [ ] ↑ ↓ 子图切换
- [ ] 缩略图点击切换
- [ ] 全局快捷键
- [ ] FolderBar 导航条

### Phase 4: 信息面板 + 导出（~150 行）
- [ ] 右侧文件/图片信息动态更新
- [ ] 导出当前图片
- [ ] 导出全部子图
- [ ] 状态栏

### Phase 5: 打包 + 打磨（~100 行）
- [ ] `cargo tauri build` 生成 .exe
- [ ] 窗口标题更新
- [ ] 拖放文件打开
- [ ] 关于对话框

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
| 多文件夹 | 动态添加/删除，每行可设遍历深度 (0-100，默认0) |
| 输出目录 | 留空=原地转换；填写后镜像输入目录结构输出 |
| 自动跳过 IMS | 扫描时跳过 stem 含 `.ims` 的已打包文件 |
| 删除源文件 | 勾选后转换成功即删除原始图片 |
| 多线程 | 勾选后用 rayon 并行转换 |
| 实时进度 | 进度条 + `[当前/总数] 文件名` 文字 |

### 13.3 镜像输出示例

```
输出目录: Output
输入目录: Input0, Input1

Input1/tex0/image0.jpg + image0_00001_.jpg + image0_00002_.jpg
  → Output/Input1/tex0/image0.ims.jpg
```

### 13.4 文件分组规则

- 封面: `{base}.{ext}`
- 子图: `{base}_XXXXX_.{ext}`（5 位数字序号）
- 输出: `{base}.ims.{cover_ext}`

### 13.5 架构

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
