# TinyPlayer

跨平台（Windows / macOS）本地视频播放器。Rust + egui（OpenGL）+ libmpv，目标是快、稳、占用小。

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

## 快捷键

| 按键 | 功能 |
|---|---|
| Space / K | 播放 / 暂停 |
| ← / → | 快退 / 快进 5 秒（加 Ctrl/⌘ 为 60 秒） |
| ↑ / ↓、滚轮 | 音量 ±5% |
| M | 静音 |
| F / Enter、双击画面 | 全屏；Esc 退出全屏 |
| [ / ] | 减速 / 加速，Backspace 恢复 1.0× |
| C | 切换字幕 |
| N / P、PageDown / PageUp | 下一个 / 上一个 |
| Ctrl/⌘ + O | 打开文件 |
| Ctrl/⌘ + S | 截图到桌面 |
| Home | 回到开头 |
| Enter / Esc | 续播提示出现时：继续播放 / 忽略 |
| Ctrl/⌘ + , | 设置 |
| L | 打开 / 收起播放列表 |

## 播放列表

- 打开单个文件时，自动把同一文件夹的其他媒体文件按**自然顺序**加入列表（「第2讲」排在「第10讲」前面），可在 设置 → 播放 中关闭
- 拖入或打开文件夹 = 播放整个文件夹
- 播完后：播下一个 / 列表循环 / 单个循环；随机播放保证每个文件播一次后才重复
- 侧栏**浮在画面上方，不会缩放视频**。打开方式：暂停后点画面右侧的 ‹，或控制栏的列表按钮，或按 L
- 侧栏内：点击播放、拖动排序、悬停出现 × 移除；鼠标在侧栏上时滚轮滚动列表（不调音量）

列表由程序自己管理（不用 mpv 内置播放列表），以便控制循环、随机、排序等规则。

## 播放记录

数据存在 SQLite：Windows 为 `%APPDATA%\TinyPlayer\tinyplayer.db`，macOS 为 `~/Library/Application Support/TinyPlayer/tinyplayer.db`。
开发测试时可以设置环境变量 `TINYPLAYER_DATA_DIR` 指向别的目录，避免写入真实记录。

| 规则 | 说明 |
|---|---|
| 文件识别 | 路径 + 文件大小；同名但内容换了的文件不会继承旧进度 |
| 何时保存 | 每播放 10 秒、暂停、播完、切换文件、正常退出时 |
| 防误删 | 实际观看不到 5 秒不写入，打开后马上关闭不会覆盖上次的进度 |
| 播放次数 | 实际观看 ≥ 30 秒或时长的 10%（取较小值）才 +1；30 分钟内重复打开只算一次；按真实时间计，倍速不影响 |
| 退出时清除 | 只清空进度和最近播放，**播放次数保留**；启动时也会再清一次，防止上次崩溃没清掉 |
| 无痕模式 | 本次运行不写入任何记录（包括播放次数），重启后自动关闭 |

已知限制：程序被强制结束时，最多丢失最近 10 秒的进度。

## 代码结构

```
src/
  main.rs     窗口配置、命令行参数
  app.rs      播放界面：视频层、底部控制栏、播放列表侧栏、OSD、空状态
  player.rs   播放状态与命令（基于 mpv 属性监听）
  history.rs  SQLite 存储：进度、播放次数、设置
  session.rs  当前文件的会话：何时保存、何时计数、是否续播
  playlist.rs 播放队列：同目录扫描、自然排序、循环与随机
  settings_panel.rs  设置面板（播放 / 播放记录）
  mpv.rs      libmpv 运行时加载的 FFI 封装 + OpenGL 渲染上下文
  video.rs    mpv 渲染目标（FBO → egui 纹理）
  icons.rs    矢量图标（不依赖图标字体或图片）
  theme.rs    配色、系统中文字体
```

渲染流程：mpv 通过 render API 把画面画进 FBO 纹理，egui 把它当普通图片显示，控制栏和侧栏叠在上面。
只有 mpv 通知有新帧，或者有用户输入时才重绘，暂停时不占 CPU。

## 路线图

- [x] 播放内核、硬件解码、底部控制栏（通栏进度条、倍速菜单）、OSD、快捷键、拖放打开
- [x] 播放列表侧栏：同目录自动加入、自然排序、循环/随机、拖动排序
- [x] 播放记录：记住进度、续播方式、退出时清除（保留播放次数）、播放次数统计、最近播放
- [ ] 媒体库：SQLite + FTS5，缩略图、标签、搜索
- [ ] 文件夹视图、收藏夹、播放次数统计
- [ ] 设置页其余分组（播放、画面、字幕、快捷键）
- [ ] macOS 打包（.app、签名公证）
