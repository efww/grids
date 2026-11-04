# Capture MaguroServer Add dialog with FAKE demo fields only (never real hosts/credentials).
# Env: MAGURO_PORTABLE, MAGURO_CFG_DIR, MAGURO_SHOT_DIR
param()
$ErrorActionPreference='Stop'
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

Add-Type @"
using System; using System.Runtime.InteropServices;
public class AddrEdit {
  public delegate bool CB(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, CB cb, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, string l);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, UIntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr h);
  public static IntPtr FindId(IntPtr parent, int id){
    IntPtr found=IntPtr.Zero;
    EnumChildWindows(parent,(h,l)=>{ if(GetDlgCtrlID(h)==id){ found=h; return false;} return true;}, IntPtr.Zero);
    return found;
  }
  public static void SetAndEnter(IntPtr edit, string text){
    SendMessage(edit, 0x000C /*WM_SETTEXT*/, IntPtr.Zero, text);
    System.Threading.Thread.Sleep(100);
    try { SetFocus(edit); } catch {}
    PostMessage(edit, 0x0100 /*KEYDOWN*/, (UIntPtr)0x0D, IntPtr.Zero);
    PostMessage(edit, 0x0102 /*CHAR*/, (UIntPtr)0x0D, IntPtr.Zero);
    PostMessage(edit, 0x0101 /*KEYUP*/, (UIntPtr)0x0D, IntPtr.Zero);
  }
}
"@

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
  [MaguroCap]::ForceOwnForeground($main, $pid_); Start-Sleep -Milliseconds 400
  # Focus address (Ctrl+L) — gated keybd_event after Maguro FG assert
  [MaguroCap]::ChordTo($main, [uint32]$proc.Id, 0x11, 0x4C)
  Start-Sleep -Milliseconds 500
  # Set address Edit (id 104) via WM_SETTEXT on Maguro-owned HWND only; Enter via PostMessage to that Edit
  $edit = [AddrEdit]::FindId($main, 104)
  if ($edit -eq [IntPtr]::Zero) { throw "no address Edit(104) $($pair.n)" }
  [AddrEdit]::SetAndEnter($edit, $FakeUrl)
  Start-Sleep -Milliseconds 400
  # Fallback Enter via gated TapTo if dialog not yet up
  $srv = [IntPtr]::Zero
  for ($i=0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 250
    $cand = [MaguroCap]::FindClsAny($pid_, 'MaguroServer')
    if ($cand -ne [IntPtr]::Zero) {
      $br = [MaguroCap]::Ext($cand)
      if (($br.Right - $br.Left) -ge 80) { $srv = $cand; break }
    }
    if ($i -eq 8 -or $i -eq 20) {
      [MaguroCap]::ForceOwnForeground($main, $pid_)
      [MaguroCap]::TapTo($main, [uint32]$proc.Id, 0x0D)
      [AddrEdit]::SetAndEnter($edit, $FakeUrl)
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
  [void][MaguroCap]::SetEdit($srv, 15, '')
  $wr = [MaguroCap]::Ext($srv)
  $cw = $wr.Right - $wr.Left
  $slot = [int](($cw - 32 - 24) / 4)
  $cx = 16 + [int]($slot * 2 + $slot / 2)
  $cy = 76
  [MaguroCap]::ClickClient($srv, $cx, $cy)
  Start-Sleep -Milliseconds 400
  Cap-MaguroWindow $srv (Join-Path $Out $pair.n) 5
  [void][MaguroCap]::PostMessage($srv, [MaguroCap]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero)
  [void][MaguroCap]::PostMessage($main, [MaguroCap]::WM_CLOSE, [UIntPtr]::Zero, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 400; Kill-PortableMaguro
}
