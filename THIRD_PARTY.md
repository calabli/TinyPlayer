# 第三方组件

TinyPlayer 使用了以下开源组件：

| 组件 | 用途 | 许可 |
|---|---|---|
| [mpv / libmpv](https://mpv.io) | 播放内核 | LGPL-2.1+ 或 GPL-2.0+（取决于编译选项） |
| [FFmpeg](https://ffmpeg.org)（随 libmpv 打包） | 解码 | LGPL-2.1+ 或 GPL-2.0+（取决于编译选项） |
| [egui / eframe](https://github.com/emilk/egui) | 界面 | MIT 或 Apache-2.0 |
| [winit](https://github.com/rust-windowing/winit)、[glutin](https://github.com/rust-windowing/glutin)、[glow](https://github.com/grovesNL/glow) | 窗口与 OpenGL | MIT 或 Apache-2.0 |
| [rusqlite](https://github.com/rusqlite/rusqlite) | SQLite 绑定 | MIT |
| [SQLite](https://sqlite.org) | 本地数据库 | 公有领域 |
| [image](https://github.com/image-rs/image)、[zune-jpeg](https://github.com/etemesi254/zune-image) | 缩略图解码 | MIT 或 Apache-2.0 |
| [rfd](https://github.com/PolyMeilex/rfd) | 文件选择对话框 | MIT |
| [libloading](https://github.com/nagisa/rust_libloading) | 运行时加载 libmpv | ISC |
| [souvlaki](https://github.com/Sinono3/souvlaki) | 系统媒体键 | MIT |
| [sha2](https://github.com/RustCrypto/hashes) | 校验更新文件 | MIT 或 Apache-2.0 |
| [Inno Setup](https://jrsoftware.org/isinfo.php) | Windows 安装程序（`scripts/ChineseSimplified.isl` 来自其 6.7.1 版，由 Kira 维护的社区译文） | Inno Setup 许可 |

egui 自带的界面字体随程序内嵌：Ubuntu-Light（Ubuntu Font Licence 1.0）、Hack（MIT 和 Bitstream Vera 许可）、Noto Emoji（SIL OFL 1.1）、emoji-icon-font 及其子集 egui-icons（MIT）。其余 Rust 依赖的许可证均为 MIT、Apache-2.0、BSD、ISC 或 Zlib 这类宽松许可，完整列表可用 `cargo metadata` 查看。

## 关于 libmpv 的许可

TinyPlayer 自身以 [GPL-3.0-or-later](LICENSE) 发布。

常见的预编译 libmpv（如 shinchiro/mpv-winbuild-cmake、Homebrew 的 mpv）默认按 **GPL** 编译。Homebrew 的 FFmpeg 带 `--enable-gpl --enable-version3`，因此 macOS 安装包整体受 GPLv3 约束。
随附这样的 libmpv 分发 TinyPlayer 时，需要遵守 GPL 的要求（例如提供源代码，包括 libmpv 和 FFmpeg 对应版本的源代码）。

如果需要以闭源方式分发，请使用以 `-Dgpl=false` 编译的 LGPL 版 libmpv（FFmpeg 也需按 LGPL 编译），
并保持动态链接（TinyPlayer 本来就是在运行时加载 libmpv 的）。
