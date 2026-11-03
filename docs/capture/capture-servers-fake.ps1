# Capture MaguroServer Add dialog with FAKE demo fields only (never real hosts/credentials).
# Required fill (t633u policy):
#   display name: Demo Server
#   host:         sftp.example.com
#   port:         22
#   remote path:  /home/demo/projects
#   user:         demo
#   password:     empty
# Env: MAGURO_PORTABLE, MAGURO_CFG_DIR, MAGURO_SHOT_DIR (see capture-settings-panes.ps1)
param()
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
. (Join-Path $PSScriptRoot 'cap-lib.ps1')
$Portable = $env:MAGURO_PORTABLE; if (-not $Portable) { throw 'Set MAGURO_PORTABLE' }
$CfgDir   = $env:MAGURO_CFG_DIR;   if (-not $CfgDir)   { throw 'Set MAGURO_CFG_DIR' }
$Out      = $env:MAGURO_SHOT_DIR;  if (-not $Out)      { throw 'Set MAGURO_SHOT_DIR' }
$Exe = Join-Path $Portable 'maguro.exe'
New-Item -ItemType Directory -Force $Out | Out-Null
$FakeUrl = 'sftp://demo@sftp.example.com/home/demo/projects'

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
  @{ c='favorites.json';    lang='en'; n='servers.png' },
  @{ c='favorites.ko.json'; lang='ko'; n='servers.ko.png' },
  @{ c='favorites.ja.json'; lang='ja'; n='servers.ja.png' }
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
  Start-Sleep 7
  [void][MaguroCap]::SetForegroundWindow($main); Start-Sleep -Milliseconds 400
  [MaguroCap]::Chord(0x11, 0x4C)  # Ctrl+L
  Start-Sleep -Milliseconds 500
  [Windows.Forms.SendKeys]::SendWait($FakeUrl)
  Start-Sleep -Milliseconds 300
  [MaguroCap]::Tap(0x0D)
  $srv = [IntPtr]::Zero
  for ($i=0; $i -lt 30; $i++) {
    Start-Sleep -Milliseconds 300
    $cand = [MaguroCap]::FindClsAny($pid_, 'MaguroServer')
    if ($cand -ne [IntPtr]::Zero) {
      $br = [MaguroCap]::Ext($cand)
      if (($br.Right - $br.Left) -ge 80) { $srv = $cand; break }
    }
  }
  if ($srv -eq [IntPtr]::Zero) { throw "no MaguroServer $($pair.n)" }
  Start-Sleep -Milliseconds 400
  # IDs from ui/srvdlg.rs: NAME=10 HOST=11 PORT=12 REMOTE=13 USER=14 PASS=15
  [void][MaguroCap]::SetEdit($srv, 10, 'Demo Server')
  [void][MaguroCap]::SetEdit($srv, 11, 'sftp.example.com')
  [void][MaguroCap]::SetEdit($srv, 12, '22')
  [void][MaguroCap]::SetEdit($srv, 13, '/home/demo/projects')
  [void][MaguroCap]::SetEdit($srv, 14, 'demo')
  [void][MaguroCap]::SetEdit($srv, 15, '')   # empty password — never real credentials
  # Click SFTP kind (index 2 of SMB/WebDAV/SFTP/FTP)
  $wr = [MaguroCap]::Ext($srv)
  $cw = $wr.Right - $wr.Left
  $slot = [int](($cw - 32 - 24) / 4)
  $kx = $wr.Left + 16 + [int]($slot * 2 + $slot / 2)
  $ky = $wr.Top + 76
  [MaguroCap]::Click($kx, $ky)
  Start-Sleep -Milliseconds 400
  Cap-MaguroWindow $srv (Join-Path $Out $pair.n) 5
  [void][MaguroCap]::PostMessage($srv, [MaguroCap]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero)
  [void][MaguroCap]::PostMessage($main, [MaguroCap]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 400; Kill-PortableMaguro
}
