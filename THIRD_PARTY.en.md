# Third-party components

English | [简体中文](THIRD_PARTY.md)

TinyPlayer uses these open source components:

| Component | Used for | License |
|---|---|---|
| [mpv / libmpv](https://mpv.io) | Playback core | LGPL-2.1+ or GPL-2.0+ (depending on build options) |
| [FFmpeg](https://ffmpeg.org) (bundled with libmpv) | Decoding | LGPL-2.1+ or GPL-2.0+ (depending on build options) |
| [egui / eframe](https://github.com/emilk/egui) | Interface | MIT or Apache-2.0 |
| [winit](https://github.com/rust-windowing/winit), [glutin](https://github.com/rust-windowing/glutin), [glow](https://github.com/grovesNL/glow) | Windows and OpenGL | MIT or Apache-2.0 |
| [rusqlite](https://github.com/rusqlite/rusqlite) | SQLite bindings | MIT |
| [SQLite](https://sqlite.org) | Local database | Public domain |
| [image](https://github.com/image-rs/image), [zune-jpeg](https://github.com/etemesi254/zune-image) | Thumbnail decoding | MIT or Apache-2.0 |
| [rfd](https://github.com/PolyMeilex/rfd) | File dialogs | MIT |
| [libloading](https://github.com/nagisa/rust_libloading) | Loading libmpv at runtime | ISC |
| [souvlaki](https://github.com/Sinono3/souvlaki) | System media keys | MIT |
| [sha2](https://github.com/RustCrypto/hashes) | Verifying update files | MIT or Apache-2.0 |
| [Inno Setup](https://jrsoftware.org/isinfo.php) | Windows installer (`scripts/ChineseSimplified.isl` comes from its version 6.7.1, a community translation maintained by Kira) | Inno Setup License |

egui's own interface fonts are embedded in the app: Ubuntu-Light (Ubuntu Font Licence 1.0), Hack (MIT and Bitstream Vera licenses), Noto Emoji (SIL OFL 1.1), and emoji-icon-font with its subset egui-icons (MIT). The other Rust dependencies use permissive licenses such as MIT, Apache-2.0, BSD, ISC or Zlib; `cargo metadata` lists them all.

## About libmpv's license

TinyPlayer itself is released under [GPL-3.0-or-later](LICENSE).

Common prebuilt libmpv builds (such as shinchiro/mpv-winbuild-cmake and Homebrew's mpv) are built as **GPL** by default. Homebrew's FFmpeg is built with `--enable-gpl --enable-version3`, so the macOS package as a whole falls under GPLv3.
Distributing TinyPlayer with such a libmpv means following the GPL, for example by providing the source code, including that of the matching libmpv and FFmpeg versions.

## Third-party source code

Since 0.1.14, every release comes with the source of the third-party libraries it contains:

- `third-party-sources-<version>-macos-arm64.tar`: the source of every Homebrew library in the package, the patches and resources it's built with, and the formula that says how it's built. Made by `scripts/collect-sources-macos.py`.
- `third-party-sources-<version>-windows-x64.tar`: the mpv and FFmpeg source matching libmpv-2.dll, and the build scripts of [shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake). The build scripts list the other libraries statically linked into the DLL and where they come from. Made by `scripts/collect-sources-windows.ps1`.

`SOURCES.txt` in each archive records the versions. Most of the other libraries in the Windows DLL (including GPL libraries such as x264) are built from their upstream's latest commit at build time, and the builder doesn't record which commit, so their exact versions can't be determined for now.

To distribute TinyPlayer as closed source, use an LGPL libmpv built with `-Dgpl=false` (with FFmpeg built as LGPL too),
and keep it dynamically linked (TinyPlayer already loads libmpv at runtime).
