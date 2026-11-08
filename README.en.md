# TinyPlayer

English | [简体中文](README.md)

A local video player for Windows and macOS, built with Rust, egui (OpenGL) and libmpv. It aims to be fast, stable and light.

## Packaging

| Platform | Command | Output |
|---|---|---|
| Windows | `powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1` | `dist\TinyPlayer-<version>-windows-x64.zip` (portable: unzip and run, about 48 MB). With [Inno Setup](https://jrsoftware.org/isinfo.php) installed, also a `-setup.exe` installer that needs no administrator rights |
| macOS | `scripts/package-macos.sh` (needs `brew install mpv dylibbundler` and the Xcode command line tools) | `dist/TinyPlayer.app` and `dist/TinyPlayer-<version>-macos-<arch>.dmg` (about 32 MB; open it and drag TinyPlayer to Applications). Set `SIGN_IDENTITY` and `NOTARY_PROFILE` to sign and notarize |

The lowest macOS version the package runs on depends on its libraries, and the script writes it into `Info.plist`. With Homebrew's libmpv it's usually the system version of the machine that built it.

By default the macOS script only signs ad hoc, which runs fine on your own Mac. Someone you send it to will see it blocked the first time: they open it once, then click "Open Anyway" in System Settings → Privacy & Security. A package signed with a Developer ID and notarized isn't blocked. The package only runs on the architecture it was built on. At the end the script loads every library in the package, and fails if one won't load or still depends on Homebrew.

GitHub Actions builds the Windows x64 and macOS (Apple silicon) packages when a `v*` tag is pushed, or when the workflow is run by hand; the packages can be downloaded from the run on the Actions page. A tag also publishes them on Releases, together with the source of the bundled third-party libraries. The packages aren't signed with a paid certificate, so Windows SmartScreen or macOS may block them the first time they're opened.

Before distributing TinyPlayer, read about libmpv's license (GPL / LGPL) in [THIRD_PARTY.en.md](THIRD_PARTY.en.md).

## Building and running

```bash
cargo run --release -- "D:/path/to/video.mp4"
```

The build script copies libmpv from `third_party/mpv/<platform>/` next to the executable:

| Platform | File needed |
|---|---|
| Windows | `third_party/mpv/windows/libmpv-2.dll` (from the `mpv-dev-x86_64` package of [shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake/releases)) |
| macOS | `third_party/mpv/macos/libmpv.2.dylib` (copy it from Homebrew after `brew install mpv`, and bundle its libraries with it) |

libmpv is large, so it isn't committed to git.

## Keyboard shortcuts

| Key | Action |
|---|---|
| Space / K | Play / pause |
| ← / → | Back / forward 5 seconds (60 seconds with Ctrl/⌘) |
| ↑ / ↓, mouse wheel | Volume ±5% |
| M | Mute |
| Click the video | Play / pause |
| F / Enter, double-click the video | Full screen; Esc leaves it |
| [ / ] | Slower / faster (0.25× to 4×); with Shift, fine steps of ±0.05×; Backspace returns to 1.0× |
| C | Switch subtitles |
| N / P, PageDown / PageUp | Next / previous |
| Ctrl/⌘ + O | Open a file |
| Ctrl/⌘ + S | Screenshot to the desktop |
| Home | Back to the start |
| Enter / Esc | When the resume prompt shows: resume / dismiss |
| Ctrl/⌘ + , | Settings |
| Ctrl/⌘ + D | Add the current video to favorites, or remove it |
| Ctrl/⌘ + K | Search the library |
| L | Show / hide the playlist |
| , / . | Previous / next frame (pauses) |
| = / - / 0, Ctrl/⌘ + wheel | Zoom in / out / reset (drag the video to pan when zoomed) |
| Shift + ← / → | Previous / next chapter |
| G | Show / hide chapters and bookmarks |
| Ctrl/⌘ + B | Add a bookmark |
| B | A-B loop: set A, set B, clear, in turn |
| V | VR panorama on / off |
| T | Keep the window on top |
| I | Picture in picture; Esc or a double-click leaves it |
| H | Borderless playback; Esc leaves it |
| Ctrl/⌘ + Shift + P | Private mode |

## Performance

Measured with a release build on a Windows machine with 16 cores and an NVIDIA graphics card (1440p H.264, NVDEC hardware decoding):

| Case | CPU | Memory (Task Manager's "Memory" column) |
|---|---|---|
| Start until the window shows | — | about 0.47 s |
| Idle (empty start page) | 0% | about 94 MB |
| Playing 1440p | about 2–16% of one core (under 1% of the machine) | about 173 MB |
| Paused / finished | 0% | about 138 MB |
| Scrolling a library of 400 videos end to end | — | 109 → 112 MB |

Optimizations so far: redraw only on a new frame or on input; the single-instance check claims its port before connecting (about 300 ms off a cold start); Chinese fonts are memory-mapped (about −19 MB); local files aren't forced into the read-ahead cache (`cache=auto`); the mpv instance that makes thumbnails is released after use (about −130 MB for a large library); at most 160 thumbnail textures are kept, and the rest load again when scrolled to.

## Desktop integration

- **Remembers the window**: its position, size and whether it's maximized are saved on exit and restored at the next start (full screen isn't saved)
- **Single instance**: while TinyPlayer runs, a video opened again (double-click, dropped on the exe, command line) plays in the existing window, which comes to the front, instead of in a new one. This works over a local loopback port derived from the data folder, so instances with a different `TINYPLAYER_DATA_DIR` (tests, the portable version) don't interfere
- **File associations** (Windows installer): every supported format can choose TinyPlayer in "Open with" and in the system's default apps settings, and the context menu has "Play with TinyPlayer" (on Windows 11, under "Show more options"). Windows doesn't let an app make itself the default player; to make it the default, tick "Always" in "Open with". Uninstalling removes all of this. The portable zip registers none of it
- **Open with** (macOS): double-click in Finder or choose TinyPlayer in "Open With"; if TinyPlayer is already running, the existing window plays it
- **Icon**: `python scripts/make-icon.py` makes the icons in `assets/` with the standard library only. Windows builds embed it in the exe and use it at the window's top-left; the macOS packaging script turns it into `.icns`
- **Media keys**: keyboard media keys, macOS Control Center and the Windows volume flyout can play, pause and skip to the previous or next video, and show the current title and progress
- **Keeps the computer awake**: while a video plays the screen stays on; with audio only the system stays awake but the screen may turn off. Normal behavior returns on pause or at the end
- **Volume per device**: headphones, speakers and other outputs each keep their own volume, and switching the system's output switches to that device's volume. A device used for the first time keeps the current volume. Turn this off, or view, change and delete each device's volume, in Settings → Sound
- **Borderless playback**: hides the window's title bar during playback, so once the controls hide only the video is left. Toggle it with H or in the quick panel's "Enhance" page; the choice is saved. Drag the video to move the window; move the mouse to the top for a close button at the top-right. The title bar comes back in the library or in full screen
- **VR panorama**: watch 180° and 360° equirectangular videos on an ordinary screen. Drag to look around, use the wheel or = / - to zoom, and 0 to face forward. Side-by-side and top-bottom 3D videos show the left eye only. It turns on by itself when the file name has a mark such as `360`, `180`, `VR`, `SBS` or `TB` and the picture's shape matches; it can also be set with V or on the quick panel's "VR" page, and each file remembers its own choice (not saved in private mode). In panorama mode subtitles don't show, and quality enhancement and frame interpolation pause until you leave it; a screenshot saves the whole panorama. VR headsets and fisheye formats aren't supported
- **Picture in picture**: a small borderless window kept on top at the screen's bottom-right. Drag the video to move it, and drag an edge or a corner to resize it in the video's shape; it opens at the last size next time. Leaving it returns the window to where it was. Leave full screen first to use it. Keep on top and picture in picture only last until the app quits
- **Subtitles make room**: while the control bar shows, subtitles move up above the progress bar, and go back down when it hides

## Quick settings panel

During playback, the settings button on the control bar opens this panel at the bottom-right of the video:

| Page | Contents |
|---|---|
| Subtitles | Subtitle track (or none), load a subtitle file, size, position, delay ±0.5 s |
| Subtitle style | Second subtitle (dual subtitles), font, color, outline, shadow, override ASS styles |
| Sound | Audio track, channels (stereo / left / right / mix), audio delay ±0.1 s, volume ±5% / mute, volume normalization |
| Equalizer | 10 bands (31 Hz to 16 kHz, ±12 dB); presets: Off / Bass / Voice / Treble / Cinema |
| Picture | Aspect ratio (original / 4:3 / 16:9 / 2.35:1 / stretch to window / 21:9), rotate left / right, mirror / flip, hardware decoding, quality enhancement (debanding and sharper scaling, more GPU work), reset |
| Enhance | Crop (black bars / 16:9 / 4:3 / 2.35:1 / 1:1), zoom, HDR tone mapping, frame interpolation |
| Color | Brightness, contrast, saturation, gamma, hue (-100 to +100, snapping to 0 near it), reset |

Aspect ratio, hardware decoding, quality enhancement, subtitle size, position and style, volume normalization and the equalizer are saved, and so are HDR tone mapping and frame interpolation. The second subtitle and the channels last until the app quits; crop and zoom apply to the current file only; rotation, flipping, color and delays last until the app quits. Flipping uses a video filter on the CPU, so while it's on hardware decoding switches to copy-back mode (`auto-copy-safe`), which costs a little more; turning it off switches back. Click outside the panel or press Esc to close it; "More settings" opens the full settings window (also Ctrl/⌘+,).

## Interface

Settings → Interface switches the theme and the language, at once, and saves them:

- **Theme**: dark, light, or follow the system. It affects the library, the start page and the settings window; the control bar, sidebar and quick panel sit on the video and stay dark
- **In-app updates**: at most once a day after starting, TinyPlayer asks GitHub for a new version. With the same version number it also compares the build's commit, and offers the update if they differ; a build newer than the latest release isn't offered one, and a local build only compares version numbers. The update downloads and installs inside the app, with no browser. By default it downloads in the background and then asks "Restart now / Later"; "Later" installs it on exit. With automatic download off, a new version asks "Update now / Remind me later / Skip this version", and the download can be canceled. Nothing pops up while a video plays. A download installs only if it matches the published `SHA256SUMS`, and a failed replacement restores the old version. Turn checks off or check by hand in Settings → Interface; private mode doesn't check by itself. Only the installed version updates; the portable zip and an app run straight from the DMG only say there's a new version. It uses the system's own `curl` (macOS and Windows 10/11 both have it)
- **Language**: Chinese (the default) or English. Some playback messages, such as `VOLUME 60%`, read the same in both. The update dialog shows the changelog in the chosen language

## Playlist

- Opening a single file adds the other media files of its folder in **natural order** ("Lesson 2" before "Lesson 10"); turn this off in Settings → Playback. **Playback starts at once, and the folder is scanned in the background**, then merged into the list, so it never delays the start (a folder of 20,000 files starts playing right away). If the list is changed by hand or another file is opened during the scan, its result is dropped
- Dropping or opening a folder plays the whole folder
- At the end: play the next one / loop the list / loop one; shuffle plays every file once before repeating
- The sidebar **floats over the video and never resizes it**. Open it with the ‹ at the video's right edge while paused, the list button on the control bar, or L
- In the sidebar: click to play, drag to reorder, hover for an × to remove; the wheel over the sidebar scrolls the list instead of changing the volume

TinyPlayer keeps the list itself (not mpv's built-in playlist), so it controls looping, shuffling and ordering.

## Multi-view

During playback, the **Multi-view** switch at the right of the top bar plays up to 4 videos in one window:

- **All with sound, each on its own**: every slot is its own player, with its own play/pause, progress, volume (new slots start at 50%) and history
- **Click a video to play or pause it**. **Double-click it, press F, or click the full-screen button at the right end of its control bar to put that video alone on the whole screen**, while the others play on in the background; double-click, Esc or the exit button goes back to multi-view. The enlarge button in a slot's title bar enlarges it inside the window only. Click an empty slot to add a video from "Open file…" or the playlist, or drop a file straight onto a slot: while dragging, the slot it will replace is outlined, and on release it plays there, replacing the video that was in it. The **×** at the right end of a slot's title bar closes its video and brings back the empty "Add a video" screen, keeping the layout; the × on an empty slot removes the slot itself
- **Flexible layouts**: presets Single / Left-right / Top-bottom / Grid / 1 + 3 / Picture in picture. The buttons in each title bar split the slot again left-right or top-bottom, turn it into a small floating window (drag to move, drag the bottom-right corner to resize) or put it back in the layout. Drag a divider to change the split, and drag a title onto another slot to swap them
- **Hiding the progress bars**: each slot's control bar and title bar show only while the mouse is over that slot, and hide about 2 seconds after the mouse stops (they stay while paused); in full screen the pointer hides too. With the top bar's **Progress bar** switch off, a control bar shows only when the mouse is at the bottom of its slot
- **Leaving multi-view**: not possible while several videos are playing; close the others with × first. With one video left, turning **Multi-view** off makes it ordinary single-video playback, carrying on from where it was
- **Link**: when on, play/pause, seeking and speed apply to every slot; volumes stay separate
- In multi-view the keyboard works on the selected slot (blue outline), and Tab moves on; Space, ←/→, ↑/↓ and M work as with one video. The big control bar and the sidebar hide, and each slot has a small control bar of its own
- Measured with 4 videos of 1440p (hardware decoding) at once: about 12% of one core, about 590 MB of memory

## Picture enhancement

- **Removing black bars**: a windowless libmpv instance looks at about 1 second of the video at 20% to find black bars, without touching the playing video; it doesn't crop if that part is all black or the bars are under 2%
- **HDR tone mapping**: maps HDR pictures to the display's range; it does nothing to non-HDR videos
- **Frame interpolation**: blends neighboring frames for smoother motion; off by default. When on, the window redraws at the display's refresh rate the whole time, using more CPU/GPU and more power. The render API doesn't give the refresh rate, so every 2 seconds TinyPlayer reads mpv's measurement and snaps it to the nearest standard rate; moving the window to another display follows along

## Sound effects

Volume normalization narrows the gap between loud and quiet parts with FFmpeg's `dynaudnorm` (RMS mode). It looks about 1 second ahead, so that much delay applies only while it's on. When the equalizer raises a band, it first lowers the overall volume by the largest boost to prevent clipping, so with it on things may sound quieter overall.

## Chapters and bookmarks

- **Chapters**: files with chapters show marks on the progress bar, with the chapter's name on hover. Previous chapter goes to the one before within 3 seconds of a chapter's start, and to the start of the current one after that
- **Bookmarks**: Ctrl/⌘+B adds one at the current position, shown as a dot on the progress bar; a second one within 1 second isn't added. Like play counts, bookmarks stay when history is cleared; private mode doesn't save them. They work for local files only, and no longer match once the file's size changes
- G or the sidebar's MARKS tab lists chapters and bookmarks in time order; click one to jump there, and hover a bookmark for an × to delete it

## Progress bar preview

Hovering over or dragging the progress bar shows the picture at that point. A windowless libmpv instance fetches the nearest keyframe in the background, handling only the latest request; up to 24 pictures are kept per file, and the instance is released after 20 idle seconds. Network addresses get no preview.

With an A-B loop set, the progress bar marks the loop in yellow; opening another file clears it.

## Library

When nothing is playing, the library's home page shows. After "+ Add folder", the folder is scanned recursively in the background: a windowless libmpv instance (`vo=image`) reads each video's length and resolution and takes the picture at 10% as its thumbnail (480 pixels wide, about 0.2 seconds each). Every start scans again to find new or changed files.

- Left navigation: All / Continue watching / Recently added (within 7 days), tag filters (several at once, all must match), and folder filters; hover a folder for an × to remove it from the library (the files stay)
- Start page: the app opens on the player page; "Open library" or the player page's "Library" goes to the library
- Library password: once turned on in Settings → Library, the library asks for the password, again after every start, and "Lock" locks it at once. Only a salted hash is saved; it guards the way in and doesn't encrypt videos or thumbnails. A forgotten password can only be cleared together with the library
- Clearing the library: Settings → Library removes every folder, tag, favorite and folder set in one go, without deleting video files or clearing history
- Playback first: while a video is open, scanning, thumbnails, the duplicate check and copying across disks all pause, and carry on back in the library
- Large libraries: a progress bar at the bottom shows scanning and processing progress with the time left. Up to 3 background processes make thumbnails, and an interrupted run carries on next time. The interface stays smooth with 20,000 videos
- Manage page: "Manage" at the top. Group folders into folder sets; the list shows thumbnails, length, size and modified date, and sorts by the column you click; search matches folders and videos; click a video to play it, and select several with the check boxes or Ctrl/⌘ + click
- Renaming and batch actions: right-click a video to rename it, keeping its progress, favorites and tags; select several to tag them or add them to favorites together
- External drives: an unplugged drive shows as disconnected and keeps its records, and comes back by itself when plugged in again
- Cut: move videos or whole folders elsewhere in the library, and create a folder while choosing where. A name clash renames the moved one and keeps both; progress, play counts, tags, bookmarks and VR settings all stay. Across disks, files are copied and checked first and the originals deleted only then; canceling leaves the originals as they were. A finished move can be undone
- Possible duplicates: can be limited to one folder and its subfolders (right-click a folder in the Folders view or the left folder list, or open a folder on the Manage page and use the button or the right-click menu). Videos are compared loosely by length, size and name, without reading the files; choose what to compare and how strictly. Results are only suggestions: everything is kept until you mark it, and only marked videos go to the system trash. "Not duplicates" stops showing that group
- Search: matches titles, tags and paths, with words separated by spaces; Ctrl/⌘+K to focus, Esc to clear
- Sorting: recently added / recently played / name (natural order) / length
- Cards: thumbnail, resolution badge, length, progress, play count and tags; "+ Tag" adds one, and the right-click menu removes tags, shows the file in Explorer / Finder, or removes it from the library
- "‹ Library" at the player's top-left goes back to the home page
- **Favorites**: hover a card and click the star at its top-right to choose which collections it goes in (several are fine, and a new one can be made right there); "+" next to COLLECTIONS on the left makes one, and right-click deletes one. During playback, the star next to the title or Ctrl/⌘+D adds or removes the video quickly (in the first collection). Only videos in the library can be favorites
- **Most played**: ranked by play count, with the top three highlighted; any view can also sort by play count
- **Folders view**: switch "Library / Folders" in the top bar, with a breadcrumb and ↑ to go up. Subfolders show as cards with four covers (videos, total length, how many are finished); "Play all", or right-clicking a folder card, plays all its videos in natural order

Search uses `LIKE` rather than FTS5: SQLite's full-text tokenizer doesn't split Chinese, so a two-character word wouldn't match inside a longer title. Filtering a few thousand rows in memory takes under 1 ms.

Thumbnails are kept in `thumbs/` in the data folder.

## History

Data is kept in SQLite: `%APPDATA%\TinyPlayer\tinyplayer.db` on Windows, `~/Library/Application Support/TinyPlayer/tinyplayer.db` on macOS.
For development and testing, point the environment variable `TINYPLAYER_DATA_DIR` at another folder to keep real records out of it.

| Rule | Details |
|---|---|
| Telling files apart | Path + file size; a file replaced under the same name doesn't take over the old progress |
| When it saves | Every 10 seconds of playback, on pause, at the end, when switching files, and on a normal exit |
| Against accidents | Under 5 seconds actually watched isn't saved, so opening and closing at once doesn't overwrite the last position |
| Play counts | +1 after at least 30 seconds or 10% of the length watched, whichever is less; opening again within 30 minutes counts once; real time counts, so speed doesn't matter |
| Clearing on exit | Clears progress and recently played only; **play counts stay**. It clears again at start, in case a crash skipped it |
| Private mode | Ctrl/⌘+Shift+P or in settings. Nothing is written to history (play counts included) while it's on; continue watching, recently played and progress are hidden; the system's media controls show TinyPlayer instead of the title; PRIVATE shows at the top. It turns off on restart |

Known limit: if the app is killed, up to the last 10 seconds of progress may be lost.

## Code layout

```
src/
  main.rs     window setup, command line
  app.rs      player: video layer, control bar, playlist sidebar, OSD, start page
  player.rs   playback state and commands (from mpv property changes)
  history.rs  SQLite storage: progress, play counts, bookmarks, settings
  library.rs  library storage: folders, videos, tags
  library_ui.rs library home page
  library_manage.rs library Manage page: folder sets, moves, duplicate check
  scanner.rs  background scanning and thumbnail threads
  hover_thumbs.rs progress bar preview (background frames and cache)
  marks.rs    jumping between chapters and bookmarks
  view.rs     zoom and pan, crop math, refresh rate snapping
  crop_detect.rs background black bar detection
  audio_fx.rs filters for volume normalization, the equalizer and channels
  audio_output.rs follows the system's output device, volume per device
  ab_loop.rs  A-B loop points
  media_keys.rs system media keys and now-playing info
  keep_awake.rs keeps the system awake during playback
  probe.rs    windowless libmpv: length/resolution, thumbnails
  session.rs  the current file's session: when to save, when to count, whether to resume
  playlist.rs play queue: folder scan, natural order, loop and shuffle
  multiview.rs multi-view: extra players, per-slot controls, dragging and linking
  split_layout.rs multi-view layout: split tree, floating windows, presets (pure math)
  settings_panel.rs  full settings window
  quick_panel.rs     quick settings panel during playback
  single_instance.rs single instance: hands later files to the running window
  mac_open.rs files opened from the macOS Finder
  mpv.rs      FFI for libmpv loaded at runtime + OpenGL render context
  video.rs    mpv render target (FBO → egui texture)
  icons.rs    vector icons (no icon font or images)
  theme.rs    colors, system CJK fonts, light theme
  update.rs   in-app updates: check, download, verify, install
  i18n.rs     interface language: English text in the code is the key, `tr` / `tf` translate it
locales/
  zh-CN.tsv   Chinese translations: English key, a tab, Chinese
```

The code itself has no Chinese: a test fails if a source file contains any, and another if a translated key has no Chinese entry. Keys that read the same in English but differ in Chinese carry a context after `|` (for example `Off|short`), which never shows.

Rendering: mpv draws into an FBO texture through its render API, egui shows that as an ordinary image, and the control bar and sidebar sit on top.
The window redraws only when mpv has a new frame or there's input, so a paused video uses no CPU.

## Roadmap

- [x] Playback core, hardware decoding, control bar (full-width progress bar, speed menu), OSD, shortcuts, drag and drop
- [x] Playlist sidebar: same-folder files, natural order, loop/shuffle, drag to reorder
- [x] History: remembers progress, resume options, clearing on exit (keeping play counts), play counts, recently played
- [x] Library: folder scanning, thumbnails, tags, search, sorting, continue watching
- [x] Folders view (breadcrumb), favorites, most played
- [x] Settings: playback, picture (hardware decoding, aspect ratio), subtitles (auto-load, size, position, language), history, shortcuts
- [x] Windows portable package
- [x] macOS package: .app, bundled libraries, ad hoc signing and dmg verified on an Apple silicon Mac
- [ ] macOS notarization: the script is ready, a Developer ID certificate is needed

## License

Copyright (C) 2026 calabli

TinyPlayer is released under the [GNU General Public License version 3 or later](LICENSE) (GPL-3.0-or-later), without any warranty. Third-party licenses are in [THIRD_PARTY.en.md](THIRD_PARTY.en.md).
