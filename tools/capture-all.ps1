# Retake every README and help capture (ko / ja / en, light, plus hero_dark*).
# Machine paths come from the environment or from -Portable / -CfgDir / -ShotDir.
# No personal paths, hosts, or passwords belong in this file.
#
#   $env:MAGURO_PORTABLE = 'D:\path\to\maguro-app'
#   $env:MAGURO_CFG_DIR  = 'D:\path\to\readme-cfgs'
#   $env:MAGURO_SHOT_DIR = (Resolve-Path .\docs\images-v4)
#   $env:MAGURO_FORCE    = '1'
#   pwsh -NoProfile -ExecutionPolicy Bypass -File .\tools\capture-all.ps1
param(
  [string]$Portable,
  [string]$CfgDir,
  [string]$ShotDir,
  [string]$HelpDemo,
  [string]$ExcelDemo
)

$ErrorActionPreference = 'Continue'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$CapDir = Join-Path $RepoRoot 'docs\capture'

function Resolve-CapPath([string]$Given, [string]$EnvName, [string]$Fallback) {
  $v = $Given
  if (-not $v) { $v = [Environment]::GetEnvironmentVariable($EnvName) }
  if (-not $v -and $Fallback) { $v = $Fallback }
  if (-not $v) { return $null }
  if (-not (Test-Path -LiteralPath $v)) { return $v }
  return (Resolve-Path -LiteralPath $v).Path
}

if (-not $Portable) { $Portable = $env:MAGURO_PORTABLE }
if (-not $Portable) { throw 'Set MAGURO_PORTABLE or pass -Portable (folder that contains maguro.exe).' }
$Portable = (Resolve-Path -LiteralPath $Portable).Path
$exe = Join-Path $Portable 'maguro.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw "maguro.exe not found under $Portable" }

$parent = Split-Path -Parent $Portable
if (-not $CfgDir) { $CfgDir = $env:MAGURO_CFG_DIR }
if (-not $CfgDir) {
  $guess = Join-Path $parent 'readme2-cfgs'
  if (Test-Path -LiteralPath $guess) { $CfgDir = $guess }
}
if (-not $CfgDir -or -not (Test-Path -LiteralPath $CfgDir)) { throw 'Set MAGURO_CFG_DIR to the seed folder (hero_light.json and the rest).' }
$CfgDir = (Resolve-Path -LiteralPath $CfgDir).Path

if (-not $ShotDir) { $ShotDir = $env:MAGURO_SHOT_DIR }
if (-not $ShotDir) { $ShotDir = Join-Path $RepoRoot 'docs\images-v4' }
New-Item -ItemType Directory -Force -Path $ShotDir | Out-Null
$ShotDir = (Resolve-Path -LiteralPath $ShotDir).Path

if (-not $HelpDemo) { $HelpDemo = $env:MAGURO_HELP_DEMO }
if (-not $HelpDemo) {
  $guess = Join-Path $parent 'help-demo'
  if (Test-Path -LiteralPath $guess) { $HelpDemo = $guess }
}
if ($HelpDemo -and (Test-Path -LiteralPath $HelpDemo)) { $HelpDemo = (Resolve-Path -LiteralPath $HelpDemo).Path }

if (-not $ExcelDemo) { $ExcelDemo = $env:MAGURO_EXCEL_DEMO }
if (-not $ExcelDemo) {
  $guess = Join-Path $parent 'readme-excel-demo'
  if (Test-Path -LiteralPath $guess) { $ExcelDemo = $guess }
}
if ($ExcelDemo -and (Test-Path -LiteralPath $ExcelDemo)) { $ExcelDemo = (Resolve-Path -LiteralPath $ExcelDemo).Path }

$env:MAGURO_PORTABLE = $Portable
$env:MAGURO_CFG_DIR = $CfgDir
$env:MAGURO_SHOT_DIR = $ShotDir
if ($HelpDemo) { $env:MAGURO_HELP_DEMO = $HelpDemo }
if ($ExcelDemo) { $env:MAGURO_EXCEL_DEMO = $ExcelDemo }
if (-not $env:MAGURO_FORCE) { $env:MAGURO_FORCE = '1' }

$log = Join-Path $env:TEMP 'maguro-capture-all.log'
function Log([string]$msg) {
  $line = '{0} {1}' -f (Get-Date -Format 'HH:mm:ss'), $msg
  Add-Content -LiteralPath $log -Value $line -Encoding utf8
  Write-Host $line
}

Set-Content -LiteralPath $log -Value ("capture-all start {0}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss')) -Encoding utf8
Log "portable=$Portable"
Log "cfg=$CfgDir"
Log "shots=$ShotDir"
Log "helpDemo=$HelpDemo"
Log "excelDemo=$ExcelDemo"
Log "force=$($env:MAGURO_FORCE)"

$scripts = @(
  'capture-main.ps1',
  'capture-features.ps1',
  'capture-help-dialogs.ps1',
  'capture-servers-fake.ps1',
  'capture-settings-panes.ps1',
  'capture-settings-pages.ps1'
)

$failed = @()
foreach ($name in $scripts) {
  $ps1 = Join-Path $CapDir $name
  if (-not (Test-Path -LiteralPath $ps1)) { Log "MISSING $name"; $failed += $name; continue }
  Log "BEGIN $name"
  & pwsh -NoProfile -ExecutionPolicy Bypass -File $ps1
  $code = $LASTEXITCODE
  if ($null -eq $code) { $code = 0 }
  Log "END $name exit=$code"
  if ($code -ne 0) { $failed += $name }
}

# Expected README / help set (same names as images-v3).
$expected = @(
  'address_toolbar.ja.png','address_toolbar.ko.png','address_toolbar.png',
  'copy_progress.ja.png','copy_progress.ko.png','copy_progress.png',
  'dialog_jump.ja.png','dialog_jump.ko.png','dialog_jump.png',
  'favorites.ja.png','favorites.ko.png','favorites.png',
  'feature_basket.ja.png','feature_basket.ko.png','feature_basket.png',
  'feature_excel_readonly_tidy.ja.png','feature_excel_readonly_tidy.ko.png','feature_excel_readonly_tidy.png',
  'feature_folder_watch.ja.png','feature_folder_watch.ko.png','feature_folder_watch.png',
  'feature_win_shift_v.ja.png','feature_win_shift_v.ko.png','feature_win_shift_v.png',
  'help.ja.png','help.ko.png','help.png',
  'hero_dark.ja.png','hero_dark.ko.png','hero_dark.png',
  'hero_light.ja.png','hero_light.ko.png','hero_light.png',
  'layout.ja.png','layout.ko.png','layout.png',
  'layout_1.png','layout_2h.png','layout_2v.png','layout_3.png','layout_3_stack.png','layout_3col.png',
  'layout_4.png','layout_4_tall.png','layout_5.png','layout_preset_112.png','layout_preset_31.png',
  'pane_colors.ja.png','pane_colors.ko.png','pane_colors.png',
  'pane_tabs.ja.png','pane_tabs.ko.png','pane_tabs.png',
  'panedrag_after.png','panedrag_before.png','panedrag_dup.png',
  'preview.ja.png','preview.ko.png','preview.png',
  'preview_peek.ja.png','preview_peek.ko.png','preview_peek.png',
  'rename.ja.png','rename.ko.png','rename.png',
  'search.ja.png','search.ko.png','search.png',
  'servers.ja.png','servers.ko.png','servers.png',
  'settings.ja.png','settings.ko.png','settings.png',
  'settings_ctxmenu.ja.png','settings_ctxmenu.ko.png','settings_ctxmenu.png',
  'settings_general.ja.png','settings_general.ko.png','settings_general.png',
  'settings_panes.ja.png','settings_panes.ko.png','settings_panes.png',
  'tab_list.png',
  'tabs.ja.png','tabs.ko.png','tabs.png',
  'workspaces.ja.png','workspaces.ko.png','workspaces.png'
)

function Script-For([string]$file) {
  switch -Regex ($file) {
    '^servers\.' { return 'docs/capture/capture-servers-fake.ps1' }
    '^settings_panes\.' { return 'docs/capture/capture-settings-panes.ps1' }
    '^settings' { return 'docs/capture/capture-settings-pages.ps1' }
    '^(address_toolbar|pane_tabs|workspaces|help|dialog_jump|copy_progress)\.' { return 'docs/capture/capture-help-dialogs.ps1' }
    '^(search|rename|tab_list|feature_)' { return 'docs/capture/capture-features.ps1' }
    default { return 'docs/capture/capture-main.ps1' }
  }
}
function Lang-For([string]$file) {
  if ($file -match '\.ko\.png$') { return 'ko' }
  if ($file -match '\.ja\.png$') { return 'ja' }
  return 'en'
}
function Theme-For([string]$file) {
  if ($file -like 'hero_dark*') { return 'dark' }
  return 'light'
}

Add-Type -AssemblyName System.Drawing | Out-Null
$items = @()
$missing = @()
foreach ($name in $expected) {
  $path = Join-Path $ShotDir $name
  if (-not (Test-Path -LiteralPath $path)) { $missing += $name; continue }
  $fi = Get-Item -LiteralPath $path
  $img = [System.Drawing.Image]::FromFile($fi.FullName)
  $w = $img.Width; $h = $img.Height
  $img.Dispose()
  $items += [pscustomobject]@{
    file = $name
    width = $w
    height = $h
    bytes = $fi.Length
    lang = (Lang-For $name)
    theme = (Theme-For $name)
    script = (Script-For $name)
    mtime = $fi.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss')
  }
}

$stamp = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
$md = New-Object System.Collections.Generic.List[string]
[void]$md.Add('# images-v4 capture manifest')
[void]$md.Add('')
[void]$md.Add("Generated: $stamp")
[void]$md.Add('')
[void]$md.Add('Portable Maguro only. Fake server fields are Demo Server / sftp.example.com / demo. No real hosts or passwords.')
[void]$md.Add('')
[void]$md.Add(('Present: {0} / {1}. Missing: {2}.' -f $items.Count, $expected.Count, $missing.Count))
if ($failed.Count -gt 0) {
  [void]$md.Add('')
  [void]$md.Add('Scripts that returned non-zero: ' + ($failed -join ', '))
}
if ($missing.Count -gt 0) {
  [void]$md.Add('')
  [void]$md.Add('Missing: ' + ($missing -join ', '))
}
[void]$md.Add('')
[void]$md.Add('| file | WxH | bytes | lang | theme | script | mtime |')
[void]$md.Add('| --- | --- | ---: | --- | --- | --- | --- |')
foreach ($it in $items) {
  [void]$md.Add(('| {0} | {1}x{2} | {3} | {4} | {5} | {6} | {7} |' -f $it.file, $it.width, $it.height, $it.bytes, $it.lang, $it.theme, $it.script, $it.mtime))
}
[void]$md.Add('')
$manMd = Join-Path $ShotDir 'MANIFEST.md'
Set-Content -LiteralPath $manMd -Value ($md -join "`n") -Encoding utf8

$json = @{
  generated = $stamp
  count = $items.Count
  expected = $expected.Count
  missing = $missing
  failed_scripts = $failed
  files = $items
} | ConvertTo-Json -Depth 5
Set-Content -LiteralPath (Join-Path $ShotDir 'manifest.json') -Value $json -Encoding utf8
Log ("manifest {0}/{1} missing={2}" -f $items.Count, $expected.Count, $missing.Count)
if ($missing.Count -gt 0 -or $failed.Count -gt 0) { exit 2 }
exit 0
