# Capture MaguroMain heroes/layouts/tabs/favorites/colors/panedrag/preview from cfg seeds.
# Env: MAGURO_PORTABLE, MAGURO_CFG_DIR, MAGURO_SHOT_DIR
# MAGURO_FORCE=1 overwrites; otherwise skips existing PNG.
# Caveats: no ShowWindow (shell-ext deadlock). Peek = Space on selected image; if MaguroPeek missing, fall back to main frame.
$ErrorActionPreference='Continue'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'cap-lib.ps1')
$Portable = $env:MAGURO_PORTABLE; if (-not $Portable) { throw 'set MAGURO_PORTABLE' }
$CfgDir = $env:MAGURO_CFG_DIR; if (-not $CfgDir) { throw 'set MAGURO_CFG_DIR' }
$Out = $env:MAGURO_SHOT_DIR; if (-not $Out) { throw 'set MAGURO_SHOT_DIR' }
$Exe = Join-Path $Portable 'maguro.exe'
$Json = Join-Path $Portable 'maguro.json'
$Force = ($env:MAGURO_FORCE -eq '1')
New-Item -ItemType Directory -Force $Out | Out-Null

function CapW([IntPtr]$h,[string]$path){
  if((Test-Path $path) -and -not $Force){ Write-Host "skip $path"; return }
  try { Cap-MaguroWindow $h $path } catch { Write-Host "CAP FAIL $path : $_" }
}

if (-not ('CapMain' -as [type])) {
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic;
public static class CapMain {
  public const uint WM_CLOSE=0x0010;
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc lp, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, UIntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint f, UIntPtr e);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  public static void Tap(byte k){ keybd_event(k,0,0,UIntPtr.Zero); keybd_event(k,0,2,UIntPtr.Zero); }
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
  do { $l=[CapMain]::FindOwned($pidMag,$cls); if($l.Count -gt 0){ return $l[0] }; Start-Sleep -Milliseconds 200 } while((Get-Date) -lt $d)
  return [IntPtr]::Zero
}
function StartMag($cfg){
  KillP
  Copy-Item -Force (Join-Path $CfgDir $cfg) $Json
  $p=Start-Process $Exe -WorkingDirectory $Portable -PassThru
  $h=WaitCls ([uint32]$p.Id) 'MaguroMain' 20
  if($h -eq [IntPtr]::Zero){ throw "no MaguroMain $cfg" }
  Start-Sleep -Milliseconds 900
  return @{ Proc=$p; Hwnd=$h; Pid=[uint32]$p.Id }
}
function CloseMag($st){
  if($st -and $st.Hwnd -ne [IntPtr]::Zero){ [void][CapMain]::PostMessage($st.Hwnd,[CapMain]::WM_CLOSE,[UIntPtr]::Zero,[IntPtr]::Zero) }
  Start-Sleep -Milliseconds 400; KillP
}
function ShotMain($cfg,$name){
  $dest=Join-Path $Out "$name.png"
  if((Test-Path $dest) -and -not $Force){ Write-Host "skip $name"; return }
  Write-Host "=== $name ($cfg) ==="
  $st=StartMag $cfg
  CapW $st.Hwnd $dest
  CloseMag $st
}
$jobs = @(
  @{c='hero_light.json';n='hero_light'},@{c='hero_dark.json';n='hero_dark'},
  @{c='hero_light.ko.json';n='hero_light.ko'},@{c='hero_dark.ko.json';n='hero_dark.ko'},
  @{c='hero_light.ja.json';n='hero_light.ja'},@{c='hero_dark.ja.json';n='hero_dark.ja'},
  @{c='layout_1.json';n='layout_1'},@{c='layout_2h.json';n='layout_2h'},@{c='layout_2v.json';n='layout_2v'},
  @{c='layout_3_L.json';n='layout_3'},@{c='layout_3_T.json';n='layout_3_stack'},@{c='layout_3col.json';n='layout_3col'},
  @{c='layout_4.json';n='layout_4'},@{c='layout_4_tall.json';n='layout_4_tall'},@{c='layout_5.json';n='layout_5'},
  @{c='layout_preset_112.json';n='layout_preset_112'},@{c='layout_preset_31.json';n='layout_preset_31'},
  @{c='layout_4.json';n='layout'},@{c='layout.ko.json';n='layout.ko'},@{c='layout.ja.json';n='layout.ja'},
  @{c='tabs.json';n='tabs'},@{c='tabs.ko.json';n='tabs.ko'},@{c='tabs.ja.json';n='tabs.ja'},
  @{c='favorites.json';n='favorites'},@{c='favorites.ko.json';n='favorites.ko'},@{c='favorites.ja.json';n='favorites.ja'},
  @{c='pane_colors.json';n='pane_colors'},@{c='pane_colors.ko.json';n='pane_colors.ko'},@{c='pane_colors.ja.json';n='pane_colors.ja'},
  @{c='panedrag_before.json';n='panedrag_before'},@{c='panedrag_after.json';n='panedrag_after'},@{c='panedrag_dup.json';n='panedrag_dup'}
)
foreach($x in $jobs){ ShotMain $x.c $x.n }
foreach($x in @(
  @{c='preview.json';n='preview';pk='preview_peek'},
  @{c='preview.ko.json';n='preview.ko';pk='preview_peek.ko'},
  @{c='preview.ja.json';n='preview.ja';pk='preview_peek.ja'}
)){
  $destMain=Join-Path $Out "$($x.n).png"
  $destPeek=Join-Path $Out "$($x.pk).png"
  if((Test-Path $destMain) -and (Test-Path $destPeek) -and -not $Force){ Write-Host "skip $($x.n)/$($x.pk)"; continue }
  Write-Host "=== $($x.n) / $($x.pk) ==="
  $st=StartMag $x.c
  CapW $st.Hwnd $destMain
  [void][CapMain]::SetForegroundWindow($st.Hwnd); Start-Sleep -Milliseconds 300
  # click first file then Space for peek
  $r=[MaguroCap]::Ext($st.Hwnd)
  [MaguroCap]::Click(($r.Left+280),($r.Top+280)); Start-Sleep -Milliseconds 400
  [CapMain]::Tap(0x20); Start-Sleep -Milliseconds 900
  $pk=[IntPtr]::Zero
  foreach($cls in @('MaguroPeek','MaguroPreview','MaguroLightbox')){
    $l=[CapMain]::FindOwned($st.Pid,$cls); if($l.Count -gt 0){ $pk=$l[0]; break }
  }
  if($pk -ne [IntPtr]::Zero){
    try { CapW $pk $destPeek }
    catch { Write-Host "WARN peek fail $($x.pk): $_"; CapW $st.Hwnd $destPeek }
  } else {
    Write-Host "WARN no peek hwnd; fallback main"
    CapW $st.Hwnd $destPeek
  }
  CloseMag $st
}
Write-Host 'capture-main DONE'
