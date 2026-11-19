# v0.1 发布验收

## 自动验收（一键）
`powershell -File scripts\run_acceptance.ps1`：release 构建、安装脚本校验（ISCC 编译 + 注册表对称性）、Portable 构建与校验、全语料 E2E 冒烟、冷启动基准、深浅色截图（输出到 logs/）。

## 人工验收（未在开发机执行，避免污染注册表）
1. 运行 `scripts\build_installer.ps1` 产出安装包；当前用户安装（应无 UAC）、全机安装（向导弹出权限选择后选择全机）。
2. 分别勾选图像/视频/音频/文本类，检查 HKCU/HKLM 下 `Software\AnyFile\Capabilities`、`RegisteredApplications`、`Software\Classes\AnyFile.*`。
3. 完成页勾选“默认应用”，确认跳转到 Windows 设置并可选择 AnyFile。
4. 卸载后确认上述注册表项与安装目录均已清除（%APPDATA%\AnyFile 交互卸载时询问）。
5. 查看 `logs/screenshots/*.png` 的深浅色截图。
