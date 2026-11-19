---
status: accepted
---

# 音视频与图像采用“运行时零额外 DLL + 视频按需加载 libmpv”混合解码架构

为兼顾非视频文件的极致冷启动与视频格式的高兼容性，我们决定按媒体大类拆分解码引擎：图像与主流音频优先使用纯 Rust 及静态链接解码库（如 AVIF 使用 `dav1d`），保持运行时零额外 DLL；视频及纯 Rust 尚未成熟的音频编码（如 Opus、部分 HE-AAC）则按需动态加载随包分发的 `libmpv`（`mpv-2.dll`），以直接获得 FFmpeg 全格式支持、硬解、音画同步、倍速变调与 ASS/SRT 字幕渲染。

## Consequences

- 打开普通图片与主流音频时不加载 `libmpv`，避免拖慢冷启动。
- 动态加载 `mpv-2.dll` 时必须使用可执行文件目录绝对路径并调用 `SetDefaultDllDirectories` 防 DLL 劫持；初始化 mpv 强制禁用外部配置、脚本与网络协议（`config=no`、`load-scripts=no`、`ytdl=no`）。
- 动工初期需优先完成 Slint Skia（Windows 默认 D3D）与 `libmpv`（OpenGL/软件渲染）画面合成方案的 1–2 天技术验证（Spike），并另行沉淀渲染集成 ADR。
