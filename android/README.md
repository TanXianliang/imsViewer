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

从仓库根目录构建 Release artifact：

```powershell
.\buildrelease_apk.bat
# bin/android/imsViewer-android-release.apk
```

当前 Release artifact 未配置正式签名。

## 本地浏览 (SAF)

- SAF 打开 IMS / 常见图片文件（JPG / PNG / WebP / AVIF）。
- 普通图片适屏显示、双指缩放、放大拖拽、双击围绕点击点缩放。
- 手势边界保护：放大后拖拽限制在图片可见范围内，缩回适屏时自动归零偏移。
- IMS v1/v2 footer 解析，兼容可选 `IMSTHMB` Trailer，按子图切换显示。
- IMS 多子图真实缩略图条。
- IMS 适屏状态下支持主图区域上下滑循环切换子图。
- SAF 同目录扫描，适屏状态下左右滑切换上一 / 下一文件。
- 本地文件夹支持大图标、中图标、小图标和列表四种显示模式，并记住上次选择。
- 文件和 IMS 子图信息 bottom sheet。
- 系统 ShareSheet 分享当前原文件 URI。

## 远程浏览 (SFTP)

- 保存多个 SFTP 连接并浏览远程目录树（记住连接 / 自动回到上次路径）。
- 密码默认隐藏并可查看；仅在用户勾选时保存，使用 Android Keystore AES-GCM 加密。
- 远程文件列表按文件名排序，目录优先；显示文件大小和修改时间。
- 普通图片缩略图：后台拉取并按 SHA-256(path:size:mtime) 缓存到本地 `sftp_thumbnails` 目录；源文件超过 20MB 不生成缩略图。
- 新 IMS 缩略图：先读取尾部 36B，再按 `IMSTHMB` Trailer 只读取内嵌 160px JPEG 和校验 CRC32。
- 旧 IMS 缩略图：分段读取尾部索引定位封面，只下载封面数据，不下载整个 IMS。
- 远程目录只加载可见项及前后各 8 项；可见项优先，快速滚动时保留当前下载并动态重排待处理队列。
- 远程目录支持大图标、中图标、小图标和列表四种显示模式，并记住上次选择。
- 缩略图使用单 worker，同一批次复用一个 SFTP Session/Channel，避免逐张重新登录。
- 远程 IMS 文件点击下载到本地缓存后进入浏览；同目录切换复用缓存。
- 缩略图任务随目录切换 / 返回首页自动取消，避免无谓流量。
- 可配置网络缓存上限（默认 512MB），缓存超出自动按 LRU 清理。

## 网络缓存

- 缓存目录：`cacheDir/sftp`（远程文件）+ `cacheDir/sftp_thumbnails`（缩略图）。
- `sftp_thumbnails` 独立限制为 96MB，按最后访问时间执行 LRU 淘汰。
- 设置页可调整上限（MB），`清除缓存` 一并清空两个目录。
- 缩略图缓存键包含修改时间，远端文件更新后自动失效重建。
- App 正常退出时不会主动清空缓存；由容量上限、系统缓存回收或用户手动清理控制。

## Known Gaps

- 同目录扫描依赖 DocumentsProvider 支持；不支持列目录时会降级为单文件查看。
- 手势体验仍需真机验证，尤其是上下滑子图切换、双指缩放、边界拖拽和横竖屏切换。
- 普通超大图仍需要更完整的降采样 / OOM 保护。
- 需要真机验证 SAF 兼容性、横屏布局和分享 URI。
- SFTP 仅支持密码认证；大文件下载未断点续传。
- SFTP 当前关闭严格 host key 校验，正式发布前应增加 known-hosts/指纹确认。
