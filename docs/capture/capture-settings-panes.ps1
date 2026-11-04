# Capture Maguro Settings > Panes & view (EN/KO/JA).
# Usage:
#   $env:MAGURO_PORTABLE = 'D:\path\to\maguro-app'   # folder containing maguro.exe + maguro.json
#   $env:MAGURO_CFG_DIR  = 'D:\path\to\cfgs'         # dialogs.json, favorites.ko.json, hero_light.ja.json
#   $env:MAGURO_SHOT_DIR = 'D:\path\to\out'
#   powershell -NoProfile -ExecutionPolicy Bypass -File docs\capture\capture-settings-panes.ps1
param()
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'cap-lib.ps1')
$Portable = $env:MAGURO_PORTABLE; if (-not $Portable) { throw 'Set MAGURO_PORTABLE' }
$CfgDir   = $env:MAGURO_CFG_DIR;   if (-not $CfgDir)   { throw 'Set MAGURO_CFG_DIR' }
$Out      = $env:MAGURO_SHOT_DIR;  if (-not $Out)      { throw 'Set MAGURO_SHOT_DIR' }
$Exe = Join-Path $Portable 'maguro.exe'
New-Item -ItemType Directory -Force $Out | Out-Null

function Kill-PortableMaguro {
  Get-CimInstance Win32_Process -Filter "Name='maguro.exe'" -EA SilentlyContinue |
    Where-Object { $_.ExecutablePath -like "$Portable*" } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -EA SilentlyContinue }
  Start-Sleep -Milliseconds 500
}
function Seed([string]$cfg,[string]$lang){
  Copy-Item -Force (Join-Path $CfgDir $cfg) (Join-Path $Portable 'maguro.json')
  $j = Join-Path $Portable 'maguro.json'
  $raw = Get-Content $j -Raw -Encoding utf8 | ConvertFrom-Json
  $raw.lang = $lang; $raw.theme = 'light'; $raw.dark = $false
  ($raw | ConvertTo-Json -Depth 30) + "`n" | Set-Content $j -Encoding utf8
}

foreach ($pair in @(
  @{ c='dialogs.json';       lang='en'; n='settings_panes.png' },
  @{ c='favorites.ko.json'; lang='ko'; n='settings_panes.ko.png' },
  @{ c='hero_light.ja.json';lang='ja'; n='settings_panes.ja.png' }
)) {
  Write-Host "=== $($pair.n) ==="
  Kill-PortableMaguro; Seed $pair.c $pair.lang
  $proc = Start-Process $Exe -WorkingDirectory $Portable -PassThru
  $pid_ = [uint32]$proc.Id
  $main = [IntPtr]::Zero
  for ($i=0; $i -lt 100; $i++) {
    Start-Sleep -Milliseconds 100
    $main = [MaguroCap]::FindCls($pid_, 'MaguroMain')
    if ($main -ne [IntPtr]::Zero) { break }
  }
  if ($main -eq [IntPtr]::Zero) { throw "no MaguroMain $($pair.n)" }
  Start-Sleep 7   # shell-ext settle; do not ShowWindow during enum
  [void][MaguroCap]::SetForegroundWindow($main)
  [MaguroCap]::ChordTo($main, [uint32]$pid_, 0x11, 0xBC)  # Ctrl+,
  Start-Sleep -Milliseconds 1800
  $sh = [IntPtr]::Zero
  for ($i=0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 200
    $sh = [MaguroCap]::FindCls($pid_, 'MaguroSettings')
    if ($sh -ne [IntPtr]::Zero) { break }
  }
  if ($sh -eq [IntPtr]::Zero) { throw "no MaguroSettings $($pair.n)" }
  Start-Sleep -Milliseconds 600
  Click-MaguroSettingsPage $sh 2   # PanesView
  Start-Sleep -Milliseconds 700
  Cap-MaguroWindow $sh (Join-Path $Out $pair.n) 5
  [void][MaguroCap]::PostMessage($sh, [MaguroCap]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero)
  [void][MaguroCap]::PostMessage($main, [MaguroCap]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 400; Kill-PortableMaguro
}

