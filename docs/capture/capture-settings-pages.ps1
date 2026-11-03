# Settings General (page 0), Panes (page 2), Right-click menu items (CTX_PAGE=8).
# Env: MAGURO_PORTABLE, MAGURO_CFG_DIR, MAGURO_SHOT_DIR
# Uses Click-MaguroSettingsPage (client WM_LBUTTON*) — screen clicks often stay on General.
$ErrorActionPreference='Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'cap-lib.ps1')
$Portable = $env:MAGURO_PORTABLE; if (-not $Portable) { throw 'MAGURO_PORTABLE' }
$CfgDir = $env:MAGURO_CFG_DIR; if (-not $CfgDir) { throw 'MAGURO_CFG_DIR' }
$Out = $env:MAGURO_SHOT_DIR; if (-not $Out) { throw 'MAGURO_SHOT_DIR' }
$Exe = Join-Path $Portable 'maguro.exe'
$Json = Join-Path $Portable 'maguro.json'
$Force = ($env:MAGURO_FORCE -eq '1')
New-Item -ItemType Directory -Force $Out | Out-Null

if (-not ('CapSet' -as [type])) {
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic;
public static class CapSet {
  public const uint WM_CLOSE=0x0010;
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc lp, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, UIntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint f, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool RedrawWindow(IntPtr h, IntPtr r, IntPtr ur, uint f);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  public static void Chord(params byte[] keys){ foreach(var k in keys) keybd_event(k,0,0,UIntPtr.Zero); for(int i=keys.Length-1;i>=0;i--) keybd_event(keys[i],0,2,UIntPtr.Zero); }
  public static string Cls(IntPtr h){ var sb=new StringBuilder(64); GetClassName(h,sb,64); return sb.ToString(); }
  public static List<IntPtr> FindOwned(uint pid,string cls){
    var f=new List<IntPtr>();
    EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid||!IsWindowVisible(h)) return true; if(Cls(h)==cls) f.Add(h); return true; }, IntPtr.Zero);
    return f;
  }
}
"@
}
function KillP {
  Get-CimInstance Win32_Process -Filter "Name='maguro.exe'" | ForEach-Object {
    if($_.ExecutablePath -and $_.ExecutablePath -like "$Portable*"){ Stop-Process -Id $_.ProcessId -Force -EA SilentlyContinue }
  }
  Close-MaguroBackdrop; Start-Sleep -Milliseconds 500
}
function WaitCls([uint32]$pidMag,[string]$cls,[int]$sec=20){
  $d=(Get-Date).AddSeconds($sec)
  do { $l=[CapSet]::FindOwned($pidMag,$cls); if($l.Count -gt 0){ return $l[0] }; Start-Sleep -Milliseconds 200 } while((Get-Date) -lt $d)
  return [IntPtr]::Zero
}
function StartMag($cfg){
  KillP
  Copy-Item -Force (Join-Path $CfgDir $cfg) $Json
  $p=Start-Process $Exe -WorkingDirectory $Portable -PassThru
  $h=WaitCls ([uint32]$p.Id) 'MaguroMain' 20
  if($h -eq [IntPtr]::Zero){ throw "no MaguroMain $cfg" }
  Start-Sleep -Milliseconds 7000  # shell-ext enum settle
  return @{ Proc=$p; Hwnd=$h; Pid=[uint32]$p.Id }
}
function CloseMag($st){
  if($st -and $st.Hwnd -ne [IntPtr]::Zero){ [void][CapSet]::PostMessage($st.Hwnd,[CapSet]::WM_CLOSE,[UIntPtr]::Zero,[IntPtr]::Zero) }
  Start-Sleep -Milliseconds 400; KillP
}
function SaveIf([string]$path,[scriptblock]$cap){
  if((Test-Path $path) -and -not $Force){ Write-Host "skip $path"; return }
  & $cap
}

# pairs: general + general alias + ctx page 8. Panes already have dedicated script.
foreach($pair in @(
  @{c='dialogs.json'; gen='settings'; gen2='settings_general'; ctx='settings_ctxmenu'},
  @{c='favorites.ko.json'; gen='settings.ko'; gen2='settings_general.ko'; ctx='settings_ctxmenu.ko'},
  @{c='hero_light.ja.json'; gen='settings.ja'; gen2='settings_general.ja'; ctx='settings_ctxmenu.ja'}
)){
  Write-Host "=== settings suite $($pair.gen) ==="
  $st=StartMag $pair.c
  [void][CapSet]::SetForegroundWindow($st.Hwnd)
  [CapSet]::Chord(0x11,0xBC)  # Ctrl+,
  Start-Sleep -Milliseconds 2000
  $sh=WaitCls $st.Pid 'MaguroSettings' 10
  if($sh -eq [IntPtr]::Zero){ Write-Host "FAIL settings"; CloseMag $st; continue }
  Start-Sleep -Milliseconds 800
  try { [void][CapSet]::RedrawWindow($sh,[IntPtr]::Zero,[IntPtr]::Zero,0x0105) } catch {}
  Start-Sleep -Milliseconds 400
  # page 0 General
  Click-MaguroSettingsPage $sh 0
  Start-Sleep -Milliseconds 700
  try { [void][CapSet]::RedrawWindow($sh,[IntPtr]::Zero,[IntPtr]::Zero,0x0105) } catch {}
  Start-Sleep -Milliseconds 400
  SaveIf (Join-Path $Out "$($pair.gen).png") { Cap-MaguroWindow $sh (Join-Path $Out "$($pair.gen).png") }
  SaveIf (Join-Path $Out "$($pair.gen2).png") { Cap-MaguroWindow $sh (Join-Path $Out "$($pair.gen2).png") }
  # page 8 Right-click menu items (CTX_PAGE)
  Click-MaguroSettingsPage $sh 8
  Start-Sleep -Milliseconds 900
  try { [void][CapSet]::RedrawWindow($sh,[IntPtr]::Zero,[IntPtr]::Zero,0x0105) } catch {}
  Start-Sleep -Milliseconds 500
  SaveIf (Join-Path $Out "$($pair.ctx).png") { Cap-MaguroWindow $sh (Join-Path $Out "$($pair.ctx).png") }
  CloseMag $st
}
Write-Host 'capture-settings-pages DONE'
