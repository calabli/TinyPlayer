# TinyPlayer

[English](README.en.md) | 简体中文

跨平台（Windows / macOS）本地视频播放器和个人媒体库。Rust + egui（OpenGL）+ libmpv，目标是快、稳、占用小。

## 主要功能

- **播放**：libmpv 内核与硬件解码，倍速、逐帧、A-B 循环、章节和书签、进度条画面预览、续播、睡眠定时
- **画面与声音**：画面比例、裁剪去黑边、旋转翻转、色彩、画质增强、HDR 色调映射、插帧；音轨、声道、延迟、10 段均衡器、音量标准化、按输出设备记音量
- **字幕**：自动加载、双字幕、大小位置字体颜色描边、延迟
- **窗口**：全屏、无边框播放、画中画、窗口置顶、记住窗口布局、多开窗口
- **多屏同播**：一个窗口最多同时播 4 个视频，布局自由切分，可联动或只听选中
- **VR 全景**：在普通屏幕上看 180° / 360° 视频
- **媒体库**：文件夹扫描与缩略图、标签、收藏夹、集合、最常播放、搜索、重命名、跨盘移动、可能重复检查、媒体库密码
- **其他**：单实例、文件关联（Windows 安装版）、系统媒体键、播放时阻止休眠、应用内更新、隐私模式、深色/浅色主题、中英文界面

## 下载

到 [Releases](https://github.com/calabli/TinyPlayer/releases) 下载：

- Windows x64：`-setup.exe`（安装版，无需管理员权限）或 `.zip`（便携版）
- macOS（Apple 芯片）：`.dmg`，打开后把 TinyPlayer 拖到“应用程序”

安装包没有付费签名，首次打开时 Windows SmartScreen 或 macOS 可能拦截。macOS 上先打开一次，再到“系统设置 → 隐私与安全性”点“仍要打开”。

## 文档

| 文档 | 内容 |
|---|---|
| [产品文档](docs/product.md) | 产品定位、全部功能和规则、快捷键、设置参考、性能、隐私、已知限制、后续方向 |
| [技术文档](docs/architecture.md) | 架构、模块地图、渲染管线、多屏、数据存储、系统集成、更新机制、构建发布、排错、开发约定 |
| [更新记录](CHANGELOG.md) | 各版本的变化 |
| [第三方许可](THIRD_PARTY.md) | libmpv 等组件的许可（分发前请阅读） |

## 构建与运行

```bash
cargo run --release -- "D:/path/to/video.mp4"
```

构建脚本会把 `third_party/mpv/<平台>/` 下的 libmpv 复制到可执行文件旁边：

| 平台 | 需要的文件 |
|---|---|
| Windows | `third_party/mpv/windows/libmpv-2.dll`（来自 [shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake/releases) 的 `mpv-dev-x86_64` 包） |
| macOS | `third_party/mpv/macos/libmpv.2.dylib`（可用 `brew install mpv` 后从 Homebrew 目录复制，并一起打包依赖库） |

libmpv 体积较大，没有提交到 git。

## 打包

| 平台 | 命令 | 产物 |
|---|---|---|
| Windows | `powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1` | `dist\TinyPlayer-<版本>-windows-x64.zip`（便携版，解压即用，约 48MB）；装了 [Inno Setup](https://jrsoftware.org/isinfo.php) 时还会生成 `-setup.exe` 安装程序（无需管理员权限） |
| macOS | `scripts/package-macos.sh`（需要 `brew install mpv dylibbundler` 和 Xcode 命令行工具） | `dist/TinyPlayer.app` 和 `dist/TinyPlayer-<版本>-macos-<架构>.dmg`（约 32MB，打开后把 TinyPlayer 拖到“应用程序”即可安装）；设置 `SIGN_IDENTITY`、`NOTARY_PROFILE` 可签名公证 |

安装包要求的最低 macOS 版本由打包时的依赖库决定，脚本会自动写入 `Info.plist`：用 Homebrew 的 libmpv 打包时，通常等于打包机器的系统版本。

macOS 脚本默认只做临时签名，在本机可直接运行。发给别人的话，对方首次打开时系统会拦截，要先打开一次，再到“系统设置 → 隐私与安全性”点“仍要打开”；用 Developer ID 签名并公证后分发就不会被拦截。安装包只支持打包机器的架构。脚本最后会逐个加载包内的库，有库加载失败或仍依赖 Homebrew 时直接报错。

推送 `v*` 标签或手动运行工作流时，GitHub Actions 会编译 Windows x64 和 macOS（Apple 芯片）安装包，可在 Actions 页面的构建记录里下载。推送标签时还会发布到 Releases，并附上所含第三方库的源代码。安装包都没有付费签名，首次打开时 Windows SmartScreen 或 macOS 可能会拦截。

分发前请阅读 [THIRD_PARTY.md](THIRD_PARTY.md) 中关于 libmpv 许可（GPL / LGPL）的说明。

## 许可证

Copyright (C) 2026 calabli

TinyPlayer 以 [GNU 通用公共许可证 3.0 或更高版本](LICENSE)（GPL-3.0-or-later）发布，不提供任何担保。第三方组件的许可见 [THIRD_PARTY.md](THIRD_PARTY.md)。
