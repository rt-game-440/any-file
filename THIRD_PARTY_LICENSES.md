# 第三方许可清单

AnyFile 自身为 GPL-3.0-or-later（`LICENSE`）。下列直接依赖的版本取自 `Cargo.lock`，许可证取自各 crate 的 `Cargo.toml`；传递依赖的完整清单可用 `cargo tree` / `cargo license` 生成（待补：发布前附带自动生成的全量清单）。

## 随包本地分叉

| 组件 | 版本 | 许可证 | 说明 |
| --- | --- | --- | --- |
| `vendor/avif-decode` | 3.0.0（本地精简分叉） | BSD-3-Clause，Copyright 2021 Kornel Lesiński | 移除 bin/lodepng，rav1d 关闭 `asm` 特性（纯 Rust 路径）。许可证全文见 `vendor/avif-decode/LICENSE` |

## Rust 直接依赖

| crate | 版本 | 许可证 |
| --- | --- | --- |
| slint | 1.19.0 | GPL-3.0-only（选用此路径，见 ADR-0001） |
| image | 0.25.10 | MIT OR Apache-2.0 |
| resvg | 0.49.0 | Apache-2.0 OR MIT |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| kamadak-exif | 0.6.2 | BSD-2-Clause |
| rav1d（经 avif-decode） | 1.1.0 | BSD-2-Clause |
| symphonia | 0.5.5 | MPL-2.0 |
| lofty | 0.22.4 | MIT OR Apache-2.0 |
| cpal | 0.15.3 | Apache-2.0 |
| notify | 8.2.2 | CC0-1.0 |
| encoding_rs | 0.8.44 | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| serde / serde_json | 1.0.229 / 1.0.154 | MIT OR Apache-2.0 |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 |

## 随包二进制

| 组件 | 许可证 | 说明 |
| --- | --- | --- |
| `mpv-2.dll`（libmpv，静态含 FFmpeg 等） | GPL（见 `GPL-COMPLIANCE.md`） | 版本、来源、源码获取方式见 `GPL-COMPLIANCE.md` |

## 渲染相关

Slint 的 Skia 渲染器通过 `skia-bindings` / `skia-safe`（MIT）链接预编译 Skia（BSD-3-Clause，Google）。待补：核对 `skia-binaries` 预编译包版本与其附带的第三方声明。
