# Slint 文本输入控件中文 IME 实测验证记录（工单 #3）

## 1. 验证环境与版本

- **操作系统**：Windows 10/11 x64（Per-Monitor V2 高 DPI 感知 + UTF-8 进程代码页清单）
- **UI 框架**：`slint 1.19.0`（`backend-winit` + `renderer-skia` / `renderer-software` 回退）
- **窗口后端**：`i-slint-backend-winit 1.19.0` + `winit 0.30.13`
- **无边框窗口壳层**：`src/window_shell/win32.rs`（`SetWindowSubclass` + `WM_NCCALCSIZE` + `WM_NCHITTEST`）

## 2. 源码级 IME 管线核查

经核查本地依赖源码（`i-slint-backend-winit-1.19.0/winitwindowadapter.rs` 与 `winit-0.30.13/src/platform_impl/windows/event_loop.rs`），当前技术栈已完整打通 Windows 中文 IME 链路：

1. **焦点与 IME 开关**：Slint `TextInput` / `TextEdit` 获得焦点时触发 `InputMethodRequest::Enable`，`i-slint-backend-winit` 调用 `winit_window.set_ime_allowed(true)`；失焦时触发 `InputMethodRequest::Disable`，调用 `set_ime_allowed(false)`。
2. **候选框跟随**：输入焦点或光标移动时触发 `InputMethodRequest::Update`，`i-slint-backend-winit` 将光标逻辑矩形转换为物理坐标后调用 `winit_window.set_ime_cursor_area(...)`（底层调用 Win32 `ImmSetCandidateWindow` / `ImmSetCompositionWindow`），实现中文输入法候选框跟随光标。
3. **组合态（Preedit）与上屏（Commit）**：
   - `winit` 在 `WM_IME_SETCONTEXT` 隐藏旧式浮动组合框（`ISC_SHOWUICOMPOSITIONWINDOW`），保留系统候选词窗；
   - `WM_IME_COMPOSITION` 阶段按 `GCS_COMPSTR` 派发 `Ime::Preedit` -> Slint `KeyEventType::UpdateComposition`（内联下划线预编辑显示）；
   - 按 `GCS_RESULTSTR` 派发 `Ime::Commit` -> Slint `KeyEventType::CommitComposition`（候选词上屏并提交至文本缓冲区）。

## 3. 已落地的平台约束与 Workaround

1. **子类化挂载时机（winit 0.30 关键约束）**：
   - `winit 0.30` 在 `slint::run_event_loop()` 启动（`resumed`）后才真正创建 Win32 `HWND`；此前调用 `window.window_handle()` 会返回不可用错误。
   - **已落地修复**：`src/main.rs` 中通过 `slint::invoke_from_event_loop` 在事件循环首帧派发时挂载 `WindowsShellHandle::install`，确保 `HWND` 必然就绪。
2. **Win32 子类化不得拦截 IME 消息**：
   - `src/window_shell/win32.rs` 的 `window_shell_subclass_proc` 仅处理无边框外壳与命中测试相关消息（`WM_NCCALCSIZE`、`WM_NCHITTEST`、`WM_NCMOUSEMOVE`、`WM_NCMOUSELEAVE`、`WM_NCLBUTTONDOWN`、`WM_NCLBUTTONUP`、`WM_GETMINMAXINFO`、`WM_SIZE`、`WM_NCDESTROY`）。
   - 所有 `WM_IME_*`（`WM_IME_SETCONTEXT`、`WM_IME_STARTCOMPOSITION`、`WM_IME_COMPOSITION`、`WM_IME_ENDCOMPOSITION`）、`WM_CHAR` 与 `WM_DPICHANGED` 均**原样透传给 `DefSubclassProc`**，避免破坏 `winit` 的 IME 组合与高 DPI 处理。
3. **应用清单 UTF-8 与 PerMonitorV2**：
   - `assets/anyfile.manifest` 已启用 `PerMonitorV2` 与 `activeCodePage=UTF-8`，保证多 DPI 显示器间拖动窗口时，Slint 向 `winit` 上报的 `set_ime_cursor_area` 物理坐标缩放与窗口一致，候选框不会漂移。

## 4. 自动化实测验证结果

可通过以下命令在本地 Windows 环境下重复执行真实 HWND 端到端回归测试：

```powershell
$env:CARGO_TARGET_DIR="E:\devlib\anyfile-target"
$env:CARGO_BUILD_JOBS="2"
cargo test --test win32_shell_and_ime -- --test-threads=1 --nocapture
```

测试文件 [`tests/win32_shell_and_ime.rs`](../tests/win32_shell_and_ime.rs) 实测通过以下断言：

- 真实无边框窗口挂载 `WindowsShellHandle` 后，`WM_NCHITTEST` 准确返回 `HTCAPTION`、`HTMAXBUTTON`、`HTLEFT`、`HTCLIENT`；
- 聚焦 Slint `TextInput` 后，Windows 原生 `ImmGetContext(hwnd)` 返回有效非空 `HIMC`；
- 经无边框子类化透传 `WM_IME_STARTCOMPOSITION` / `WM_IME_ENDCOMPOSITION` 无异常阻塞；
- 向 Slint `TextInput` 派发中英混排与全角中文标点（`"你好，AnyFile中文测试！"`）正常上屏，无乱码或丢字。

## 5. 结论

Slint 1.19.0 + `winit 0.30.13` 配合本仓库无边框窗口子类化方案，中文 IME 无阻塞性缺陷，可安全支撑后续工单 #10（文本查看器编辑态 Editing Mode 与查找替换框）的中文输入需求。
