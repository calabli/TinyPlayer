# Changelog

English | [简体中文](CHANGELOG.md)

The in-app update dialog shows the entries of the new version, so each one is written for users.

## 0.1.23

- The library is much faster: with a large library (tens of thousands of videos) and many favorites, favoriting, tagging and coming back to the library after playback no longer freeze for seconds
- Thumbnails load in the background, so scrolling the library is smoother
- The move dialog is smoother, even with many folders or a target on an external drive
- Multi-view uses less with a very long playlist

## 0.1.22

- New sleep timer: on the quick panel's "Timer" page, pick 15 to 90 minutes to pause by itself, or stop when the current video ends
- New "Focus sound" switch in multi-view: only the selected video has sound and the others are muted; the sound follows the selection
- In multi-view, the , and . keys step the selected video frame by frame
- Fixed the "Add a video" menu of an empty multi-view slot not appearing where it was clicked

## 0.1.21

- In multi-view, double-clicking a video, pressing F or clicking the button at the right end of its control bar makes it fill its own slot with the black bars cropped away, instead of going to the monitor's full screen; double-click again or press Esc to see the whole video
- New English documentation

## 0.1.20

- Updated how the packages are built and released; the player works the same as 0.1.19

## 0.1.19

- In multi-view, a file dropped onto a slot replaces that slot's video, and the slot is outlined while dragging
- Multi-view can't be turned off while several videos are playing; with one left, turning it off makes it ordinary playback that carries on from where it was

## 0.1.18

- In multi-view, the × at the right end of each slot's title bar closes its video and brings back the empty "Add a video" screen, keeping the layout; the main slot can be closed too

## 0.1.17

- In multi-view, one video can go full screen on its own: double-click it, press F, or click the full-screen button at the right end of its control bar, and it takes the whole screen while the others keep playing; double-click again or press Esc to go back
- Each slot's control bar and title bar show only while the mouse is over that slot, and hide when the mouse rests; in full screen the pointer hides too
- Each slot's control bar shows the current time and the total length

## 0.1.16

- New multi-view: the "Multi-view" switch in the player's top bar plays up to 4 videos in one window, each with sound and its own progress and volume
- Slots split left-right or top-bottom, dividers drag to resize, titles drag to swap, and a slot can become a small floating window; presets: Single, Left-right, Top-bottom, Grid, 1 + 3, Picture in picture
- Click a video to play or pause it; double-click to enlarge it to the whole area, and double-click again or press Esc to go back
- "Link" keeps play, pause, seeking and speed in step across all slots; the "Progress bar" switch hides each slot's control bar

## 0.1.13

- Right-click a folder in the Folders view or the left folder list to check it for duplicates

## 0.1.12

- Scanning, thumbnails, the duplicate check and copying across disks pause during playback, and carry on back in the library
- The duplicate check can be limited to one folder and its subfolders

## 0.1.11

- The app now opens on the player page; "Open library" goes to the library
- New library password, asked before entering the library; the library can be locked at any time
- Settings can clear the library in one go, without deleting video files
- Quitting stops every background task, so no update process is left behind

## 0.1.10

- Large libraries are much smoother: scrolling and switching pages no longer stutter with tens of thousands of videos
- A new progress bar at the bottom of the library shows scanning and processing progress and the time left
- Thumbnails are made faster, and an interrupted run carries on next time
- The duplicate check is much faster and shows its progress

## 0.1.9

- New "Manage" page in the library: organize videos with folder sets; search matches folders and videos
- The list shows thumbnails, length, size and modified date, and sorts by the column you click
- Videos or whole folders can be cut to another place; videos can be renamed, tagged and added to favorites in batches
- New possible-duplicates check, comparing length, size and name; videos go to the trash only after you confirm
- An unplugged external drive keeps its records, and comes back by itself when plugged in again
- New private mode (Ctrl/⌘ + Shift + P): nothing is recorded and watch history is hidden

## 0.1.8

- New VR panorama mode for watching 180° and 360° videos on an ordinary screen: drag to look around, scroll to zoom
- Side-by-side and top-bottom 3D panoramas are supported, showing the left eye only
- It turns on by itself when the file name has a mark such as 360, 180 or VR, or switch it with V; each video remembers its own setting
- An update is offered for a different build of the same version too

## 0.1.7

- New "Sound" page in Settings to view, change and delete each output device's volume
- Volume per device can be turned off on the "Sound" page

## 0.1.6

- The floating window resizes from its edges and corners, keeping the video's shape, and opens at the last size next time
- On macOS, the floating window follows you to other desktops
- Headphones, speakers and other outputs each keep their own volume, switched along with the device

## 0.1.5

- In-app updates: download and install new versions inside the app, without a browser
- Choose to update now, be reminded later or skip a version; the download can be canceled
- Downloaded updates are verified first, and aren't installed if the check fails
