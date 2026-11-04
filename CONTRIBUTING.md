# Contributing

## Build from source

Needs the Rust GNU toolchain (`x86_64-pc-windows-gnu`) and MSYS2's MinGW tools (`windres`, `dlltool`, `as` in `C:\msys64\mingw64\bin`).

```
build.cmd
```

This builds `target\release\maguro.exe` and copies it to `build\maguro.exe`. Explorer, the title bar, and About all use `assets/Maguro.ico` (the cerulean three-panel mark). Group 1 is the exe icon. Groups 2 and 3 are the same file for dark and light windows.

## Release

Upload two GitHub release assets, and do not zip them:

- `maguro.exe`
- `maguro.exe.sha256`

Always upload `maguro.exe.sha256` with the exe. The file is one sha256sum line, `<64 hex>  maguro.exe` (two spaces). Create it in the folder that contains the exe so the name in the file is `maguro.exe`:

```
sha256sum maguro.exe > maguro.exe.sha256
```

The in-app updater downloads both files, checks the SHA-256, and replaces the exe only when they match. A release with no `maguro.exe.sha256`, or only a zip, is shown to the user and left uninstalled.

