# imsViewer Android 首版开发计划

更新日期：2026-06-17  
参考文档：[ANDROID_IMAGE_VIEWER_RESEARCH.md](ANDROID_IMAGE_VIEWER_RESEARCH.md)、[DESIGN.md](DESIGN.md)

## 1. 首版定位

`imsViewer Android` 首版定位为本地优先的 IMS/图片查看器，不做完整相册应用。

核心目标：
- 可靠打开 IMS、JPEG、PNG、WebP、AVIF 文件。
- 完成单图查看、适屏、缩放、平移。
- 清晰展示 IMS 子图和基础元数据。
- 保持只读查看器边界，不做编辑、管理、批处理。

首版用户路径：

```text
启动 App
  -> 选择 IMS/图片文件
  -> 查看当前图片
  -> IMS 文件可点击缩略图切换子图
  -> 打开信息面板查看文件/子图信息
  -> 可选调用系统分享
```

## 2. 当前工程状态

当前仓库原本没有 `android/` 工程目录。本轮从零创建 Android 工程基线，入口位于：

- `android/settings.gradle.kts`
- `android/build.gradle.kts`
- `android/app/build.gradle.kts`
- `android/app/src/main/java/com/imsviewer/android/MainActivity.kt`

本机已发现：
- JDK：`C:\Program Files\Eclipse Adoptium\jdk-17.0.19.10-hotspot`
- Android SDK：`C:\Users\monst\AppData\Local\Android\Sdk`
- 可用 SDK：API 35、34、28
- Gradle 缓存：Gradle 8.7、AGP 8.4.0、Kotlin 2.0.0、Compose BOM 2024.06.00

## 3. 技术选型

| 层面 | 首版选择 | 说明 |
|---|---|---|
| 开发语言 | Kotlin | Android 主流选择 |
| UI | Jetpack Compose + Material 3 | 声明式 UI，适合状态驱动查看器 |
| 架构 | 单 Activity + ViewModel + Repository | 避免 Activity 承担业务 |
| 状态 | `StateFlow` | ViewModel 对外暴露 UI 状态 |
| 文件访问 | SAF / `ACTION_OPEN_DOCUMENT` | 支持本地和文档提供器文件 |
| 图片解码 | Android `ImageDecoder` / `BitmapFactory` | 普通图片和 IMS 子图字节解码 |
| IMS 解析 | Kotlin 原生实现 | 对齐桌面端格式 |
| 并发 | Coroutines | 文件读取、解析、解码放后台 |
| minSdk | API 26 | Android 8.0+ |
| targetSdk | API 35 | 以当前可用 SDK 为准 |

## 4. 首版功能范围

### 4.1 必须实现

| 模块 | 功能 |
|---|---|
| 文件打开 | 通过系统文件选择器打开 IMS/JPEG/PNG/WebP/AVIF |
| 普通图片查看 | 适屏显示、双指缩放、单指平移、双击缩放 |
| 手势体验 | 单击显隐 UI、双击围绕点击点缩放、双指围绕手势中心缩放、放大后拖拽并限制边界 |
| IMS 查看 | 解析 IMS 索引，显示封面和子图 |
| 子图切换 | IMS 多子图时显示真实缩略图条，点击或适屏上下滑循环切换 |
| 文件序列 | 支持 SAF 同目录扫描，适屏状态左右滑切换上一/下一文件 |
| 信息面板 | 展示文件名、格式、尺寸、大小、URI、IMS 子图信息 |
| 沉浸查看 | 单击主图显示/隐藏顶部栏、底部栏、缩略图 |
| 横屏适配 | 横屏下主图区域最大化，缩略图移到侧边 |
| 错误处理 | 不支持格式、解析失败、权限失效、解码失败都有提示 |
| 性能保护 | 后台解码，任务可取消，缩略图基础缓存 |

### 4.2 首版可选

- 系统 ShareSheet 分享当前文件。
- 接收其他应用“打开方式”传入的文件 URI。
- 同目录图片序列浏览。
- 复制 URI 或 SHA-256。
- 最近打开记录。

### 4.3 明确不做

- 完整图库首页。
- 相册管理、移动、复制、删除、回收站。
- 图片编辑、裁剪、滤镜、标注。
- IMS 编辑、重排、保存、批量导出。
- 多选批量操作。
- 云同步、账号、AI 搜索。
- 视频播放。
- 复杂主题和大量设置。

## 5. 任务状态

说明：以下状态已按用户要求重置，所有开发任务重新从未完成开始记录。已经在代码中推进的内容也暂不勾选，等完成构建和验收后再逐项标记。

### M0 代码盘点与基线确认

目标：确认 Android 工程能作为首版开发基线。

- [ ] 创建或确认 `android/` 工程目录。
- [ ] 运行 `android/gradlew.bat :app:assembleDebug`。
- [ ] 记录包名、minSdk、targetSdk、Compose/Kotlin 版本。
- [ ] 梳理 `android/app/src` 文件结构。
- [ ] 建立构建命令备注。

验收：
- Debug 构建成功。
- 明确现有工程状态和后续入口文件。

### M1 工程骨架与基础 UI

目标：App 可启动，有清晰空状态和基础架构。

- [ ] 建立推荐包结构。
- [ ] 实现 `MainActivity` + `MainScreen`。
- [ ] 实现 `ViewerViewModel` 和 `ViewerUiState`。
- [ ] 实现空状态，提供“打开文件”按钮。
- [ ] 接入深色主题和基础 Material 3 样式。
- [ ] 加入基础错误状态组件。

验收：
- 启动后显示空状态。
- 点击打开按钮能唤起系统文件选择器。
- 旋转屏幕后不崩溃。

### M2 文件打开与普通图片显示

目标：能打开普通图片并完成基础查看。

- [ ] 实现 SAF `ACTION_OPEN_DOCUMENT`。
- [ ] 处理 URI 持久读权限。
- [ ] 实现 `ImageFormatDetector`。
- [ ] 实现普通图片 metadata 读取。
- [ ] 实现图片解码和 `ViewerImage` 状态。
- [ ] 实现 `ImageCanvas` 适屏绘制。
- [ ] 实现加载、成功、失败状态流。

验收：
- JPEG/PNG/WebP/AVIF 至少各验证一张。
- 首次显示整张图片可见，不变形。
- 解码失败时显示错误并可重新选择。

### M3 手势查看体验

目标：普通图片查看体验达到可用标准。

- [ ] 实现 `ViewerTransform` 和 `fitScale` 计算。
- [ ] 实现单击显示/隐藏 UI。
- [ ] 实现双指缩放和平移。
- [ ] 实现放大状态单指拖拽。
- [ ] 实现双击适屏/放大切换。
- [ ] 实现偏移 clamp。
- [ ] 实现横竖屏重新计算合法 transform。

验收：
- 单击、双击、双指缩放、拖拽符合研究文档手势规范。
- 放大状态不会误触发切图。
- 横竖屏切换后图片仍在合法范围。

### M4 IMS 解析与子图显示

目标：IMS 文件能解析、显示、切换子图。

- [ ] 实现 `ImsFile`、`SubImage`、`ImageFormat`。
- [ ] 实现 little-endian 字节读取工具。
- [ ] 实现 `ImsParser`。
- [ ] 实现 IMS 子图按需读取。
- [ ] 实现 IMS 子图解码。
- [ ] 使用 `samples/` 建立解析测试。
- [ ] 对损坏 IMS 增加错误处理。

验收：
- 样例 IMS 能解析出正确子图数量。
- 默认显示封面或指定当前子图。
- 切换子图不重新解析整个文件。
- 损坏 IMS 不崩溃。

### M5 同目录浏览

目标：打开一个文件后可浏览同目录支持格式。

- [ ] 实现 `FileRepository` 同目录扫描。
- [ ] 过滤 IMS/JPEG/PNG/WebP/AVIF。
- [ ] 实现自然排序。
- [ ] 当前文件定位到序列中。
- [ ] 实现 `openPreviousFile()` / `openNextFile()`。
- [ ] 实现适屏状态左右滑切换。
- [ ] 处理目录不可扫描降级。

验收：
- 底部显示 `当前序号 / 总数`。
- 适屏左滑下一张，右滑上一张。
- 第一张、最后一张不循环。
- 无法扫描目录时仍可查看当前文件。

### M6 IMS 缩略图与信息面板

目标：IMS 多子图浏览和元数据查看完整。

- [ ] 实现 `ThumbnailRepository`。
- [ ] 实现 IMS 子图缩略图异步生成。
- [ ] 实现竖屏 `ThumbnailStrip`。
- [ ] 实现横屏侧边缩略图布局。
- [ ] 实现当前子图高亮和滚动定位。
- [ ] 实现 `ImageInfoSheet`。
- [ ] 展示普通图片信息。
- [ ] 展示 IMS 文件和当前子图信息。
- [ ] 可选实现 URI/SHA-256 复制。

验收：
- IMS 多子图显示缩略图条。
- 点击缩略图切换子图，信息同步更新。
- 普通图片不显示 IMS 子图条。
- 信息面板可打开、关闭、返回键关闭。

### M7 UI 打磨、分享与横屏

目标：首版体验完整，达到手动验收标准。

- [ ] 完善顶部栏、底部栏、更多菜单。
- [ ] 实现沉浸模式动画。
- [ ] 完成横屏布局。
- [ ] 实现系统 ShareSheet 分享。
- [ ] 分享普通图片 URI。
- [ ] IMS 首版分享原文件 URI。
- [ ] 优化长文件名、省略、按钮间距。
- [ ] 检查小屏和高 DPI 设备布局。

验收：
- UI 不重叠、不溢出。
- 横屏主图区域最大化。
- 分享能唤起系统分享面板。
- 沉浸模式显示/隐藏自然。

### M8 稳定性、性能与发布包

目标：生成可验收 APK。

- [ ] 快速切图时取消旧解码任务。
- [ ] 限制 bitmap/thumbnail 缓存。
- [ ] 大图降采样和 OOM 防护。
- [ ] 后台计算 SHA-256。
- [ ] 增加关键单元测试。
- [ ] 增加手动验收样例清单。
- [ ] 运行 debug/release 构建。
- [ ] 生成签名 APK 或 release artifact。

验收：
- 快速左右滑 20 次不崩溃。
- 连续打开多个 IMS 文件内存不持续不可控增长。
- 大图失败时显示错误，不闪退。
- APK 可安装，核心验收清单通过。

## 6. 首版实现备注

本轮推进目标：
- 创建 Android/Compose 工程。
- 实现 SAF 打开文件。
- 实现普通图片和 IMS 文件解析显示。
- 实现首版手势体验：单击显隐 UI、双击缩放、双指缩放、放大拖拽、边界 clamp。
- 实现 IMS 子图真实缩略图和适屏上下滑循环切换。
- 实现 SAF 同目录扫描和适屏左右滑切换图片文件。
- 实现信息面板和系统分享入口。

受本地环境影响，构建命令建议：

```powershell
$env:JAVA_HOME='C:\Program Files\Eclipse Adoptium\jdk-17.0.19.10-hotspot'
$env:ANDROID_HOME='C:\Users\monst\AppData\Local\Android\Sdk'
cd android
.\gradlew.bat :app:assembleDebug
```

## 7. 手动验收清单

准备以下文件：
- 普通 JPEG 横图、竖图、方图。
- PNG 透明图。
- WebP 静态图。
- AVIF 图片。
- 多子图 IMS。
- 损坏 IMS。
- 超大分辨率图片。
- 包含非图片文件的混合目录。

首版总体验收：

| 类别 | 验收项 |
|---|---|
| 打开 | 能打开 IMS/JPEG/PNG/WebP/AVIF |
| 显示 | 首次显示适屏，不裁切，不变形 |
| 手势 | 单击、双击、双指缩放、放大拖拽可用 |
| IMS | 能显示子图数量，点击缩略图或适屏上下滑循环切换子图 |
| 文件切换 | 同目录支持格式文件可在适屏状态左右滑切换 |
| 信息 | 普通图片和 IMS 信息准确，面板可关闭 |
| 沉浸 | 单击隐藏 UI 后图片区域最大化 |
| 横屏 | 横屏布局可用，当前文件/子图保持 |
| 错误 | 不支持格式、损坏 IMS、权限失效都有提示 |
| 分享 | 普通图片和 IMS 原文件可调用系统分享 |
