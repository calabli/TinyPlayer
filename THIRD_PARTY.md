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

## 关于 libmpv 的许可

常见的预编译 libmpv（如 shinchiro/mpv-winbuild-cmake、Homebrew 的 mpv）默认按 **GPL** 编译。
随附这样的 libmpv 分发 TinyPlayer 时，需要遵守 GPL 的要求（例如提供源代码）。

如果需要以闭源方式分发，请使用以 `-Dgpl=false` 编译的 LGPL 版 libmpv（FFmpeg 也需按 LGPL 编译），
并保持动态链接（TinyPlayer 本来就是在运行时加载 libmpv 的）。
