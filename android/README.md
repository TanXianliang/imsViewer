# imsViewer Android

首版 Android 工程入口。

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

## Current Scope

- SAF 打开 IMS/常见图片文件。
- 普通图片适屏显示、双指缩放、放大拖拽、双击围绕点击点缩放。
- 手势边界保护：放大后拖拽限制在图片可见范围内，缩回适屏时自动归零偏移。
- IMS footer 解析，按子图切换显示。
- IMS 多子图真实缩略图条。
- IMS 适屏状态下支持主图区域上下滑循环切换子图。
- SAF 同目录扫描，适屏状态下左右滑切换上一/下一文件。
- 文件和 IMS 子图信息 bottom sheet。
- 系统 ShareSheet 分享当前原文件 URI。

## Known Gaps

- 同目录扫描依赖 DocumentsProvider 支持；不支持列目录时会降级为单文件查看。
- 手势体验仍需真机验证，尤其是上下滑子图切换、双指缩放、边界拖拽和横竖屏切换。
- 普通超大图仍需要更完整的降采样/OOM 保护。
- 需要真机验证 SAF 兼容性、横屏布局和分享 URI。
