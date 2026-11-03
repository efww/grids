# Feature windows: basket, folder watch, Win+Shift+V, excel tidy; also rename, search, tab_list.
# Env: MAGURO_PORTABLE, MAGURO_CFG_DIR, MAGURO_SHOT_DIR, MAGURO_FORCE=1
# Demo paths come from cfg seeds / optional MAGURO_DEMO_* — never real servers.
$ErrorActionPreference='Continue'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'cap-lib.ps1')
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$Portable = $env:MAGURO_PORTABLE; if (-not $Portable) { throw 'MAGURO_PORTABLE' }
$CfgDir = $env:MAGURO_CFG_DIR; if (-not $CfgDir) { throw 'MAGURO_CFG_DIR' }
$Out = $env:MAGURO_SHOT_DIR; if (-not $Out) { throw 'MAGURO_SHOT_DIR' }
$Exe = Join-Path $Portable 'maguro.exe'
$Json = Join-Path $Portable 'maguro.json'
$Force = ($env:MAGURO_FORCE -eq '1')
$Root = Split-Path $Portable -Parent
New-Item -ItemType Directory -Force $Out | Out-Null

if (-not ('CapFeat' -as [type])) {
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic;
public static class CapFeat {
  public const uint WM_CLOSE=0x0010, WM_COMMAND=0x0111;
  public const int CB_GETCOUNT=0x0146, CB_SETCURSEL=0x014E, CB_GETLBTEXT=0x0148;
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc lp, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, EnumProc lp, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, UIntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, int m, IntPtr w, IntPtr l);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, int m, IntPtr w, StringBuilder l);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint f, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool DestroyWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  public static void Tap(byte k){ keybd_event(k,0,0,UIntPtr.Zero); keybd_event(k,0,2,UIntPtr.Zero); }
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
  Close-MaguroBackdrop; Start-Sleep -Milliseconds 400
}
function WaitCls([uint32]$pidMag,[string]$cls,[int]$sec=20){
  $d=(Get-Date).AddSeconds($sec)
  do { $l=[CapFeat]::FindOwned($pidMag,$cls); if($l.Count -gt 0){ return $l[0] }; Start-Sleep -Milliseconds 200 } while((Get-Date) -lt $d)
  return [IntPtr]::Zero
}
function SaveCap([IntPtr]$h,[string]$path){
  if((Test-Path $path) -and -not $Force){ Write-Host "skip $path"; return }
  try { Cap-MaguroWindow $h $path } catch { Write-Host "CAP FAIL $path : $_" }
}
function StartMag($cfg){
  KillP
  Copy-Item -Force (Join-Path $CfgDir $cfg) $Json
  $p=Start-Process $Exe -WorkingDirectory $Portable -PassThru
  $h=WaitCls ([uint32]$p.Id) 'MaguroMain' 20
  if($h -eq [IntPtr]::Zero){ Write-Host "no MaguroMain $cfg"; KillP; return $null }
  Start-Sleep -Milliseconds 1000
  return @{ Proc=$p; Hwnd=$h; Pid=[uint32]$p.Id }
}
function CloseMag($st){
  if($st -and $st.Hwnd -ne [IntPtr]::Zero){ [void][CapFeat]::PostMessage($st.Hwnd,[CapFeat]::WM_CLOSE,[UIntPtr]::Zero,[IntPtr]::Zero) }
  Start-Sleep -Milliseconds 400; KillP
}

# ---- search ----
Write-Host '=== search ==='
foreach($x in @(@{c='preview.json';n='search'},@{c='preview.ko.json';n='search.ko'},@{c='preview.ja.json';n='search.ja'})){
  $dest=Join-Path $Out "$($x.n).png"
  if((Test-Path $dest) -and -not $Force){ Write-Host "skip $($x.n)"; continue }
  $st=StartMag $x.c; if(-not $st){ continue }
  [void][CapFeat]::SetForegroundWindow($st.Hwnd); Start-Sleep -Milliseconds 400
  [CapFeat]::Chord(0x11,0x46); Start-Sleep -Milliseconds 400
  [Windows.Forms.SendKeys]::SendWait('sunset'); Start-Sleep -Milliseconds 1400
  SaveCap $st.Hwnd $dest
  CloseMag $st
}

# ---- rename ----
Write-Host '=== rename ==='
foreach($rn in @(@{c='rename_camera.en.json';n='rename'},@{c='rename_camera.ko.json';n='rename.ko'},@{c='rename_camera.ja.json';n='rename.ja'})){
  $dest=Join-Path $Out "$($rn.n).png"
  if((Test-Path $dest) -and -not $Force){ Write-Host "skip $($rn.n)"; continue }
  $st=StartMag $rn.c; if(-not $st){ continue }
  [void][CapFeat]::ShowWindow($st.Hwnd,9); [void][CapFeat]::SetForegroundWindow($st.Hwnd); Start-Sleep -Milliseconds 1800
  $r=[MaguroCap]::Ext($st.Hwnd)
  [MaguroCap]::Click(($r.Left+520),($r.Top+300)); Start-Sleep -Milliseconds 500
  [CapFeat]::Chord(0x11,0x41); Start-Sleep -Milliseconds 600
  [CapFeat]::Chord(0x11,0x10,0x52); Start-Sleep -Milliseconds 2500
  $rh=WaitCls $st.Pid 'MaguroRename' 8
  if($rh -eq [IntPtr]::Zero){ Write-Host "FAIL rename $($rn.n)"; CloseMag $st; continue }
  $script:combos=New-Object Collections.Generic.List[IntPtr]
  $cb=[CapFeat+EnumProc]{ param($h,$l) $sb=New-Object Text.StringBuilder 64; [void][CapFeat]::GetClassName($h,$sb,64); if($sb.ToString() -match 'ComboBox'){ $script:combos.Add($h) }; return $true }
  [void][CapFeat]::EnumChildWindows($rh,$cb,[IntPtr]::Zero)
  foreach($c in $script:combos){
    $n=[int][CapFeat]::SendMessage($c,[CapFeat]::CB_GETCOUNT,[IntPtr]::Zero,[IntPtr]::Zero)
    for($i=0;$i -lt $n;$i++){
      $sb=New-Object Text.StringBuilder 256
      [void][CapFeat]::SendMessage($c,[CapFeat]::CB_GETLBTEXT,[IntPtr]$i,$sb)
      if($sb.ToString() -match 'Trip|여행|トリップ|Kyoto|京都'){
        [void][CapFeat]::SendMessage($c,[CapFeat]::CB_SETCURSEL,[IntPtr]$i,[IntPtr]::Zero)
        $cid=[CapFeat]::GetDlgCtrlID($c); $wp=[IntPtr]((1 -shl 16) -bor ($cid -band 0xFFFF))
        [void][CapFeat]::SendMessage($rh,0x0111,$wp,$c); break
      }
    }
  }
  Start-Sleep -Milliseconds 1400
  SaveCap $rh $dest
  CloseMag $st
}

# ---- tab_list ----
Write-Host '=== tab_list ==='
$dest=Join-Path $Out 'tab_list.png'
if(-not (Test-Path $dest) -or $Force){
  $st=StartMag 'tabs.json'
  if($st){
    [void][CapFeat]::SetForegroundWindow($st.Hwnd); Start-Sleep -Milliseconds 400
    [CapFeat]::Chord(0x11,0x10,0x4C); Start-Sleep -Milliseconds 800
    $tl=WaitCls $st.Pid 'MaguroTabList' 6
    if($tl -ne [IntPtr]::Zero){ SaveCap $tl $dest } else { Write-Host 'WARN no MaguroTabList' }
    CloseMag $st
  }
}

# ---- features ----
function SeedFeat([string]$lang){
  $py = @"
import json
from pathlib import Path
p=Path(r'$($Json.Replace('\','\\'))')
src=Path(r'$($CfgDir.Replace('\','\\'))')/'hero_light.json'
base=json.loads(src.read_text(encoding='utf-8'))
base['lang']='$lang'
base['theme']='light'; base['dark']=False
base['accent']='0A9BD6'; base['accent_dark']='0A9BD6'
root=r'$($Root.Replace('\','\\'))'
base['file_basket']=[
  root+r'\\demo-253\\basket-src\\demo-file-01.txt',
  root+r'\\demo-253\\basket-src\\demo-file-02.txt',
  root+r'\\demo-253\\basket-src\\demo-file-03.txt',
  root+r'\\demo-253\\basket-src\\demo-file-04.txt',
  root+r'\\demo-253\\basket-src\\demo-file-05.txt',
]
base['folder_watches']=[
  {'path': root+r'\\demo-readme-watch\\photos', 'subfolders': True, 'paused': False},
]
base['panes']=[{'index':0,'path':root+r'\\readme-demo','tint':1,'view':[4,16]},{'index':1,'path':'::{20D04FE0-3AEA-1069-A2D8-08002B30309D}','tint':5,'view':[4,16]}]
base['start_path']=root+r'\\readme-demo'
base['favorites']=[{'path':root+r'\\readme-demo','name':'readme-demo'}]
base['tabs']=[{'active':True,'locked':False,'name':'readme-demo','layout':base.get('layout',{'columns':[1,1],'colr':[0.5,0.5],'rowr':[[1.0],[1.0]],'active':0}),'panes':base['panes']}]
p.write_text(json.dumps(base,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print('seeded feat', '$lang')
"@
  $tmp=Join-Path $env:TEMP '_maguro_seed_feat.py'
  Set-Content $tmp $py -Encoding utf8
  python $tmp | Out-Host
}
foreach($pair in @(
  @{lang='en'; basket='feature_basket.png'; watch='feature_folder_watch.png'; wsv='feature_win_shift_v.png'; excel='feature_excel_readonly_tidy.png'},
  @{lang='ko'; basket='feature_basket.ko.png'; watch='feature_folder_watch.ko.png'; wsv='feature_win_shift_v.ko.png'; excel='feature_excel_readonly_tidy.ko.png'},
  @{lang='ja'; basket='feature_basket.ja.png'; watch='feature_folder_watch.ja.png'; wsv='feature_win_shift_v.ja.png'; excel='feature_excel_readonly_tidy.ja.png'}
)){
  Write-Host "=== features $($pair.lang) ==="
  $need = @($pair.basket,$pair.watch,$pair.wsv,$pair.excel) | Where-Object { $Force -or -not (Test-Path (Join-Path $Out $_)) }
  if($need.Count -eq 0){ Write-Host "skip features $($pair.lang)"; continue }
  SeedFeat $pair.lang
  KillP
  $p=Start-Process $Exe -WorkingDirectory $Portable -PassThru
  $st=@{ Proc=$p; Hwnd=(WaitCls ([uint32]$p.Id) 'MaguroMain' 20); Pid=[uint32]$p.Id }
  if($st.Hwnd -eq [IntPtr]::Zero){ KillP; continue }
  Start-Sleep -Milliseconds 1500
  [void][CapFeat]::ShowWindow($st.Hwnd,9); [void][CapFeat]::SetForegroundWindow($st.Hwnd); Start-Sleep -Milliseconds 800

  if($Force -or -not (Test-Path (Join-Path $Out $pair.basket))){
    [void][CapFeat]::PostMessage($st.Hwnd,[CapFeat]::WM_COMMAND,[UIntPtr]::new(1125),[IntPtr]::Zero)
    $hb=WaitCls $st.Pid 'MaguroBasket' 6
    if($hb -ne [IntPtr]::Zero){ Start-Sleep -Milliseconds 700; SaveCap $hb (Join-Path $Out $pair.basket); [void][CapFeat]::DestroyWindow($hb); Start-Sleep -Milliseconds 300 }
  }
  if($Force -or -not (Test-Path (Join-Path $Out $pair.watch))){
    [void][CapFeat]::PostMessage($st.Hwnd,[CapFeat]::WM_COMMAND,[UIntPtr]::new(1132),[IntPtr]::Zero)
    $hw=WaitCls $st.Pid 'MaguroWatch' 6
    if($hw -ne [IntPtr]::Zero){ Start-Sleep -Milliseconds 800; SaveCap $hw (Join-Path $Out $pair.watch); [void][CapFeat]::DestroyWindow($hw); Start-Sleep -Milliseconds 300 }
  }
  if($Force -or -not (Test-Path (Join-Path $Out $pair.wsv))){
    Set-Clipboard -Value (Join-Path $Root 'readme-demo')
    Start-Sleep -Milliseconds 300
    [void][CapFeat]::SetForegroundWindow($st.Hwnd)
    # Win+Shift+V
    [CapFeat]::Chord(0x5B,0x10,0x56)
    Start-Sleep -Milliseconds 1200
    SaveCap $st.Hwnd (Join-Path $Out $pair.wsv)
  }
  if($Force -or -not (Test-Path (Join-Path $Out $pair.excel))){
    [void][CapFeat]::PostMessage($st.Hwnd,[CapFeat]::WM_COMMAND,[UIntPtr]::new(1140),[IntPtr]::Zero)
    Start-Sleep -Milliseconds 800
    $ex=[IntPtr]::Zero
    foreach($cls in @('MaguroExcelTidy','MaguroExcel','MaguroTidy','MaguroAsk')){
      $l=[CapFeat]::FindOwned($st.Pid,$cls); if($l.Count -gt 0){ $ex=$l[0]; break }
    }
    if($ex -ne [IntPtr]::Zero){ SaveCap $ex (Join-Path $Out $pair.excel) }
    else { Write-Host "no excel tidy $($pair.lang)" }
  }
  CloseMag $st
}
Write-Host 'capture-features DONE'
