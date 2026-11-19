---
status: accepted
---

# 采用 Skia OpenGL + WGL 独立共享上下文合成 libmpv 视频帧，并以 SW 帧回调作为软件渲染回退

## 背景与前置问题

根据 [ADR-0003](0003-skia-renderer-and-spring-motion.md) 与 [ADR-0005](0005-hybrid-media-decoding.md)，AnyFile 在 Windows 10/11 上采用 Slint Skia GPU 渲染后端（支持软件渲染回退）并按需动态加载 `mpv-2.dll` 进行视频解码。然而在正式实现视频视图（Viewer）前存在两项核心技术不确定性：

1. Slint 1.19 的 `renderer-skia` 在 Windows 上默认走 WGPU/Direct3D，若不额外配置不会暴露 `GraphicsAPI::NativeOpenGL`；而 `libmpv` 的跨驱动通用渲染接口为 `MPV_RENDER_API_TYPE_OPENGL` 与 `MPV_RENDER_API_TYPE_SW`。
2. 直接使用 Win32 子 HWND（`wid`）嵌入 `mpv` 会带来经典的原生窗口 Airspace 遮挡问题，导致 Slint 的渐隐悬浮标题栏、底部播放控制条与物理弹簧动效无法直接叠加在视频画面之上。

因此我们在工单 #8 中构建了可复现的 Windows Spike 验证程序（`examples/mpv_compositing_spike.rs`）与安全加载/沙箱测试套件（`tests/video_spike.rs`），使用真实官方 `libmpv`（`mpv-2.dll`）与 FFmpeg 生成的 `1920x1080 @ 60fps` H.264 视频，对三种候选方案进行了端到端实测对比。

## 实测环境与 GPLv3 二进制合规记录

- **操作系统**：Windows 10/11 x64
- **Slint 版本**：`1.19.0`（`backend-winit` + `renderer-skia` + `renderer-skia-opengl` + `renderer-software`）
- **测试媒体**：FFmpeg 生成的 `1920x1080 @ 60fps` H.264 (`yuv420p`) 3 秒循环测试视频
- **随包 `libmpv` 上游来源（满足 ADR-0001 GPLv3 合规）**：
  - 上游仓库：`https://github.com/shinchiro/mpv-winbuild-cmake`
  - 固定版本标签：`20261111`
  - 归档文件：`mpv-dev-x86_64-20261111-git-bcde6b93cc.7z`
  - 归档 SHA256：`273c079b11ea5575822f972756f4c9a71149a5c10130353abeb8586738938452`
  - 提取的 `mpv-2.dll`（原名 `libmpv-2.dll`）SHA256：`88504ee5560942a955405e99dfb5971368e4cf851321f36565bf2a667271db1b`
  - 可复现获取与校验脚本：`scripts/fetch_libmpv.ps1`（大体积 DLL 不直接提交进 Git 仓库，构建/打包阶段由脚本拉取并校验摘要）

## 候选方案实测对比

| 维度 | 方案 A：Skia OpenGL + WGL 独立共享上下文 + `mpv_render_context(opengl)` | 方案 B：Win32 子 HWND (`wid`) + 透明顶层浮层 | 方案 C（软件回退）：`mpv_render_context(sw)` -> `slint::Image` |
| :--- | :--- | :--- | :--- |
| **实际生效硬解 (`hwdec-current`)** | `nvdec`（自动选择硬解，无 NV GPU 时回退 `d3d11va-copy` / `dxva2-copy`） | `d3d11va`（原生零拷贝） | `no`（纯 CPU 软解 + `sws-fast`） |
| **2.2s 实测帧数 / 帧率** | **102 帧**（~60fps 稳定驱动，1280x720 FBO 纹理） | 60.00 fps（0 丢帧） | 60 帧（1280x720，受 CPU 渲染耗时制约） |
| **单帧渲染耗时 (`avg_render_ms`)** | **1.264 ms/frame** | 由 mpv 独立 swapchain 呈现 | **21.987 ms/frame**（约方案 A 的 17.4 倍） |
| **进程 CPU 时间增量（~2s 播放）** | **1125.0 ms**（含 Slint UI 同帧合成） | 718.8 ms（不含透明浮层窗口） | 1937.5 ms（仅 1.75s，CPU 压力高） |
| **渐隐控件叠加 / Airspace 表现** | **极佳（零 Airspace）**：通过 `BorrowedOpenGLTextureBuilder` 直接作为普通 Slint `Image` 参与同帧 Alpha 混合 | **差（存在 Airspace 遮挡）**：必须额外维护 `WS_EX_LAYERED` 透明附属窗口，带来拖拽撕裂、焦点/Z-order 同步与多屏 DPI 漂移 | **极佳（零 Airspace）**：与方案 A 共享同一套 Slint UI 树，完全无遮挡 |
| **高 DPI / 缩放与弹簧动效** | **原生支持**：随 Slint `Image` 自动缩放、圆角裁剪与弹簧动效联动 | 复杂：需手动监听 `WM_SIZE`/`WM_DPICHANGED` 同步子 HWND 与浮层坐标 | **原生支持**：与方案 A 完全一致 |
| **软件渲染回退可行性** | 无 OpenGL 驱动时自动切换至方案 C | 依赖 `vo=gpu` 或 D3D/WGL，无 GPU 时仍需退回软渲且仍有 Airspace 问题 | **专为软件渲染回退设计** |

## Spike 捕获的关键工程发现与解法

1. **`renderer-skia` 默认不编译 OpenGL 路径**：
   实测表明仅在 `Cargo.toml` 启用 `slint` 的 `renderer-skia` 时，`renderer_name("skia-opengl")` 会报 `unrecognized renderer`。必须同时启用 `renderer-skia-opengl` feature；启用后，Slint 1.19 的 `renderer_name("skia")` 在 Windows 上会自动优先初始化 `OpenGLSurface` 并暴露 `GraphicsAPI::NativeOpenGL`，若驱动不支持 OpenGL 则自动回退 `wgpu` -> `software`，与现有 `init_slint_backend()` 零破坏兼容。
2. **Skia 与 libmpv 共用同一 WGL 上下文会导致 `flush failed`**：
   Slint 1.19 的 `i-slint-renderer-skia` 在 `BeforeRendering` 回调后未调用 `GrDirectContext::reset(None)`。若 `mpv_render_context_render` 直接在 Slint 的 WGL 上下文中执行，会污染 Skia 内部缓存的 GL 状态机并在下一帧触发 `Skia renderer: flush failed`。
   **解法**：在 `RenderingSetup` 阶段通过 `wglCreateContext(hdc)` + `wglShareLists(slint_glrc, mpv_glrc)` 创建独立的共享 WGL 上下文。`mpv` 的 FBO 与 `mpv_render_context` 全部隔离在 `mpv_glrc` 中执行（渲染后调用 `glFlush()` 并切回 `slint_glrc`），而目标 `GL_TEXTURE_2D` 通过 `wglShareLists` 零拷贝共享给 Slint 的 `BorrowedOpenGLTextureBuilder`。实测彻底消除 `flush failed` 且单帧耗时仅 `1.264 ms`。
3. **`demuxer-lavf-o` 逗号分隔语法陷阱**：
   向 `mpv_set_option_string("demuxer-lavf-o", "protocol_whitelist=file,crypto,data")` 传入未转义逗号时，mpv 会将逗号拆分为多个 `key=value` 项并返回 `MPV_ERROR_OPTION_ERROR (-7)`。改为严格单值白名单 `demuxer-lavf-o=protocol_whitelist=file` 与 `stream-lavf-o=protocol_whitelist=file`，既通过运行时校验又具有更小攻击面。

## 最终决策

1. **主合成方案（选定方案 A）**：采用 **Slint `renderer-skia-opengl` + WGL 独立共享上下文（`wglCreateContext` / `wglShareLists`）+ `mpv_render_context(MPV_RENDER_API_TYPE_OPENGL)` 离屏 FBO 纹理 + `slint::BorrowedOpenGLTextureBuilder`**。
   - 彻底消除 Airspace 遮挡，保证渐隐悬浮标题栏、底部控制条、字幕与物理弹簧动效在同一 Skia 渲染帧内平滑合成；
   - 保留 GPU 硬件解码（`hwdec=auto-safe`）与 ~1.26ms/frame 的极低渲染开销。
2. **软件渲染回退策略（选定方案 C）**：当 `ActiveRenderer::Software` 生效或 OpenGL/WGL 共享上下文创建失败时，自动降级为 **`MPV_RENDER_API_TYPE_SW` + `sws-fast=yes` -> `slint::SharedPixelBuffer<Rgba8Pixel>` -> `slint::Image::from_rgba8`**，按视口尺寸限幅（软件回退上限建议 720p）以控制 CPU 占用，UI 层代码零分叉。
3. **否决方案 B（子 HWND + 透明浮层）**：因 Airspace 遮挡与多窗口同步复杂度过高，不予采用。

## 安全动态加载与沙箱隔离规范（已验证）

- **防 DLL 劫持**：首次加载 `mpv-2.dll` 前调用进程级 `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)`，仅允许绝对路径（默认当前可执行文件同目录），使用 `LoadLibraryExW(..., LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)` 加载；拒绝任何相对路径。
- **非视频启动路径零加载**：`LazyMpvVideoBackend` 基于 `OnceLock` 懒加载，打开图片、纯 Rust 音频、文本或欢迎空态时 `mpv-2.dll` 绝不被载入进程地址空间（已通过 `GetModuleHandleW("mpv-2.dll")` 验证）。
- **强制沙箱隔离**：`MpvInstance::new_sandboxed` 在 `mpv_initialize()` 前强制覆写 `config=no`、`load-scripts=no`、`ytdl=no`、`load-auto-profiles=no`、`osc=no`、`demuxer-lavf-o=protocol_whitelist=file`、`stream-lavf-o=protocol_whitelist=file`，任意一项失败即中止初始化；应用层 `validate_local_media_path` 额外拒绝 `http://`、`https://`、`rtsp://`、`edl://`、`smb://` 等非本地协议。

## 面向工单 #9（视频视图）的接口契约

工单 #9 可直接复用 `src/video/` 已沉淀的稳定边界，无需重复实现加载器与 FFI：

- `src/video/loader.rs`：`MpvLibrary`（安全动态加载、ABI 2.x 校验、`render_context_*` 生命周期封装）与 `is_mpv_module_loaded_in_process()`。
- `src/video/sandbox.rs`：`MpvSandboxPolicy::required_options()` 与 `validate_local_media_path()`。
- `src/video/mpv_backend.rs`：`MpvInstance`（沙箱化 RAII 实例、属性读写、命令执行、事件等待）与 `LazyMpvVideoBackend`（实现 `crate::ports::VideoBackend`，提供 `probe()` 与 `render_first_frame_sw()`）。
- `src/video/compositing.rs`：`CompositingStrategy`、`VideoFrameRgba`、`VideoPlaybackState`、`VideoCommand`。
- 渲染生命周期约束：工单 #9 在 `RenderingState::RenderingTeardown` 时，必须严格按 `mpv_render_context_free` -> 删除 FBO -> 切回 `slint_glrc` -> `wglDeleteContext(mpv_glrc)` -> 删除共享纹理 -> `drop(MpvInstance)` 的顺序释放资源，避免 libmpv 或 WGL 悬空资源。

## 工单 #9 落地状态与实测修正

- **实际落地：软件渲染路径（方案 C）**。工单 #9 的视频视图 (`src/video_viewer/`) 以 `MPV_RENDER_API_TYPE_SW`（`rgb0`，alpha 强制置 255）渲染到 `Image::from_rgba8`，渲染分辨率限幅 1280x720（`src/video_glue.rs`），`hwdec=auto-copy-safe`（硬解后拷回内存，供软件渲染使用）。该路径经真实 libmpv 实测（`tests/video_viewer.rs`，覆盖 MP4/MKV/WebM/MOV/AVI/WMV/FLV 首帧非黑、seek/倍速/音量/静音、内嵌与外挂字幕开关、Opus 音频回退）。
- **方案 A（Skia OpenGL + WGL 共享上下文）尚未接入应用**：仅有 `examples/mpv_compositing_spike.rs` 的 spike 实现。本工单未复现上文方案 A 的实测数据（102 帧、~1.26ms/frame 等）；这些数字应视为未经复现的参考值，后续接入 GL 路径时需重新测量。因此上文“最终决策”中的方案 A 仍是目标方案，但当前发布路径为方案 C。
- **后续**：在 `RenderingNotifier` 中接入方案 A 并以 `ActiveRenderer` 与上下文创建结果自动选择，失败则保留现有软件路径（UI 层 `VideoViewerPanel` 仅依赖 `image` 属性，无需改动）。
