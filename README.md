# imsViewer

IMS 图片浏览器 — 基于 Tauri v2 的三栏图片浏览与批量转换工具。

## 功能

### 图片浏览
- **三栏布局**: 缩略图列表 | 画布显示 | 信息面板
- **IMS 容器支持**: 解析多子图 IMS 文件，按 `↑` `↓` 切换子图
- **文件夹导航**: `←` `→` 切换同目录文件，滚轮快捷操作
- **Canvas 渲染**: 缩放（Ctrl+滚轮）、平移（拖拽）、适应窗口 / 原始尺寸
- **拖放打开**: 原生 Tauri 拖放，直接拖入图片或 IMS 文件
- **导出**: 导出当前图片或 IMS 全部子图
- **暗色主题**: CSS Variables 暗色主题，响应式布局

### 批量转换
- 多文件夹批量扫描，按命名规范自动分组
- 支持遍历深度控制 (0-100)
- 可选输出目录，镜像输入目录结构
- 多线程并行转换 (rayon)
- 可选转换后删除源文件
- 实时进度条

## 截图

![imsViewer 主界面](ui_design/icon.png)

## 安装

从 [Releases](../../releases) 下载 `imsViewer_1.0.0_x64-setup.exe` 安装，或直接下载 `imsViewer.exe` 免安装运行。

> 需要 Windows 10+ (内置 WebView2)。

## 使用

### 浏览模式

| 操作 | 快捷键 |
|------|--------|
| 打开文件 | `Ctrl+O` 或拖放 |
| 切换文件 | `←` `→` 或 滚轮 |
| 切换子图 (IMS) | `↑` `↓` 或 `Shift+滚轮` |
| 缩放 | `Ctrl+滚轮` / `+` `-` |
| 适应窗口 | `0` / 双击 |
| 原始大小 | `1` / 双击 |
| 导出当前 | `Ctrl+E` |
| 导出全部 | `Ctrl+Shift+E` |
| 批量转换 | 菜单 工具 → 批量转换 |

### 批量转换

1. 打开 **工具 → 批量转换**
2. 添加文件夹（可多个），设置遍历深度（0=无限）
3. 可选指定输出目录和是否保持目录结构
4. 选择是否删除源文件 / 多线程
5. 点击 **开始转换**

### CLI 工具 (Python)

```bash
# 打包为 IMS
python ims_pack.py cover.jpg sub1.jpg sub2.png -o output.ims.jpg

# 解包列出内容
python ims_unpack.py -l file.ims.jpg

# 提取全部图片
python ims_unpack.py -a file.ims.jpg -o out_dir/

# 批量打包文件夹
python ims_batch.py ./images/
```

## IMS 格式

IMS (Image Set) 是一种轻量图片容器格式，将封面图 + 多张子图合并为单个文件。

```
[封面 JPEG/PNG 数据]
[子图 1 数据]
[子图 2 数据]
...
[索引尾部: IMSIMAGE + header + entries + index_offset(4B)]
```

- 封面在文件最前端 → 普通图片查看器可显示封面
- 尾部索引记录每张图的偏移和长度 → IMS 查看器可切换子图
- 子图无限制，支持 JPEG / PNG / WebP / AVIF（WebP/AVIF 自动转 PNG）
- **v2**: 每张子图附带 SHA-256 内容哈希，支持完整性校验

详见 [DESIGN.md](DESIGN.md)。

### 命名规范（批量转换）

| 角色 | 模式 | 示例 |
|------|------|------|
| 封面 | `{name}.{ext}` | `texture.jpg` |
| 子图 | `{name}_XXXXX_.{ext}` | `texture_00001_.png` |
| 输出 | `{name}.ims.{ext}` | `texture.ims.jpg` |

## 开发

```bash
# 环境要求
# - Rust 1.70+
# - Node.js 18+
# - Windows 10+ (WebView2)

# 安装依赖
npm install

# 开发模式
npm run tauri dev

# 构建发布
npm run tauri build
# → src-tauri/target/release/imsViewer.exe
```

## 技术栈

| 层 | 技术 |
|----|------|
| 框架 | Tauri v2 |
| 后端 | Rust (ims 解析/打包, walkdir, rayon) |
| 前端 | Vanilla JS + Canvas API + CSS Grid |
| 打包 | Vite + tauri-build |
| CLI 工具 | Python (ims_pack, ims_unpack, ims_batch) |

## 许可证

MIT
