# GPLv3 合规说明（libmpv / FFmpeg 随包二进制）

AnyFile 以 GPL-3.0-or-later 发布（见 `LICENSE`、ADR-0001）。安装包 / Portable ZIP 中随附的 `mpv-2.dll` 是第三方预编译的 libmpv，内含静态链接的 FFmpeg 及其他库，按 GPL 条款分发。本文档记录其确切来源，供获取对应源码。

## 随包二进制

| 项目 | 内容 |
| --- | --- |
| 文件 | `mpv-2.dll`（由上游 `libmpv-2.dll` 原样复制重命名，ADR-0005） |
| 获取脚本 | `scripts/fetch_libmpv.ps1`（固定版本 + SHA256 校验） |
| 上游项目 | shinchiro/mpv-winbuild-cmake（<https://github.com/shinchiro/mpv-winbuild-cmake>） |
| Release 标签 | `20261111` |
| 归档名 | `mpv-dev-x86_64-20261111-git-bcde6b93cc.7z`（SHA256 `273c079b11ea5575822f972756f4c9a71149a5c10130353abeb8586738938452`） |
| 下载地址 | `https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20261111/mpv-dev-x86_64-20261111-git-bcde6b93cc.7z` |
| 解压出的 `libmpv-2.dll` | 121,789,952 字节（本机实测） |
| mpv 版本 | `v0.42.0-38-gbcde6b93c`（DLL 内嵌版本串；对应 mpv 仓库提交 `bcde6b93cc`） |
| FFmpeg | libavformat `Lavf63.18.100`；DLL 内嵌 FFmpeg Git 版本串 `N-128766-gcd4e96eb3`（即 FFmpeg 主干提交 `cd4e96eb3`，非正式发行版） |
| 目标平台 | x86_64-w64-mingw32（GCC / MinGW-w64 工具链，由上游 GitHub Actions 构建） |

## 构建配置

- 由上游 CMake 超级构建工程 `mpv-winbuild-cmake` 在 GitHub Actions 中构建，包内各依赖（FFmpeg、libplacebo、libvpx、libass 等）均作为上游工程 `src_packages/` 的固定提交静态链接进 DLL。
- 可从 DLL 中确认的片段：libvpx 以 `--target=x86_64-win64-gcc --disable-examples --disable-docs --disable-tools --enable-vp9-highbitdepth` 配置；内含 libplacebo。
- **待补**：FFmpeg 的完整 `./configure` 参数串未内嵌在该 DLL 中（需到上游 Release 对应的 CI 构建日志或 `ffmpeg.cmake` 包定义核对，确认是否启用 `--enable-gpl` / `--enable-version3`）。在核对完成前，按保守原则整体视为 GPLv3 组合作品分发。
- **待补**：随包 DLL 中全部静态链接子库（如 libass、libplacebo、libvpx、dav1d 等）的逐项版本表，应从上游该 Release 的 CI 提交记录核对后补入。

## 源码获取

1. mpv / FFmpeg / 各依赖的对应源码：上游工程 `mpv-winbuild-cmake` 的提交历史记录了每个依赖的固定版本；按 Release 标签 `20261111`（构建日期 2026-11-11）检出该工程，运行其 CMake 超级构建即可得到各组件源码。
2. mpv 源码：<https://github.com/mpv-player/mpv>，提交 `bcde6b93cc`。
3. FFmpeg 源码：<https://git.ffmpeg.org/ffmpeg.git>，提交 `cd4e96eb3`。
4. 如无法访问上述地址，AnyFile 维护者承诺自分发之日起至少 3 年内，应请求提供上述源码的完整副本（请在 <https://github.com/rt-game-440/any-file/issues> 提交请求）。
5. AnyFile 自身源码：<https://github.com/rt-game-440/any-file>（GPL-3.0-or-later）。

## 用户替换权

`mpv-2.dll` 是独立的动态库，位于程序目录，用户可替换为自行构建的兼容 libmpv（需导出相同的 libmpv C API）。AnyFile 仅在播放视频或需回退的音频时才加载它，加载路径固定为可执行文件目录（ADR-0005）。

## 许可证相互关系

- AnyFile：GPL-3.0-or-later；Slint 通过其 GPLv3 许可路径使用（ADR-0001）。
- libmpv / FFmpeg 构建：GPLv2+/GPLv3（取决于上游构建选项，见“待补”）；与 AnyFile 的 GPLv3 兼容。
- 其余第三方库见 `THIRD_PARTY_LICENSES.md`。
