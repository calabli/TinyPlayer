# TinyPlayer 技术文档

本文面向开发者，说明 TinyPlayer 的架构、模块划分、关键流程、数据存储、构建发布和开发约定。功能行为见 [product.md](product.md)。

## 1. 技术栈

| 层 | 选型 | 说明 |
|---|---|---|
| 语言 | Rust 2024 edition | 单一可执行文件，release 开启 LTO、`panic = "abort"`、strip |
| 界面 | eframe / egui 0.37（glow 后端）+ winit 0.30 | 即时模式 GUI，自绘全部控件和矢量图标，不用图标字体或图片 |
| 渲染 | OpenGL（glow） | mpv 通过 render API 画进离屏 FBO，egui 当纹理显示 |
| 播放内核 | libmpv（运行时用 `libloading` 加载） | 不链接导入库；找不到 libmpv 时显示明确错误 |
| 存储 | SQLite（`rusqlite`，bundled），WAL 模式 | 播放记录、媒体库、设置共用一个文件 |
| 其他依赖 | `image`（jpeg 缩略图）、`memmap2`（字体）、`rfd`（文件对话框）、`sha2`（校验）、`souvlaki`（媒体键）、`trash`（废纸篓） | macOS 额外用 `objc2`，Windows 额外用 `windows`、`raw-window-handle` |
| 网络 | 系统自带 `curl` | 只用于更新，不内置 TLS 栈 |

## 2. 总体架构

```
┌──────────────────────────── 主线程（winit 事件循环 + egui） ────────────────────────────┐
│  App (app.rs)                                                                            │
│   ├─ Player (player.rs) ── Mpv (mpv.rs) ── libmpv（解码线程由 mpv 自己管理）             │
│   ├─ RenderContext + VideoTarget (video.rs) ── FBO 纹理 ──► egui 绘制                    │
│   ├─ Playlist / Session / Marks / AbLoop / SleepTimer（纯逻辑）                          │
│   ├─ MultiView (multiview.rs) ── 最多 3 个额外 Pane，各有 Player + RenderContext        │
│   ├─ LibraryView / Manage (library_ui.rs, library_manage.rs)                             │
│   ├─ SettingsPanel / QuickPanel（覆盖层）                                                │
│   └─ Store (history.rs) + Library (library.rs) ── SQLite                                 │
└──────────────────────────────────────────────────────────────────────────────────────────┘
        ▲ mpsc 通道 + ctx.request_repaint()
        │
后台线程：library-scanner · thumb-loader · hover-thumbs · crop-detect · duplicates ·
          file-move · folder-fill · disk-check · audio-output · single-instance ·
          update-check · update-download
```

原则：

- **主线程只做 UI 和轻量逻辑**。耗时工作都在命名后台线程里，通过 `mpsc` 把结果送回，再调用 `request_repaint()` 唤醒界面。
- **按需重绘**：只有 mpv 报告新帧、有用户输入或后台结果到达时才重绘；暂停时 CPU 为 0。插帧开启时例外，按屏幕刷新率持续重绘。
- **播放优先**：`background.rs` 提供一个全局闸门，播放期间扫描、缩略图、重复检查、跨盘复制都在 `background::wait()` 处等待。

## 3. 启动流程（`main.rs`）

1. 读取命令行中的文件参数；macOS 上安装 Apple Event 处理器（`mac_open.rs`），接收访达打开的文件。
2. 打开数据库，读取 `multi_window` 设置。
3. `single_instance::acquire`：
   - 先尝试绑定本机回环端口（端口由数据目录哈希得出，范围 49200–59199）。绑定成功即为主实例。
   - 端口已被占用且开启多开：作为额外窗口运行（`is_extra()`）。
   - 否则连接已有实例，发送 `TINYPLAYER/1\n` + 每行一个绝对路径，收到 `OK` 后退出。
4. 读取 `remember_window` 和保存的窗口位置（`WindowPlace`，格式 `x,y,w,h,maximized`），不合理的值（窗口在已移除的显示器上等）会被丢弃；没有保存值时用默认 854×480 并居中。额外窗口在主窗口位置基础上错开。
5. `eframe::run_native` 创建窗口（glow 渲染器，无多重采样、深度和模板缓冲），构造 `App`。

## 4. 模块地图

| 模块 | 职责 |
|---|---|
| `main.rs` | 窗口参数、单实例分流、窗口位置的编码与校验 |
| `app.rs` | 播放窗口：视频层、控制栏、侧栏、OSD、空状态、快捷键、拖放、全屏/无边框/画中画切换、退出流程 |
| `player.rs` | 播放状态和命令：mpv 选项、属性监听、设置应用（硬解、增强、插帧、音频滤镜、全景时的快速模式、翻转） |
| `mpv.rs` | libmpv 的最小 FFI：运行时加载、命令/属性/事件、唤醒回调；OpenGL 渲染上下文 |
| `video.rs` | 离屏 FBO 目标；VR 全景时多一遍着色器投影 |
| `vr.rs` | 全景格式识别（文件名标记 + 画面比例）与视角计算 |
| `view.rs` | 缩放、平移、裁剪的几何计算；插帧的刷新率吸附 |
| `session.rs` | 当前文件的会话规则：何时保存进度、何时计一次播放、是否续播 |
| `history.rs` | SQLite：播放记录、播放事件、书签、设置、设备音量；`data_dir()` |
| `playlist.rs` | 播放队列：同目录扫描、自然排序、循环与随机 |
| `marks.rs` | 章节与书签的跳转规则 |
| `ab_loop.rs` | A-B 循环取点 |
| `sleep_timer.rs` | 睡眠定时 |
| `audio_fx.rs` | 音量标准化、均衡器、声道选择，合成一个 mpv `af` 滤镜图 |
| `audio_output.rs` | 跟踪系统默认输出设备（Windows: MMDevice，macOS: CoreAudio），按设备记音量 |
| `crop_detect.rs` | 用无窗口 mpv 检测黑边 |
| `hover_thumbs.rs` | 进度条悬停预览：无窗口 mpv 取关键帧，只处理最新请求，LRU 缓存 |
| `probe.rs` | 无窗口 mpv（`vo=image`）读取时长、分辨率并截缩略图 |
| `scanner.rs` | 媒体库后台扫描与缩略图生成（最多 3 个探测进程） |
| `library.rs` | 媒体库存储：文件夹、视频、标签、收藏夹、集合 |
| `library_ui.rs` | 媒体库首页、文件夹视图；公共的 `elide` 文本截断 |
| `library_manage.rs` | 管理页：集合、列表视图、批量操作、重命名、剪切、重复检查入口 |
| `library_lock.rs` | 媒体库密码（加盐慢哈希） |
| `duplicates.rs` | 按时长、大小、名称模糊比较找可能重复 |
| `file_move.rs` | 后台移动文件：同盘重命名；跨盘复制到临时名、SHA-256 校验后再删除原文件 |
| `background.rs` | 播放期间暂停后台工作的闸门 |
| `multiview.rs` | 多屏同播：额外 Pane、迷你控制条、拖放替换、联动、只听选中；关闭 Pane 时回收视频目标 |
| `split_layout.rs` | 多屏布局树、浮动小窗、预设（纯计算，无绘制） |
| `float_window.rs` | 无边框/画中画窗口的拖动缩放、最小化、跟随桌面空间（macOS 用原生 API） |
| `quick_panel.rs` | 播放时的快捷设置面板 |
| `settings_panel.rs` | 设置窗口 |
| `single_instance.rs` | 单实例与多开窗口 |
| `media_keys.rs` | 系统媒体控制（souvlaki） |
| `keep_awake.rs` | 播放时阻止休眠（Windows `SetThreadExecutionState`，macOS IOPMAssertion） |
| `mac_open.rs` | macOS 打开文件的 Apple Event |
| `update.rs` | 应用内更新 |
| `theme.rs` | 配色、字体（内存映射加载系统中文字体）、egui 样式、浅色主题 |
| `icons.rs` | 矢量图标 |
| `i18n.rs` | 界面语言：英文作键，`locales/zh-CN.tsv` 提供中文 |

## 5. 播放与渲染管线

### 5.1 mpv 封装（`mpv.rs`）

- `Api::load()` 依次在可执行文件旁、`../Frameworks`（macOS .app）和系统路径中查找 libmpv，解析所需符号。存在但加载失败的文件的错误优先报告。
- `Mpv` 拥有句柄与唤醒回调。`Drop` 先调用 `mpv_terminate_destroy`，回调的 Box 作为字段在其后释放，保证 mpv 不会调用已释放的闭包。
- 事件只关心 `FILE_LOADED`、`END_FILE`、`SHUTDOWN` 和 `PROPERTY_CHANGE`；`poll_events` 非阻塞地取空队列。

### 5.2 主要 mpv 选项（`player.rs`）

| 选项 | 值 | 原因 |
|---|---|---|
| `vo` | `libmpv` | 嵌入式渲染，由 render API 输出 |
| `hwdec` | `auto-safe`（翻转时 `auto-copy-safe`） | 硬件解码；翻转用 CPU 滤镜，需要回传模式 |
| `keep-open` / `idle` | `yes` | 播完不退出，由应用决定下一步 |
| `cache` | `auto` | 本地文件不强制预读 |
| `demuxer-max-bytes` / `-back-bytes` | 50 MiB / 20 MiB | 控制内存 |

画质增强：`scale`、`cscale` 为 `ewa_lanczossharp`，加 `deband`。插帧：`video-sync=display-resample`、`interpolation`、`tscale=oversample`，刷新率每 2 秒从 mpv 的测量值吸附到最近的标准值。VR 全景时临时换成接近 mpv `fast` 配置的一组选项，退出后恢复。

### 5.3 渲染（`video.rs`）

1. mpv 的更新回调调用 `ctx.request_repaint()`。
2. 每帧 `VideoTarget::update` 调用 `mpv_render_context_update`；有新帧或尺寸变化时，让 mpv 画进 `Layer`（FBO + 纹理，RGBA8）。
3. 普通视频：直接把该纹理交给 egui（`register_native_glow_texture`），控制栏叠在上面。
4. VR 全景：mpv 以更大的尺寸（不超过 `MAX_TEXTURE_SIZE`，上限 5760）渲染整帧，再由一个全屏三角形的片段着色器按视角投影到窗口大小的第二层。

注意：egui 拥有已注册纹理直到退出，eframe 没有释放接口。因此多屏关闭的 Pane 不删除纹理，而是把 `VideoTarget` 缩到 1×1 放进 `spare` 列表，下一个 Pane 复用。

### 5.4 销毁顺序

`RenderContext` 必须先于其 `Mpv` 释放（`App` 和 `Pane` 中字段顺序保证：`render` 在 `player` 之前声明）。`RenderContext` 只持有 mpv 库的函数指针，依赖同一进程中库未被卸载。

## 6. 多屏同播（`multiview.rs`、`split_layout.rs`）

- 主播放器保留播放列表、历史和侧栏；额外 Pane 各自有独立的 `Player`、`RenderContext`、`VideoTarget` 和 `Tracker`（独立播放记录）。
- `split_layout` 是纯数据：分屏树的叶子是槽位，浮动小窗在其上；负责命中测试、分隔线拖动、交换、预设。
- 每帧调用 `MultiView::reap()`：关闭的 Pane 停止播放、释放渲染上下文、回收视频目标。退出多屏时同样回收所有额外 Pane，避免后台继续解码。
- 联动模式把播放/暂停、跳转、倍速广播给所有 Pane；“只听选中”只让选中的 Pane 出声。

## 7. 数据存储

一个 SQLite 文件 `tinyplayer.db`，`history.rs` 和 `library.rs` 各开一个连接，均为 WAL 模式、`synchronous = NORMAL`、3 秒 busy timeout（多开窗口共享同一文件）。

| 表 | 内容 |
|---|---|
| `history` | 每个文件的进度：`key`（路径 + 大小）、路径、标题、位置、时长、更新时间 |
| `play_events` | 每次有效播放的记录，用于播放次数和最近播放 |
| `bookmarks` | 书签 |
| `settings` | 键值对 `(k, v)`：所有设置、窗口位置（`window`）、媒体库密码哈希等 |
| `device_volumes` | 每个音频输出设备的音量 |
| `folders`、`videos` | 媒体库文件夹和视频（时长、分辨率、缩略图、修改时间、是否在线） |
| `tags`、`video_tags` | 标签 |
| `collections`、`collection_items` | 收藏夹 |
| `folder_sets`、`folder_set_items` | 管理页的文件夹集合 |
| `dup_ignored` | 标记为“不是重复”的组 |

设置的读写：`Settings` 结构体与键值对互转（`settings_pairs` / `load_settings`）。保存时只写改动过的键，多开窗口不会互相覆盖未修改的项。新增设置需要同时改：结构体字段、`Default`、`settings_pairs`、`load_settings`，以及往返测试。

文件识别用“路径 + 文件大小”作为 `key`：同名但内容换了的文件不继承旧进度。

## 8. 系统集成

| 能力 | Windows | macOS |
|---|---|---|
| 打开文件 | 命令行参数；安装版注册文件关联和右键菜单（Inno Setup） | Apple Event（`mac_open.rs`） |
| 单实例 | 本机回环 TCP 端口 | 同左 |
| 媒体键 | souvlaki（需要窗口句柄） | souvlaki（控制中心） |
| 阻止休眠 | `SetThreadExecutionState` | IOPMAssertion |
| 默认输出设备 | MMDevice API | CoreAudio |
| 无边框最小化 | `ViewportCommand::Minimized` | 临时加上 Miniaturizable 样式后 `miniaturize:` |

## 9. 应用内更新（`update.rs`）

1. **检查**：请求 `releases/latest`，从重定向的 `Location` 读出版本号（不用 GitHub API，避开每小时 60 次的限额）。版本相同时比较 `COMMIT` 文件与编译时的 `GITHUB_SHA`；本地编译只比版本号。最多每天一次。
2. **说明**：从对应语言的 `CHANGELOG*.md` 读取该版本的条目（最多 8 行），所以 CHANGELOG 每条都写给用户看。
3. **下载**：`curl` 下载 `SHA256SUMS` 和安装包到 `data_dir/updates/`，SHA-256 不一致就删除。macOS 挂载 dmg、`ditto` 复制出 .app 并 `codesign --verify`。
4. **安装**：写一个小脚本，等当前进程退出后替换。macOS 先把旧 .app 改名，复制失败就还原；Windows 静默运行安装程序。
5. 便携版、从 dmg 直接运行、AppTranslocation、不可写的位置不支持自更新，只提示。

## 10. 构建、打包与发布

### 本地构建

```bash
cargo run --release -- "/path/to/video.mp4"
```

`build.rs` 会把 `third_party/mpv/<平台>/` 下的 libmpv 复制到可执行文件旁边（Windows：`libmpv-2.dll`，来自 shinchiro/mpv-winbuild-cmake 的 `mpv-dev-x86_64` 包；macOS：`libmpv.2.dylib`，可从 Homebrew 复制）。libmpv 体积大，不提交到 git。Windows 上 `build.rs` 还嵌入 exe 图标。

### 打包

| 平台 | 命令 | 产物 |
|---|---|---|
| Windows | `powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1` | 便携 zip；装了 Inno Setup 时另有 `-setup.exe` |
| macOS | `scripts/package-macos.sh`（需要 `brew install mpv dylibbundler`） | `.app` 与 `.dmg`；设置 `SIGN_IDENTITY`、`NOTARY_PROFILE` 可签名公证 |

macOS 脚本用 dylibbundler 把 libmpv 的依赖一起打进 `Contents/Frameworks`，最后逐个加载包内的库，有失败或仍依赖 Homebrew 就报错。最低系统版本由打包时的依赖库决定，自动写入 `Info.plist`。图标由 `python scripts/make-icon.py` 生成（只用标准库）。

### CI（`.github/workflows/`）

| 工作流 | 触发 | 作用 |
|---|---|---|
| `build.yml` | 推送 `v*` 标签、手动运行 | macOS arm64 与 Windows x64：单元测试、Lint、Windows 上用真实视频加载 libmpv、打包、试装和卸载安装程序、收集第三方源码；标签时发布 Release 并写 `SHA256SUMS`、`COMMIT` |
| `cache.yml` | 推送到 main（源码或依赖变化时）、手动运行 | 预热 Rust 编译缓存。标签构建只能读取 main 分支保存的缓存，所以由它在 main 上填充 |

### 发布步骤

1. 更新 `Cargo.toml` 版本（`Cargo.lock` 随之更新）。
2. 在 `CHANGELOG.md` 和 `CHANGELOG.en.md` 顶部添加该版本的用户向条目。
3. 提交 `Release x.y.z`，打带注释的标签 `vx.y.z`，推送 main 和标签。
4. 等 `build.yml` 完成，检查 Release 附件：dmg、setup.exe、zip、`SHA256SUMS`、`COMMIT`、第三方源码包。

## 11. 排错

| 现象 | 排查 |
|---|---|
| 启动提示找不到 libmpv | 确认 `third_party/mpv/<平台>/` 下有库，或系统路径中有 libmpv |
| 想用干净数据测试 | 设置 `TINYPLAYER_DATA_DIR=/tmp/tp-test` |
| Windows 整机卡死 / 黑屏 | 多为 GPU 驱动超时（TDR）。依次关闭硬件解码、画质增强、插帧，避免多屏，看是否仍复现；查看事件查看器“系统”日志中的事件 4101（显示驱动程序停止响应）和 41（Kernel-Power）；记录显卡型号和驱动版本。播放时 OSD 的 `HW · <解码方式>` 显示实际使用的硬件解码 |
| 内存或显存持续上涨 | 0.1.26 起，多屏关闭的 Pane 会释放显存并停止解码；如仍上涨，记录多屏操作步骤 |

## 12. 开发约定

- **源码不含中文**：界面文字用英文写在代码里作为键，`tr("…")` / `tf("…{}…", &[..])` 取译文，中文放在 `locales/zh-CN.tsv`（英文键、制表符、中文）。源文件出现中文、或某个键缺少中文时测试失败。英文相同但中文不同的文字在键后加 `|语境`（如 `Off|short`），界面上不显示。
- **测试**：`cargo test`（约 140 个单元测试，覆盖播放规则、设置往返、布局、更新解析、文件移动等）；`cargo clippy --all-targets` 保持零警告。
- **注释**：模块顶部 `//!` 说明职责和关键取舍；注释写“为什么”，不复述代码。
- **提交信息**：用一句话描述用户能感知的变化（如“Free multi-view panes and their video memory when they close”）。
- **CHANGELOG**：中英文同步，每条写给用户看，会显示在应用内更新对话框中。
