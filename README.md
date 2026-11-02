# Grids

A fast, lightweight multi-pane file manager for Windows. Up to 16 Explorer panes in one window, a folder tree on the left, and layouts you change with one drag. One small exe (about 1 MB), nothing to install.

[한국어](README.ko.md)

[![Grids in motion](docs/demo.gif)](docs/demo.mp4)

▶ [Watch the full launch video with sound](docs/demo.mp4) (35 s)

![Grids, light theme](docs/screenshot-light.png)

Every pane is a real Windows Explorer view, so right-click menus, drag and drop, thumbnails, rename and copy progress all work exactly as they do in Explorer.

## Features

- **Free layouts**: up to 4 columns, each with its own number of panes (1 to 4). Click the layout button for an even grid, or press and drag across it to set each column; six presets for common setups. Adding or removing cells keeps every remaining pane in its place, and a pane whose cell comes back returns there.
- **Resizable everything**: drag any boundary between panes or the sidebar (hovering one marks it with a line in the selection colour); double-click a boundary to even it out.
- **Folder tree** that follows the active pane and keeps the current folder's name in view, even in deep folders.
- **Per-pane toolbar**: address bar (led by the folder's icon, as in Explorer), back / forward / up, name filter (`*.pdf;*.txt` or any part of a name; click its icon or press Alt+↓ in it to pick a file type: pictures, videos, music, documents, archives, programs, or the selected file's type; an empty filter shows "Filter names  Ctrl+F"), open the selected folder in another pane (its columns come along, so you can set one pane up and copy it around) or copy / move the selected files to another pane's folder (up to 10,000), swap places with another pane (nothing is reopened: folder, history, selection, filter and columns move along; the save button's dot shows the new arrangement until you save or swap back), favourite toggle; drag a folder onto a pane's top bar to open it there. Screen readers name the fields by pane ("Pane 2 address", "Pane 2 name filter").
- **Sessions**: the layout, folders and each pane's columns (order, widths, sort) are saved automatically. Save named sessions and load them from cards that preview each layout; hover a card for a ✕ that deletes it, or right-click it to rename it in place (a name that's taken is refused, never overwritten); a green dot on the save button shows unsaved layout changes.
- **Missing folders**: a local folder from a session, favourite, recent list or the command line that was deleted or renamed opens its nearest existing parent instead, and the status line says which folder was missing (8 s); it also leaves the recent list.
- **Columns are a pane setting**: their order, widths and sort stay as the pane moves from folder to folder, and any column can be made narrower than Explorer allows (Size stops at 90 px there).
- **Quick preview (F3 or Space)**: laid out like the app itself — a header with the file's icon, name and facts (type, dimensions, pages or lines, size, date), the content on a white plate, ‹ › at the sides and × to close. Pictures and video stills, PDF pages (Windows' own PDF engine, no reader toolbars), text and source code (line numbers), Markdown (headings, lists, code), CSV / TSV as a table, ZIP and folder contents, and Office, HTML, Outlook mail and fonts through the preview handlers Explorer uses (UTF-8 HTML pages without a charset no longer come out garbled). Without a handler, Word / Excel / PowerPoint / OpenDocument files show their text or first sheet, HWP / HWPX their built-in preview, .eml mail its headers and body. Files Grids can't show get a card: big icon, type, size, dates, and program version, shortcut target or URL. Documents over 10 MB are skipped so nothing hangs, and Esc closes at once even while a preview is opening.
- **Tabs** (View menu → Tabs in panes, off by default): each pane gets a tab strip under its header. Ctrl+T or + opens the current folder in a new tab, Ctrl+W, × or a middle click closes one, Ctrl+PgDn / Ctrl+PgUp or a click switches; double-click the empty strip for a new tab. Ctrl+K or right-click → Duplicate tab copies a tab next to it. Tabs are saved with the panes and sessions, and kept while the strips are hidden.
- **Folder favourites**: star a folder from its pane header, jump to it from the star list in the toolbar.
- **Drive buttons**: C: D: E: … in the top toolbar open that drive in the active pane; the active pane's drive is underlined in the selection colour, and buttons come and go as drives are plugged in or removed.
- **Folder sizes**: the status line also shows the total size of selected folders, summed in the background (F5 sums again; "partial" when some items couldn't be read).
- **View menu**: show hidden files and file name extensions — the same switches as Explorer's View > Show.
- **Light and dark themes** with six selection colours each; the active pane is outlined and its header tinted. Can follow Windows' light/dark setting (View menu, on for a new install; the theme button turns it off); switching keeps each pane's history, filter and selection (on PCs with Windows 11's newer file list, the panes reopen their folders instead).
- **Pane colour labels**: right-click a pane header to give the pane one of those colours, shown as a thin strip along its top and behind its number, and next to its name in the ⇄ menu; saved with the session.
- **Keyboard focus comes back**: after Alt+Tab, the taskbar, a closed popup or restoring from minimized, the keys go straight to the list, address bar, filter or tree you left, with the text and caret you left in it.
- **Single instance**: running `grids.exe C:\a D:\b` or opening a `.grids` session file reuses the open window.
- **Portable**: settings live in `grids.ini` next to the exe, named sessions in the `sessions` folder beside it. Changes are saved automatically as you make them, so after a Windows restart, sign-out or crash Grids reopens the last state it managed to save.
- **English or Korean** (View menu). The Explorer parts inside each pane, such as column names and right-click menus, follow your Windows display language.
- **Batch rename (Ctrl+Shift+R)**: rename the selected items, or every file in the folder, with a stack of rules (replace with optional regex, insert, remove, numbering, date, case, trim spaces). A live preview marks invalid names, duplicates and existing files. The rename is all or nothing, safe for swaps, and can be undone. Rule sets can be saved as presets in grids.ini.
- **More in the quick preview**: Del / Shift+Del sends the file shown to the Recycle Bin / deletes it (Windows asks as in Explorer) and moves on to the next; F2 renames it in place; Ctrl+wheel or Ctrl+= / - / 0 zooms pictures and PDF pages (drag or scrollbars to move around).
- **Small shell helpers**: Shift+F7 makes a new text file ready to rename; the ⇄ menu tells which program is using the selected files (switch to it or ask it to close) and, with [WinMerge](https://winmerge.org) installed, compares 2 selected items or this folder with another pane's.
- **Jump from file dialogs**: in another app's Open/Save dialog press Ctrl+Alt+G to pick one of Grids' open folders; the dialog goes there and keeps the file name you typed (View menu, on by default; the key is only taken while such a dialog is in front).
- **Everything search** (Ctrl+Shift+F or the orange magnifier, shown while [Everything](https://www.voidtools.com) runs): results as you type in a table of name, path, size and date modified (click a column title to sort, again to reverse; size and date start biggest / newest first, and the choice is kept), Enter shows the item selected in the active pane, Ctrl+Enter opens a folder, Alt+O searches only under the current folder.
- **Updates**: when a newer release is out, a blue Update button appears at the top right; one click downloads it from GitHub, checks its SHA-256 and restarts Grids into it with your folders and settings. View > Check for updates turns this (and all network use) off.

| Layout picker | Sessions | Favourites |
| --- | --- | --- |
| ![Layout picker](docs/layout-picker.png) | ![Sessions](docs/sessions.png) | ![Favourites](docs/favourites.png) |

![Grids, dark theme](docs/screenshot-dark.png)

## Download

Get `grids.exe` from [Releases](https://github.com/efww/grids/releases), put it in any folder and run it.

The exe is not code-signed, so Windows SmartScreen may warn the first time: choose **More info → Run anyway**.

**Requirements**: Windows 10 (1903 or later) or Windows 11, x64.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous pane |
| Ctrl+1 … Ctrl+9 | Go to pane 1–9 (the number at the left of each pane header; hidden in very narrow panes) |
| Alt+Shift+← / → / ↑ / ↓ | Pane to the left / right / above / below |
| Ctrl+Alt+Shift+← / → / ↑ / ↓ | Swap the active pane with the one to the left / right / above / below (its folder, history, selection, filter and columns stay; also in the ⇄ menu) |
| Alt+← / Alt+→ | Back / forward |
| Alt+↑ | Parent folder |
| Alt+↓ | Recent folders (all panes, newest first; right-click ← / → too) |
| Ctrl+L, Alt+D, F4 | Address bar: also `..` or a subfolder name, a file to open, or a command such as `cmd`, `wt`, `powershell` (runs in this pane's folder), or part of a recent or favourite folder's name |
| Ctrl+F | Name filter |
| Alt+↓ (in the filter) | File types menu (or click the filter's icon) |
| F6 / Shift+F6 | Move between file list, address bar, filter and folder tree |
| Enter / Esc | Apply / cancel the address or filter |
| Win+Alt+E | Bring Grids to front from any app, ready to type in the active pane (again: minimize; View menu turns it off) |
| Ctrl+H | Show hidden files |
| F3 / Space | Quick preview of the focused file: pictures, text and code, Markdown, CSV tables, Office / PDF / HTML / mail (files up to 10 MB), HWP / HWPX, ZIP contents, folders; anything else as a card of facts (arrows or the wheel: previous / next file without closing; on long text the wheel scrolls first and moves on past the end; PgUp / PgDn: scroll; Esc: close, also while it is still opening) |
| F7 | New folder (named and ready to rename, as Explorer's Ctrl+Shift+N) |
| Ctrl+T / Ctrl+W | New tab / close tab, with tabs on |
| Ctrl+K | Duplicate the current tab (also on a tab's right-click menu) |
| Ctrl+PgDn / Ctrl+PgUp | Next / previous tab |
| Ctrl+V | With a copied picture (screenshot, browser image): save it here as a PNG file |
| Ctrl+S | Save the current session |
| Ctrl+B | Favourites list (↑/↓, Enter to open, Del to remove, Alt+↑/↓ or drag to reorder) |
| Ctrl+/ | Keyboard shortcuts (this list) |
| Ctrl+Shift+M | Maximize / restore the active pane (or double-click its header) |
| Ctrl+Shift+E | Hide / show the folder tree (also View menu → Folder tree; its width is kept) |
| Ctrl+Shift+O | Open the folders of open Explorer windows and tabs in panes, from the active pane on (also View menu; panes already showing one of them stay put) |
| Ctrl+Shift+C | Copy the full path of the selected items (or of the current folder) |
| Num+ / Ctrl+Shift+A | Select files by pattern (`*.jpg;*.png` or part of a name; Shift+Enter deselects) |
| Num− | Deselect files by pattern |
| Num* | Invert the selection |
| Ctrl+Shift+R | Batch rename (in the window: Ctrl+Enter rename, Esc close; rule list: Space on/off, Ins add, Del remove, Alt+↑/↓ move) |
| Shift+F7 | New text file |
| Ctrl+Shift+F | Search with Everything (↑/↓, Enter: show in the active pane, Ctrl+Enter: open a folder, Alt+O: only under this folder) |
| Ctrl+Alt+G | In another app's Open / Save dialog: jump to a folder open in Grids |
| Del / Shift+Del (preview) | Recycle / delete the file shown, go to the next |
| F2 (preview) | Rename the file shown |
| Ctrl+wheel, Ctrl+= / Ctrl+- / Ctrl+0 (preview) | Zoom in / out / fit |
| F2, Del, Ctrl+C / X / V, F5 | Same as Explorer |

Double-clicking empty space in a pane goes up a folder (can be turned off in the view menu).

Address bar: click a folder in the path to go there; click a › to list the folders inside; click the current folder or empty space, or press Ctrl+L, to type a path.

## Known issues

**Don't run Grids as administrator.** A user reported that PowerToys Mouse Crosshairs (and similar tools) stopped following the cursor and froze where they clicked over Grids. It happens only when Grids runs as administrator: Windows doesn't let a program without administrator rights receive mouse input over a window that has them (UIPI), so a normal PowerToys gets no mouse movement over an elevated Grids. Give both the same rights, either way:

1. **Recommended**: run Grids normally. Don't open `grids.exe` with right-click → **Run as administrator**, and in the shortcut's or `grids.exe`'s **Properties → Compatibility**, clear **Run this program as an administrator**.
2. If you need Grids as administrator, run PowerToys as administrator too: **PowerToys Settings → General → Always run as administrator**, then restart PowerToys.

## Build

Needs the Rust GNU toolchain (`x86_64-pc-windows-gnu`) and MSYS2's MinGW tools (`windres`, `dlltool`, `as` in `C:\msys64\mingw64\bin`).

```
build.cmd
```

This builds `target\release\grids.exe` and copies it to `grids.exe` in the project folder. `icon.py` redraws the app icon (`app.ico`).

## Credits

Inspired by [Q-Dir](https://www.softwareok.com/?seite=Freeware/Q-Dir). Grids is an independent project, not affiliated with Q-Dir or its author.

Thanks to Jun ([@Samek86](https://github.com/Samek86)): the fix for F3, Space and F7 doing nothing in the file list on some PCs and the screen-reader pane names after a layout change come from Jun's fork, [Samek86/grids](https://github.com/Samek86/grids).

Batch rename, the preview's Del / F2 / zoom, Shift+F7 new text file, "which program is using this?", WinMerge compare, the file dialog jump, Everything search and the in-app update were ported from Jun's fork [Samek86/maguro](https://github.com/Samek86/maguro) (MIT) and re-implemented for Grids, with Jun credited as co-author on those commits.

## License

[MIT](LICENSE)
