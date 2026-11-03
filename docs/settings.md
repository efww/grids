# Maguro settings

Where the file lives is described under [Install](../README.md#install).

## Settings file

Maguro reads and writes `maguro.json` in the settings folder (`%APPDATA%\Maguro`, or beside the exe when `portable.txt` is present). A missing file starts from the built-in defaults. Named sessions are `.maguro` files in `maguro-sessions`.

## Writing, language, and view

The file is replaced atomically. A failed write is tried again in 5 seconds. A file that cannot be read is left alone (Retry or Cancel, never overwritten with defaults). Unknown keys are kept. Minimizing keeps a maximized window maximized.

The default follows the Windows display language when a language file matches it; otherwise English. Auto, English, and every language file can be chosen. Bundled files are Korean, Japanese, French, and German (`"ko"`, `"ja"`, `"fr"`, `"de"`). Saved `"en"`, `"ko"`, and `"ja"` still work, and any other ISO code such as `"es"` is kept when that file exists. See [Language files](../README.md#language-files). Explorer parts inside each pane, such as column names and right-click menus, follow the Windows display language. The font defaults to that language's face (Segoe UI, or Malgun Gothic / Yu Gothic UI for Korean and Japanese, or the face named in the file). A chosen face applies to the whole window at once (`font_face`). There is no font size setting, and an older `font` value is ignored.

View → Show file extensions is Windows' own setting, the same one Explorer uses, and it is on the Settings page too. View → Follow Windows light / dark is the check for a new install. View → Dark theme or Light theme picks light or dark by hand and turns following off. Lists are recoloured in place, so history, the filter, the selection, the scroll position, and an address being typed stay.

## Keys

| Key | Notes |
| --- | --- |
| `side_split` | The folder tree's share of the left panel. Favourites use the rest. Restored on startup, in either order. Settings can hide the whole left panel. |
| `side_order` | Which half of the left panel is on top. Missing means favourites on top. `"tree"` puts the folder tree on top. The divider still drags, and `side_split` stays the folder tree's share of the height. Settings → Panes & view applies it right away. The key is omitted while favourites are on top. |
| `use_everything` | On unless `"use_everything": false`. Subfolder search then asks Everything when it is running. Settings → General shows Installed and running, Installed but not running, or Not installed, plus the version when Everything answers. Start Everything launches the installed `Everything.exe`. Install runs `winget install --id voidtools.Everything -e` when winget is available, and otherwise opens the voidtools download page. |
| `winmerge_path` | Empty means auto-detect, and the key is omitted. Settings → General → WinMerge shows whether WinMerge was found and accepts a custom `WinMergeU.exe` or its folder. Detection looks at that path, then `HKLM`/`HKCU` `Software\Thingamahoochie\WinMerge` (`Executable`), App Paths for `WinMergeU.exe`, Program Files, and `PATH`. Compare folders with Ctrl+Alt+C, the gear menu, or the file-list menu. |
| `ev_asked` | Remembers the answer to the one-time install prompt. The first subfolder search without Everything installed asks once. |
| `drive_buttons` | On when the key is missing. Settings → Appearance → Show drive buttons hides them and gives that space to the tabs. A narrow window hides the last letters first. |
| `layout_new` | Default layout for a new tab, a new window, and a first start: two panes side by side, top and bottom, 2×2, or one pane. A fresh install is one pane (`3`). A path handed over by another launch still opens as a one-pane tab. A saved session keeps the layout stored in that file. |
| `hotkey` | On when the line is missing. Win+Shift+E brings Maguro forward; press again to minimize. If a Maguro dialog is open, that dialog comes forward. View shows a check only while the key is really registered, so one click retries after a failed start. The shortcut can be changed in Settings. |
| `dialog_jump` | On when the key is missing. While another app's Open or Save dialog is in front, the shortcut (Ctrl+Alt+G, changeable under Shortcuts) lists folders open in Maguro and navigates that dialog to the one you pick. The shortcut is registered only while such a dialog is in front. Virtual folders are skipped. Win+Shift+E is unchanged. |
| `space_preview` | On when the key is missing. Settings → Panes & view → Preview with Space turns this off. A space within a second of typing a name, or while Korean or Japanese IME text is being composed, stays part of that name. Ctrl+Space and Shift+Space are left alone. |
| `preview_pane` | Off when the key is missing. The preview pane shows the focused file of the active pane, drawn by a Windows preview handler (the same mechanism as Explorer). Otherwise a thumbnail, or a short note. Alt+P shows or hides it. A named `.maguro` file does not change this. |
| `preview_width` | Width of that pane in DIP. Missing, or `0`, means 280 until the splitter is dragged. The smallest saved width is 160. Double-click the splitter to return to the default. |
| `folders` | Folders open as pane tabs, stored on each pane, and in named sessions. An older `pane_tabs` value is ignored. |
| `favorites_tree` | The favourites tree. An older flat list is moved into the root. |
| `one_window` | Single-window mode. Off when the line is missing. Another launch restores a minimized window and opens path arguments as tabs in that window. Ctrl+N, dragging a tab off the strip, Move to new window, and Win+Shift+E while Maguro is already in front open a tab here instead of a window. |
| `lang` | `"auto"`, `"en"`, or a language-file code such as `"ko"`, `"ja"`, `"fr"`, `"de"`. A missing key, like `"auto"`, follows the Windows display language when a matching file exists; otherwise English. |
| `font_face` | Font for the whole window. No font size is stored. An older `font` value is ignored. |
| `blocked_shell_extensions` | DLL file names Maguro should not load, for example `"codec.dll"`. Empty means nothing is blocked, and the key is omitted. This applies only inside Maguro. The registry is not written, and Explorer is unchanged. A Windows system DLL (anything under System32, SysWOW64, or WinSxS, including `shell32.dll` and `explorerframe.dll`) is never blocked. Restart Maguro after editing the list. A handler the shell loads by path, with no CLSID, cannot be blocked this way; a crash report still names that DLL. |

## Crash reports

If Maguro exits from a panic, an access violation, or a fail-fast inside a shell extension, the next launch shows “Maguro closed unexpectedly last time”, the report path, and Open folder. Reports are text files in the settings folder’s `crashes` directory (`%APPDATA%\Maguro\crashes`, or next to the exe when `portable.txt` is present). A report includes the version, the thread, the panic or the exception code and address, the faulting module name and offset, the last folder, loaded third-party modules, and loaded shell-extension DLLs. Open folder opens that `crashes` directory.
