# Maguro README/help capture helpers (portable). No personal paths or secrets.
# Dot-source from a capture script after setting $MaguroCaptureRoot (output dir).
# Requires: System.Drawing; optional FastEdge.dll beside this script for edge scan.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing | Out-Null

if (-not ('MaguroCap' -as [type])) {
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class MaguroCap {
  public const int DWMWA_EXTENDED_FRAME_BOUNDS = 9;
  public const uint WM_CLOSE=0x0010, WM_COMMAND=0x0111, WM_SETTEXT=0x000C;
  public const uint WM_LBUTTONDOWN=0x0201, WM_LBUTTONUP=0x0202;
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT p, int cb);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern bool RedrawWindow(IntPtr h, IntPtr r, IntPtr ur, uint f);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc lp, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc lp, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int n);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, UIntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, string l);
  [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr hdc);
  [DllImport("gdi32.dll")] public static extern int GetDeviceCaps(IntPtr hdc, int idx);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }
  public static readonly IntPtr HWND_TOPMOST = new IntPtr(-1);
  public static readonly IntPtr HWND_NOTOPMOST = new IntPtr(-2);
  public static RECT Ext(IntPtr h){
    RECT r;
    if(DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, out r, Marshal.SizeOf(typeof(RECT))) != 0)
      GetWindowRect(h, out r);
    return r;
  }
  public static string Cls(IntPtr h){ var sb=new StringBuilder(64); GetClassName(h,sb,64); return sb.ToString(); }
  public static IntPtr FindCls(uint procId, string cls){
    IntPtr found=IntPtr.Zero;
    EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p);
      if(p!=procId||!IsWindowVisible(h)) return true;
      if(Cls(h)==cls){ found=h; return false;} return true;}, IntPtr.Zero);
    return found;
  }
  public static IntPtr FindClsAny(uint procId, string cls){
    IntPtr found=IntPtr.Zero;
    EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p);
      if(p!=procId) return true;
      if(Cls(h)==cls){ found=h; return false;} return true;}, IntPtr.Zero);
    return found;
  }
  public static void Chord(params byte[] keys){
    foreach(var k in keys) keybd_event(k,0,0,UIntPtr.Zero);
    System.Threading.Thread.Sleep(40);
    for(int i=keys.Length-1;i>=0;i--) keybd_event(keys[i],0,2,UIntPtr.Zero);
  }
  public static void Tap(byte k){ keybd_event(k,0,0,UIntPtr.Zero); System.Threading.Thread.Sleep(30); keybd_event(k,0,2,UIntPtr.Zero); }
  public static void Click(int x,int y){
    SetCursorPos(x,y); System.Threading.Thread.Sleep(50);
    mouse_event(2,0,0,0,UIntPtr.Zero); System.Threading.Thread.Sleep(40);
    mouse_event(4,0,0,0,UIntPtr.Zero);
  }
  public static bool SetEdit(IntPtr dlg, int id, string text){
    IntPtr found=IntPtr.Zero;
    EnumChildWindows(dlg,(h,l)=>{ if(GetDlgCtrlID(h)==id){ found=h; return false;} return true;}, IntPtr.Zero);
    if(found==IntPtr.Zero) return false;
    SendMessage(found, WM_SETTEXT, IntPtr.Zero, text);
    return true;
  }
  public static void ClickClient(IntPtr h, int x, int y){
    IntPtr lp = (IntPtr)((y << 16) | (x & 0xFFFF));
    PostMessage(h, WM_LBUTTONDOWN, (UIntPtr)1, lp);
    System.Threading.Thread.Sleep(40);
    PostMessage(h, WM_LBUTTONUP, UIntPtr.Zero, lp);
  }
  public static int Dpi(IntPtr h){ IntPtr dc=GetDC(h); int dpi=GetDeviceCaps(dc,88); ReleaseDC(h,dc); return dpi>0?dpi:96; }
}
"@
}

function Test-MaguroEdges([Drawing.Bitmap]$bmp) {
  # Left edge: reject if >=3 dark pixels or >5% of height. Other edges: >5% strip only.
  $w=$bmp.Width; $h=$bmp.Height
  function Dark([Drawing.Color]$c){ return ($c.R -lt 40 -and $c.G -lt 40 -and $c.B -lt 40) }
  $left=0; for($y=0;$y -lt $h;$y++){ if(Dark $bmp.GetPixel(0,$y)){ $left++ } }
  $right=0; for($y=0;$y -lt $h;$y++){ if(Dark $bmp.GetPixel($w-1,$y)){ $right++ } }
  $top=0; for($x=0;$x -lt $w;$x++){ if(Dark $bmp.GetPixel($x,0)){ $top++ } }
  $bot=0; for($x=0;$x -lt $w;$x++){ if(Dark $bmp.GetPixel($x,$h-1)){ $bot++ } }
  $ok = ($left -lt 3) -and ($left -le [int](0.05*$h)) -and ($right -le [int](0.05*$h)) -and ($top -le [int](0.05*$w)) -and ($bot -le [int](0.05*$w))
  return @{ Ok=$ok; LeftBlack=$left; RightBlack=$right; TopBlack=$top; BotBlack=$bot; Dark=($left+$right+$top+$bot) }
}

function Open-MaguroBackdrop([int]$L,[int]$T,[int]$R,[int]$B) {
  # Spawn a temporary white top-level window behind the target (see white-backdrop.ps1).
  $script:MaguroBd = Join-Path $env:TEMP ("maguro-bd-" + [guid]::NewGuid().ToString('n'))
  $ps1 = Join-Path $PSScriptRoot 'white-backdrop.ps1'
  if (-not (Test-Path $ps1)) { return }
  $script:MaguroBdProc = Start-Process -FilePath powershell -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$ps1,$L,$T,$R,$B,$script:MaguroBd) -PassThru -WindowStyle Hidden
  Start-Sleep -Milliseconds 200
}

function Close-MaguroBackdrop {
  if ($script:MaguroBdProc) {
    try { if (-not $script:MaguroBdProc.HasExited) { $script:MaguroBdProc.Kill() } } catch {}
    $script:MaguroBdProc = $null
  }
  if ($script:MaguroBd) {
    Remove-Item $script:MaguroBd -Force -ErrorAction SilentlyContinue
    Remove-Item ($script:MaguroBd + '.pspid') -Force -ErrorAction SilentlyContinue
  }
}

function Cap-MaguroWindow([IntPtr]$h, [string]$path, [int]$retries=4) {
  if ($h -eq [IntPtr]::Zero) { throw "null hwnd $path" }
  $lastErr = $null
  for ($attempt=1; $attempt -le $retries; $attempt++) {
    try {
      $r = [MaguroCap]::Ext($h)
      Open-MaguroBackdrop $r.Left $r.Top $r.Right $r.Bottom
      [void][MaguroCap]::SetWindowPos($h, [MaguroCap]::HWND_TOPMOST, 0,0,0,0, 0x0003)
      [void][MaguroCap]::BringWindowToTop($h)
      [void][MaguroCap]::SetForegroundWindow($h)
      # MaguroSettings often stays unpainted until invalidate+update; avoid ShowWindow (shell-ext deadlock).
      [void][MaguroCap]::RedrawWindow($h, [IntPtr]::Zero, [IntPtr]::Zero, 0x0105)
      Start-Sleep -Milliseconds 450
      $r = [MaguroCap]::Ext($h)
      $w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
      if ($w -lt 40 -or $hh -lt 40) { throw "bad rect $w x $hh" }
      $bmp = New-Object Drawing.Bitmap $w,$hh
      $g = [Drawing.Graphics]::FromImage($bmp)
      $g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object Drawing.Size $w,$hh))
      $g.Dispose()
      [void][MaguroCap]::SetWindowPos($h, [MaguroCap]::HWND_NOTOPMOST, 0,0,0,0, 0x0003)
      Close-MaguroBackdrop
      $navPx = $bmp.GetPixel([Math]::Min(80,$w-1), [Math]::Min(200,$hh-1))
      $bodyPx = $bmp.GetPixel([int]($w*0.55), [int]($hh*0.45))
      $solidWhite = ($navPx.R -gt 250 -and $navPx.G -gt 250 -and $navPx.B -gt 250 -and $bodyPx.R -gt 250 -and $bodyPx.G -gt 250 -and $bodyPx.B -gt 250)
      if ($solidWhite -and $w -gt 500) {
        $bmp.Dispose(); throw ("solid_white_client attempt={0}" -f $attempt)
      }
      $edge = Test-MaguroEdges $bmp
      if (-not $edge.Ok) {
        $bmp.Dispose(); throw ("dark_edges L={0} attempt={1}" -f $edge.LeftBlack,$attempt)
      }
      $dir = Split-Path $path -Parent
      if ($dir) { New-Item -ItemType Directory -Force $dir | Out-Null }
      $bmp.Save($path, [Drawing.Imaging.ImageFormat]::Png)
      $bmp.Dispose()
      Write-Host ("OK {0} {1}x{2} {3}KB" -f $path,$w,$hh,[int]((Get-Item $path).Length/1KB))
      return
    } catch {
      $lastErr = $_; Write-Host "RETRY $path : $_"; Close-MaguroBackdrop; Start-Sleep -Milliseconds 400
    }
  }
  throw "Cap-MaguroWindow FAILED $path : $lastErr"
}

function Click-MaguroSettingsPage([IntPtr]$h, [int]$page) {
  # Page index matches settings::PAGES (0=General ... 2=PanesView ...). Use client WM_LBUTTON* — screen mouse often misses.
  $dpi = [MaguroCap]::Dpi($h)
  $x = [int](8*$dpi/96) + [int]((220*$dpi/96)/2)
  $y = [int]((12+32)*$dpi/96) + [int](8*$dpi/96) + $page*[int](36*$dpi/96) + [int]((36*$dpi/96)/2)
  [MaguroCap]::ClickClient($h, $x, $y)
}
