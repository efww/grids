<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/maguro-lockup-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="assets/maguro-lockup-light.png">
    <img alt="Maguro" src="assets/maguro-lockup-light.png" width="192">
  </picture>
</p>

<h1 align="center">Maguro</h1>

<p align="center">A fast, lightweight multi-pane file manager for Windows.</p>

> **Maguro is a fork of [efww/grids](https://github.com/efww/grids)** by efww. It uses its own exe name (`maguro.exe`), window class, settings folder and hotkey (Win+Shift+E), so it can run next to the original Grids without touching its settings.

<p align="center">
  <a href="https://github.com/Samek86/maguro"><img alt="Platform: Windows" src="https://img.shields.io/badge/platform-Windows-0078D4?logo=windows&logoColor=white"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <a href="https://github.com/Samek86/maguro"><img alt="Language: Rust" src="https://img.shields.io/badge/language-Rust-000000?logo=rust&logoColor=white"></a>
  <a href="https://github.com/Samek86/maguro/commits"><img alt="Latest commit" src="https://img.shields.io/github/last-commit/Samek86/maguro"></a>
</p>

<p align="center">
  English | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/hero_dark.png">
    <img alt="Maguro — light theme hero" src="docs/images/hero_light.png">
  </picture>
</p>

Every pane is a real Windows Explorer view: the full right-click menu, drag and drop, thumbnails, and Windows' own copy / move dialog all just work. Maguro opens with a single pane by default; grow the layout when you need more.

## Features

### 1. Layout gallery — every preset

Start with one pane, then pick any built-in grid (up to 4×4). Drag boundaries to resize. Named sessions remember layout, folders, columns, and colour labels; opening a session adds a window tab beside the current one.

<p align="center"><img width="760" alt="Multi-pane layout" src="docs/images/layout.png"></p>

<table>
<tr>
<td><img alt="1 pane" src="docs/images/layout_1.png"></td>
<td><img alt="2 horizontal" src="docs/images/layout_2h.png"></td>
<td><img alt="2 vertical" src="docs/images/layout_2v.png"></td>
</tr>
<tr>
<td><img alt="3 panes" src="docs/images/layout_3.png"></td>
<td><img alt="3 stack" src="docs/images/layout_3_stack.png"></td>
<td><img alt="3 columns" src="docs/images/layout_3col.png"></td>
</tr>
<tr>
<td><img alt="4 panes" src="docs/images/layout_4.png"></td>
<td><img alt="4 tall" src="docs/images/layout_4_tall.png"></td>
<td><img alt="5 panes" src="docs/images/layout_5.png"></td>
</tr>
<tr>
<td><img alt="1+1+2 preset" src="docs/images/layout_preset_112.png"></td>
<td><img alt="3+1 preset" src="docs/images/layout_preset_31.png"></td>
<td></td>
</tr>
</table>

### 2. Window tabs and nested pane tabs

Chrome-style tabs across the window, plus tabs inside each pane. A tab is named after its active folder (full path in the tooltip). Duplicate a tab, move it to a new window, or save it as a session.

<p align="center"><img width="760" alt="Window and pane tabs" src="docs/images/tabs.png"></p>

### 3. Pin, lock, and colour tabs

Double-click or right-click a tab → Rename to pin your own name. Lock a tab so it cannot be closed by accident. Pick a tab colour so related work stands out at a glance.

### 4. Drag a pane number to swap — Ctrl+drag to duplicate

Grab the pane number badge and drop it on another pane to swap their places. Hold **Ctrl** while dragging to duplicate that pane into the target slot instead of swapping.

<p align="center">
  <img width="360" alt="Before pane drag" src="docs/images/panedrag_before.png">
  <img width="360" alt="After pane swap" src="docs/images/panedrag_after.png">
</p>
<p align="center"><img width="560" alt="Ctrl+drag duplicates a pane" src="docs/images/panedrag_dup.png"></p>

### 5. Per-pane colours

Give each pane its own accent so you always know which side is source and which is destination — useful in a busy multi-pane layout.

<p align="center"><img width="760" alt="Per-pane colours" src="docs/images/pane_colors.png"></p>

### 6. Built for speed

Cold start measured at **607 ms** on this portable demo build. Paste and drag to or from a network folder can use robocopy (Settings → File operations; off by default). Right-click → Fast paste always uses it, with Maguro's own progress and overwrite / skip / cancel.

**Pane search via Everything.** Ctrl+F filters as you type; tick Subfolders and Maguro asks [voidtools Everything](https://www.voidtools.com/) for instant results (or walks the disk if Everything is not running).

<p align="center"><img width="760" alt="Everything-powered pane search" src="docs/images/search.png"></p>

### 7. Favourites — folders and files, in groups

The sidebar is split evenly: favourites on top, folder tree below. Favourites hold Explorer Quick-access pins plus your own groups (nest folders and files, drag to reorder). Ctrl+B opens the quick favourites list. Drop files or folders onto the panel to pin them.

<p align="center"><img width="760" alt="Favourites with groups of folders and files" src="docs/images/favorites.png"></p>

### 8. Quick preview

Space or F3 previews pictures, PDF, Office, HTML, Markdown, code, CSV, ZIP, HWP/HWPX (Hancom) and more. Ctrl+wheel zooms (10–800%), drag pans, the wheel scrolls, ←/→ step through files.

<p align="center">
  <img width="480" alt="Preview in a pane" src="docs/images/preview.png">
  <img width="240" alt="Peek preview window" src="docs/images/preview_peek.png">
</p>

### 9. Configurable context menu

Settings → Right-click menu items lets you show or hide Maguro's own commands on the Explorer context menu — keep the list lean, or bring every action back.

<p align="center"><img width="560" alt="Configurable context menu settings" src="docs/images/settings_ctxmenu.png"></p>

### 10. Batch rename with live name preview

Select files, press Ctrl+Shift+R. Find / replace, numbering, extensions, and presets update a live preview of every new name before you commit.

<p align="center"><img width="560" alt="Batch rename with name preview" src="docs/images/rename.png"></p>

### 11. Server connections

Right-click the favourites panel (or a group) → **Add server connection…**. SMB and WebDAV open as normal Windows folders. SFTP and FTP are mounted with rclone and WinFsp, then browse like any other folder — including preview, the context menu, and fast copy. Passwords stay in Windows Credential Manager and are never written to `maguro.json`.

<p align="center"><img width="420" alt="Server connection dialog with filled fields" src="docs/images/servers.png"></p>

### Themes & accent

Light and dark that follow Windows, six accent colours per theme (default cerulean `#0A9BD6`), and a settings window in English, 日本語, and 한국어.

<table>
<tr>
<td><img alt="Settings" src="docs/images/settings.png"></td>
<td><img alt="Help" src="docs/images/help.png"></td>
</tr>
<tr>
<td colspan="2"><img alt="Dark theme" src="docs/images/hero_dark.png"></td>
</tr>
</table>

📦 **A small single exe.** Nothing to install.

## Install

Download the latest `maguro.exe` from [Releases](https://github.com/Samek86/maguro/releases/latest). Put it in any folder and run it. Releases ship that raw exe and a `maguro.exe.sha256` checksum, not a zip.

The exe is not code-signed, so Windows SmartScreen may warn the first time. Choose **More info → Run anyway**.

**Requirements:** Windows 10 (1903 or later) or Windows 11, x64. SFTP/FTP also need [rclone](https://rclone.org/) and [WinFsp](https://winfsp.dev/) (Maguro can install them when you add a connection).

Settings live in `%APPDATA%\Maguro\maguro.json`. Put a file named `portable.txt` next to `maguro.exe` to keep `maguro.json` beside the exe instead. Named sessions stay `.grids` files in the `maguro-sessions` folder next to the exe.

To build from source, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Keyboard shortcuts

Defaults; every one can be changed in Settings → Shortcuts.

| Group | Keys | Action |
| --- | --- | --- |
| **Tabs** | Ctrl+T | New tab |
|  | Ctrl+W | Close tab |
|  | Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
|  | Ctrl+Shift+T | Reopen closed tab |
| **Panes** | Alt+1…9 | Focus pane |
|  | Ctrl+\ | Layout picker |
| **Preview** | Space / F3 | Preview |
|  | Esc / Space / F3 | Close preview |
| **File** | Ctrl+Shift+C | Copy full path |
|  | Ctrl+Shift+R | Batch rename |
|  | Num+ or Ctrl+Shift+A / Num− / Num* | Select / deselect / invert by pattern |
|  | Ctrl+Alt+N | New text file |
|  | Ctrl+Shift+N, F7 | New folder |
|  | Ctrl+Alt+C | Compare the two panes in WinMerge |
|  | F2, Del, Ctrl+C / V, F5 | Same as Explorer. F5 reloads the active pane; a failed network folder is tried again |
| **Other** | Ctrl+B | Quick favourites list |
|  | Ctrl+S | Save this tab |
|  | Ctrl+, | Settings |
|  | F1 / Ctrl+/ | Help / shortcut list |
|  | Ctrl+N | New window |
|  | Ctrl+Alt+G | Jump to a Maguro folder in another app's Open or Save dialog |
|  | Win+Shift+E | Bring Maguro to the front (global) |

## Known issues

**Don't run Maguro as administrator.** Tools such as PowerToys Mouse Crosshairs cannot follow the cursor over an elevated Maguro window (UIPI). Run Maguro normally, or run both Maguro and PowerToys as administrator.

## Settings

The gear on the tab row opens the view menu. Settings is in that menu (`Ctrl+,`).

Language (Auto, English, and every language file), font, light / dark theme, accent, and shortcuts are all set there. Migration from older Grids settings, how the file is written, and the full `maguro.json` key table are in [docs/settings.md](docs/settings.md).

## Language files

English is compiled into the program and is the fallback. Korean, Japanese, French, and German ship as `lang/lang.ko.ini`, `lang/lang.ja.ini`, `lang/lang.fr.ini`, and `lang/lang.de.ini`. On first run, and again whenever the version changes, Maguro writes those four files into the `lang` folder next to the settings file. It always overwrites those four. It never writes, overwrites, or deletes a `lang.xx.user.ini`, or any other `lang.xx.ini` you add.

A file is UTF-8 (a leading BOM is ignored). Lines are `key = text`. `[meta]` `name` is the label in the language list. `[meta]` `font` is an optional UI font. A missing key uses the English text. `lang.xx.user.ini` is loaded on top and wins.

## Roadmap

- [ ] Code-signed builds, so SmartScreen does not warn the first time you run `maguro.exe`
- [x] Interface languages beyond English, 한국어, and 日本語

## Credits

Maguro is a fork of [efww/grids](https://github.com/efww/grids) by efww.

Inspired by [Q-Dir](https://www.softwareok.com/?seite=Freeware/Q-Dir) and [voidtools Everything](https://www.voidtools.com/). Maguro is an independent project, not affiliated with Q-Dir or its author.

## License

[MIT](LICENSE) · © 2026 efww, Samek86
