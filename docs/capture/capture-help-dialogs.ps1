# Help-site strips + workspaces ask + copy_progress + dialog_jump.
# Env: MAGURO_PORTABLE, MAGURO_SHOT_DIR, MAGURO_HELP_DEMO (optional, default sibling help-demo)
# MAGURO_FORCE=1 overwrites. Fake paths only under help-demo.
$ErrorActionPreference='Continue'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'cap-lib.ps1')
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$Portable = $env:MAGURO_PORTABLE; if (-not $Portable) { throw 'MAGURO_PORTABLE' }
$Out = $env:MAGURO_SHOT_DIR; if (-not $Out) { throw 'MAGURO_SHOT_DIR' }
$Demo = $env:MAGURO_HELP_DEMO; if (-not $Demo) { $Demo = Join-Path (Split-Path $Portable -Parent) 'help-demo' }
$Exe = Join-Path $Portable 'maguro.exe'
$Json = Join-Path $Portable 'maguro.json'
$Force = ($env:MAGURO_FORCE -eq '1')
New-Item -ItemType Directory -Force $Out | Out-Null

if (-not ('CapHelp' -as [type])) {
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic;
public static class CapHelp {
  public const uint WM_CLOSE=0x0010, WM_COMMAND=0x0111;
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc lp, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder sb, int n);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, UIntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint f, UIntPtr e);
  public static readonly IntPtr HWND_TOPMOST=new IntPtr(-1), HWND_NOTOPMOST=new IntPtr(-2);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
  public static void Chord(params byte[] keys){ throw new InvalidOperationException("FORBIDDEN Chord — use MaguroCap.ChordTo"); }
  public static void Tap(byte k){ throw new InvalidOperationException("FORBIDDEN Tap — use MaguroCap.TapTo"); }
  public static string Cls(IntPtr h){ var sb=new StringBuilder(64); GetClassName(h,sb,64); return sb.ToString(); }
  public static string Title(IntPtr h){ var sb=new StringBuilder(512); GetWindowText(h,sb,512); return sb.ToString(); }
  public static List<IntPtr> FindOwned(uint pid,string cls){
    var f=new List<IntPtr>();
    EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid||!IsWindowVisible(h)) return true; if(Cls(h)==cls) f.Add(h); return true; }, IntPtr.Zero);
    return f;
  }
  public static List<IntPtr> FindByClass(string cls){
    var f=new List<IntPtr>();
    EnumWindows((h,l)=>{ if(!IsWindowVisible(h)) return true; if(Cls(h)==cls) f.Add(h); return true; }, IntPtr.Zero);
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
  do { $l=[CapHelp]::FindOwned($pidMag,$cls); if($l.Count -gt 0){ return $l[0] }; Start-Sleep -Milliseconds 200 } while((Get-Date) -lt $d)
  return [IntPtr]::Zero
}
function Suf([string]$lang){ switch($lang){ 'ko' {'.ko'} 'ja' {'.ja'} default {''} } }
function SaveCap([IntPtr]$h,[string]$path){
  if((Test-Path $path) -and -not $Force){ Write-Host "skip $path"; return $true }
  try { Cap-MaguroWindow $h $path; return $true } catch { Write-Host "CAP FAIL $path : $_"; return $false }
}
function CapRegionMain([IntPtr]$main,[int]$maxH,[string]$path){
  if((Test-Path $path) -and -not $Force){ Write-Host "skip $path"; return }
  $r=[MaguroCap]::Ext($main)
  Open-MaguroBackdrop $r.Left $r.Top $r.Right $r.Bottom
  [void][MaguroCap]::SetWindowPos($main,[MaguroCap]::HWND_TOPMOST,0,0,0,0,0x0003)
  [void][MaguroCap]::SetForegroundWindow($main); Start-Sleep -Milliseconds 300
  $r=[MaguroCap]::Ext($main)
  $w=$r.Right-$r.Left; $hh=$r.Bottom-$r.Top
  if($maxH -gt 0 -and $hh -gt $maxH){ $hh=$maxH }
  $bmp=New-Object Drawing.Bitmap $w,$hh
  $g=[Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.Left,$r.Top,0,0,(New-Object Drawing.Size $w,$hh)); $g.Dispose()
  [void][MaguroCap]::SetWindowPos($main,[MaguroCap]::HWND_NOTOPMOST,0,0,0,0,0x0003)
  Close-MaguroBackdrop
  $edge=Test-MaguroEdges $bmp
  if(-not $edge.Ok){ $bmp.Dispose(); Write-Host "region REJECT $path"; return }
  $bmp.Save($path,[Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
  Write-Host ("OK-region {0} {1}x{2}" -f $path,$w,$hh)
}
function Seed([string]$lang,[string]$mode){
  $path0 = if($mode -eq 'copy'){ (Join-Path $Demo 'src').Replace('\','\\') } else { (Join-Path $Demo 'a').Replace('\','\\') }
  $path1 = (Join-Path $Demo 'dst').Replace('\','\\')
  $a=(Join-Path $Demo 'a').Replace('\','\\'); $b=(Join-Path $Demo 'b').Replace('\','\\'); $c=(Join-Path $Demo 'c').Replace('\','\\')
  $py = @"
import json, pathlib
p=pathlib.Path(r'$($Json.Replace('\','\\'))')
d={
  'version':1,'lang':'$lang','theme':'light','dark':False,
  'accent':'0A9BD6','accent_dark':'0A9BD6','use_everything':False,'ev_asked':True,
  'dialog_jump':True,'fast_copy':True,'confirm':False,
  'window':{'left':40,'top':30,'right':1266,'bottom':853,'maximized':False},
  'side':0,'side_dpi':96,'tree':False,'explorer_pins_open':False,'explorer_files_open':False,
  'favorites':[],'recent':[],'bar':True,'stat':True,'ext':True,'single':True,
  'layout':{'columns':[1,1],'colr':[0.5,0.5],'rowr':[[1.0],[1.0]],'active':0},
  'panes':[{'index':0,'path':r'$path0','view':[4,16]},{'index':1,'path':r'$path1','view':[4,16]}],
  'workspaces':[{'slot':1,'name':'Desk'},{'slot':2,'name':'Code'}]
}
if '$mode' != 'copy':
  d['panes'][0]['folders']=[r'$a',r'$b',r'$c']
  d['panes'][0]['folder']=0
p.write_text(json.dumps(d,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print('seeded', '$lang', '$mode')
"@
  $tmp = Join-Path $env:TEMP "_maguro_seed_help.py"
  Set-Content -Path $tmp -Value $py -Encoding utf8
  python $tmp | Out-Host
}
function CloseNotepad {
  Get-Process notepad -EA SilentlyContinue | Stop-Process -Force -EA SilentlyContinue
  Start-Sleep -Milliseconds 300
}

foreach($lang in @('en','ko','ja')){
  $sfx=Suf $lang
  Write-Host "==== help LANG $lang ===="
  KillP; CloseNotepad
  Seed $lang 'main'
  Get-ChildItem (Join-Path $Demo 'dst') -File -EA SilentlyContinue | Remove-Item -Force
  $proc=Start-Process $Exe -WorkingDirectory $Portable -PassThru
  $pidMag=[uint32]$proc.Id
  $main=WaitCls $pidMag 'MaguroMain' 25
  if($main -eq [IntPtr]::Zero){ Write-Host "no main $lang"; continue }
  [void][CapHelp]::SetForegroundWindow($main); Start-Sleep -Milliseconds 1200
  CapRegionMain $main 160 (Join-Path $Out "address_toolbar$sfx.png")
  CapRegionMain $main 420 (Join-Path $Out "pane_tabs$sfx.png")

  # workspaces name ask: Ctrl+Alt+Shift+1 then WM_COMMAND 1920
  [void][CapHelp]::SetForegroundWindow($main)
  [MaguroCap]::ChordTo($main, [uint32]$pidMag, 0x11,0x12,0x10,0x31)
  Start-Sleep -Milliseconds 700
  [void][CapHelp]::PostMessage($main,[CapHelp]::WM_COMMAND,[UIntPtr]::new(1920),[IntPtr]::Zero)
  $ask=WaitCls $pidMag 'MaguroAsk' 8
  if($ask -ne [IntPtr]::Zero){ SaveCap $ask (Join-Path $Out "workspaces$sfx.png") | Out-Null; [void][CapHelp]::PostMessage($ask,[CapHelp]::WM_CLOSE,[UIntPtr]::Zero,[IntPtr]::Zero) }
  else { Write-Host "no workspaces $lang" }

  # In-app shortcut list. F1 opens the browser Help site, which is not this crop.
  # ID_KEYS = 1101. Ctrl+/ (VK_OEM_2) is easy to miss when the shell has focus.
  [void][CapHelp]::PostMessage($main,[CapHelp]::WM_COMMAND,[UIntPtr]::new(1101),[IntPtr]::Zero)
  Start-Sleep -Milliseconds 700
  $hh=WaitCls $pidMag 'MaguroKeys' 6
  if($hh -ne [IntPtr]::Zero){ SaveCap $hh (Join-Path $Out "help$sfx.png") | Out-Null }
  else { Write-Host "no MaguroKeys $lang" }

  # dialog_jump via Notepad Open + Ctrl+Alt+G
  CloseNotepad
  $null=Start-Process notepad -PassThru
  Start-Sleep -Milliseconds 1000
  $npH=[IntPtr]::Zero
  foreach($cand in ([CapHelp]::FindByClass('Notepad') + [CapHelp]::FindByClass('NotepadWindow') + [CapHelp]::FindByClass('ApplicationFrameWindow'))){
    $t=[CapHelp]::Title($cand)
    if($t -match 'Notepad|메모장|ノート|Untitled|제목 없음'){ $npH=$cand; break }
  }
  if($npH -eq [IntPtr]::Zero){ $all=[CapHelp]::FindByClass('Notepad'); if($all.Count -gt 0){ $npH=$all[0] } }
  if($npH -ne [IntPtr]::Zero){
    [void][CapHelp]::SetForegroundWindow($npH); Start-Sleep -Milliseconds 300
    [MaguroCap]::ChordTo($main, [uint32]$pidMag, 0x11,0x4F); Start-Sleep -Milliseconds 1000
    $dlg=[IntPtr]::Zero
    $deadline=(Get-Date).AddSeconds(8)
    do {
      foreach($h in [CapHelp]::FindByClass('#32770')){
        $t=[CapHelp]::Title($h)
        if($t -match 'Open|열기|開|開き|ファイル'){ $dlg=$h; break }
      }
      if($dlg -ne [IntPtr]::Zero){ break }
      Start-Sleep -Milliseconds 200
    } while((Get-Date) -lt $deadline)
    if($dlg -ne [IntPtr]::Zero){
      [void][CapHelp]::SetForegroundWindow($dlg); Start-Sleep -Milliseconds 300
      [MaguroCap]::ChordTo($main, [uint32]$pidMag, 0x11,0x12,0x47) # Ctrl+Alt+G
      # MaguroDlgJump is a 1x1 host. The picture is the open menu (#32768).
      $menu=[IntPtr]::Zero
      $deadline=(Get-Date).AddSeconds(4)
      do {
        foreach($h in [CapHelp]::FindByClass('#32768')){
          $rr=[MaguroCap]::Ext($h)
          $mw=$rr.Right-$rr.Left; $mh=$rr.Bottom-$rr.Top
          if($mw -gt 120 -and $mh -gt 40){ $menu=$h; break }
        }
        if($menu -ne [IntPtr]::Zero){ break }
        Start-Sleep -Milliseconds 100
      } while((Get-Date) -lt $deadline)
      if($menu -ne [IntPtr]::Zero){
        try { Cap-MaguroWindow $menu (Join-Path $Out "dialog_jump$sfx.png") -Still }
        catch { Write-Host "CAP FAIL dialog_jump$sfx : $_" }
      } else { Write-Host "no jump menu $lang" }
      [MaguroCap]::TapTo($main, [uint32]$pidMag, 0x1B); Start-Sleep -Milliseconds 200; [MaguroCap]::TapTo($main, [uint32]$pidMag, 0x1B)
    } else { Write-Host "no Open dialog $lang" }
  } else { Write-Host "no Notepad $lang" }
  CloseNotepad

  [void][CapHelp]::PostMessage($main,[CapHelp]::WM_CLOSE,[UIntPtr]::Zero,[IntPtr]::Zero)
  Start-Sleep -Milliseconds 500; KillP

  # copy_progress
  Seed $lang 'copy'
  Get-ChildItem (Join-Path $Demo 'dst') -File -EA SilentlyContinue | Remove-Item -Force
  $proc=Start-Process $Exe -WorkingDirectory $Portable -PassThru
  $pidMag=[uint32]$proc.Id
  $main=WaitCls $pidMag 'MaguroMain' 25
  if($main -eq [IntPtr]::Zero){ continue }
  [void][CapHelp]::SetForegroundWindow($main); Start-Sleep -Milliseconds 1000
  $r=[MaguroCap]::Ext($main)
  $cx=[int](($r.Left+$r.Right)/2); $cy=[int](($r.Top+$r.Bottom)/2)
  Open-MaguroBackdrop ($cx-280) ($cy-120) ($cx+280) ($cy+120)
  [MaguroCap]::ClickClient($main, 300, 400); Start-Sleep -Milliseconds 300
  [MaguroCap]::ChordTo($main, [uint32]$pidMag, 0x11,0x41); Start-Sleep -Milliseconds 300
  [MaguroCap]::ChordTo($main, [uint32]$pidMag, 0x11,0x43); Start-Sleep -Milliseconds 300
  [MaguroCap]::ClickClient($main, 900, 400); Start-Sleep -Milliseconds 300
  # Local paste stays on IFileOperation. Fast paste is the Maguro progress window.
  [MaguroCap]::ChordTo($main, [uint32]$pidMag, 0x10,0x79)
  Start-Sleep -Milliseconds 600
  if(-not (Invoke-MaguroMenuName 'Fast paste|고속 붙여넣기|高速貼り付け')){ Write-Host "no fast-paste row $lang" }
  $fc=[IntPtr]::Zero
  $deadline=(Get-Date).AddSeconds(15)
  do {
    $list=[CapHelp]::FindOwned($pidMag,'MaguroFastCopy')
    if($list.Count -eq 0){ $list=[CapHelp]::FindByClass('MaguroFastCopy') }
    if($list.Count -gt 0){
      $br=[MaguroCap]::Ext($list[0])
      if(($br.Right-$br.Left) -ge 80){ $fc=$list[0]; break }
    }
    Start-Sleep -Milliseconds 40
  } while((Get-Date) -lt $deadline)
  if($fc -ne [IntPtr]::Zero){ SaveCap $fc (Join-Path $Out "copy_progress$sfx.png") | Out-Null }
  else { Write-Host "no FastCopy $lang" }
  Close-MaguroBackdrop; KillP
}
Write-Host 'capture-help-dialogs DONE'

