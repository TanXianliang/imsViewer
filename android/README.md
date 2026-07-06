# imsViewer Android

imsViewer 的 Android 端，支持本地 SAF 浏览与 SFTP 远程浏览。

## Build

```powershell
$env:JAVA_HOME='C:\Program Files\Eclipse Adoptium\jdk-17.0.19.10-hotspot'
$env:ANDROID_HOME='C:\Users\monst\AppData\Local\Android\Sdk'
.\gradlew.bat :app:assembleDebug
```

Debug APK:

```text
android/app/build/outputs/apk/debug/app-debug.apk
```

## 本地浏览 (SAF)

- SAF 打开 IMS / 常见图片文件（JPG / PNG / WebP / AVIF）。
- 普通图片适屏显示、双指缩放、放大拖拽、双击围绕点击点缩放。
- 手势边界保护：放大后拖拽限制在图片可见范围内，缩回适屏时自动归零偏移。
- IMS footer 解析，按子图切换显示。
- IMS 多子图真实缩略图条。
- IMS 适屏状态下支持主图区域上下滑循环切换子图。
- SAF 同目录扫描，适屏状态下左右滑切换上一 / 下一文件。
- 文件和 IMS 子图信息 bottom sheet。
- 系统 ShareSheet 分享当前原文件 URI。

## 远程浏览 (SFTP)

- 连接 SFTP 服务器，浏览远程目录树（记住连接 / 自动回到上次路径）。
- 远程文件列表按文件名排序，目录优先；显示文件大小和修改时间。
- 普通图片缩略图：后台拉取并按 SHA-256(path:size:mtime) 缓存到本地 `sftp_thumbnails` 目录，再次进入秒开。
- 缩略图降采样后以 JPEG 压缩存储，源文件超过 20MB 不生成缩略图（避免流量与内存压力）。
- 远程 IMS 文件点击下载到本地缓存后进入浏览；同目录切换复用缓存。
- 缩略图任务随目录切换 / 返回首页自动取消，避免无谓流量。
- 可配置网络缓存上限（默认 512MB），缓存超出自动按 LRU 清理。

## 网络缓存

- 缓存目录：`cacheDir/sftp`（远程文件）+ `cacheDir/sftp_thumbnails`（缩略图）。
- 设置页可调整上限（MB），`清除缓存` 一并清空两个目录。
- 缩略图缓存键包含修改时间，远端文件更新后自动失效重建。

## Known Gaps

- 同目录扫描依赖 DocumentsProvider 支持；不支持列目录时会降级为单文件查看。
- 手势体验仍需真机验证，尤其是上下滑子图切换、双指缩放、边界拖拽和横竖屏切换。
- 普通超大图仍需要更完整的降采样 / OOM 保护。
- 需要真机验证 SAF 兼容性、横屏布局和分享 URI。
- SFTP 仅支持密码认证；大文件下载未断点续传。
