# Maguro README/help capture helpers (portable). No personal paths or secrets.
# Dot-source from a capture script after setting $MaguroCaptureRoot (output dir).
# KEYBOARD SAFETY: NEVER use SendKeys, SendInput, or keybd_event against the global
# foreground. All text/keys go via WM_SETTEXT / WM_CHAR / WM_KEY* PostMessage to an
# HWND owned by our Maguro PID; Abort if foreground is not our Maguro.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing | Out-Null

# Force reload of MaguroCap if an older unsafe build is already loaded in this session.
if ('MaguroCap' -as [type]) {
  Write-Warning 'MaguroCap already loaded in this AppDomain — start a fresh powershell.exe to pick up ChordTo safety APIs.'
}

if (-not ('MaguroCap' -as [type])) {
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class MaguroCap {
  public const int DWMWA_EXTENDED_FRAME_BOUNDS = 9;
  public const uint WM_CLOSE=0x0010, WM_COMMAND=0x0111, WM_SETTEXT=0x000C;
  public const uint WM_LBUTTONDOWN=0x0201, WM_LBUTTONUP=0x0202;
  public const uint WM_KEYDOWN=0x0100, WM_KEYUP=0x0101, WM_CHAR=0x0102;
  public const uint WM_SYSKEYDOWN=0x0104, WM_SYSKEYUP=0x0105;
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT p, int cb);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint idAttach, uint idTo, bool fAttach);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder sb, int n);
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
    int hr = DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, out r, Marshal.SizeOf(typeof(RECT)));
    if(hr != 0 || (r.Right - r.Left) < 40 || (r.Bottom - r.Top) < 40)
      GetWindowRect(h, out r);
    return r;
  }
  public static string Cls(IntPtr h){ var sb=new StringBuilder(64); GetClassName(h,sb,64); return sb.ToString(); }
  public static string Title(IntPtr h){ var sb=new StringBuilder(256); GetWindowText(h,sb,256); return sb.ToString(); }
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
  public static bool IsOwnWindow(IntPtr h, uint expectedPid){
    if(h==IntPtr.Zero || !IsWindow(h)) return false;
    uint p; GetWindowThreadProcessId(h, out p);
    return p==expectedPid;
  }
  public static bool IsOwnMaguro(IntPtr h, uint expectedPid){
    if(!IsOwnWindow(h, expectedPid)) return false;
    string t = Title(h); string c = Cls(h);
    return (t!=null && t.IndexOf("Maguro", StringComparison.OrdinalIgnoreCase)>=0)
        || (c!=null && c.IndexOf("Maguro", StringComparison.OrdinalIgnoreCase)>=0);
  }
  public static void AssertOwnMaguroForeground(uint expectedPid){
    IntPtr fg = GetForegroundWindow();
    if(!IsOwnMaguro(fg, expectedPid))
      throw new InvalidOperationException("ABORT: foreground is not our Maguro (pid="+expectedPid+" fg="+fg+" title='"+Title(fg)+"' cls='"+Cls(fg)+"'). Refusing keystroke.");
  }
  public static void ForceOwnForeground(IntPtr h, uint expectedPid){
    AssertOwnHwnd(h, expectedPid);
    IntPtr fg = GetForegroundWindow();
    uint fgPid; uint fgTid = GetWindowThreadProcessId(fg, out fgPid);
    uint myTid = GetCurrentThreadId();
    if(fgTid != 0 && fgTid != myTid) AttachThreadInput(myTid, fgTid, true);
    BringWindowToTop(h);
    SetForegroundWindow(h);
    if(fgTid != 0 && fgTid != myTid) AttachThreadInput(myTid, fgTid, false);
    System.Threading.Thread.Sleep(150);
    // Retry once if a non-Maguro window still holds FG
    if(!IsOwnMaguro(GetForegroundWindow(), expectedPid)){
      BringWindowToTop(h);
      SetForegroundWindow(h);
      System.Threading.Thread.Sleep(200);
    }
  }
  public static void AssertOwnHwnd(IntPtr h, uint expectedPid){
    if(!IsOwnWindow(h, expectedPid))
      throw new InvalidOperationException("ABORT: hwnd not owned by Maguro pid="+expectedPid);
  }
  static void PostKey(IntPtr h, byte vk, bool down, bool sys){
    uint msg = sys ? (down?WM_SYSKEYDOWN:WM_SYSKEYUP) : (down?WM_KEYDOWN:WM_KEYUP);
    IntPtr lp = (IntPtr)(1 | (down?0:((1<<30)|(1<<31))));
    PostMessage(h, msg, (UIntPtr)vk, lp);
  }
  // ChordTo: AFTER FG is verified as our Maguro, deliver real keybd_event (accelerators need it).
  // NEVER call without AssertOwnMaguroForeground — that gate is what prevents chat leaks.
  public static void ChordTo(IntPtr h, uint expectedPid, params byte[] keys){
    AssertOwnHwnd(h, expectedPid);
    ForceOwnForeground(h, expectedPid);
    AssertOwnMaguroForeground(expectedPid);
    foreach(var k in keys){
      AssertOwnMaguroForeground(expectedPid);
      keybd_event(k,0,0,UIntPtr.Zero);
      System.Threading.Thread.Sleep(25);
    }
    System.Threading.Thread.Sleep(40);
    for(int i=keys.Length-1;i>=0;i--){
      AssertOwnMaguroForeground(expectedPid);
      keybd_event(keys[i],0,2,UIntPtr.Zero);
      System.Threading.Thread.Sleep(25);
    }
  }
  public static void TapTo(IntPtr h, uint expectedPid, byte k){ ChordTo(h, expectedPid, k); }
  public static void TypeChars(IntPtr h, uint expectedPid, string text){
    AssertOwnHwnd(h, expectedPid);
    ForceOwnForeground(h, expectedPid);
    AssertOwnMaguroForeground(expectedPid);
    foreach(char ch in text){
      PostMessage(h, WM_CHAR, (UIntPtr)ch, IntPtr.Zero);
      System.Threading.Thread.Sleep(15);
    }
  }
  public static bool SetChildText(IntPtr parent, uint expectedPid, string clsWant, string text){
    AssertOwnHwnd(parent, expectedPid);
    IntPtr found=IntPtr.Zero;
    EnumChildWindows(parent,(h,l)=>{
      uint p; GetWindowThreadProcessId(h,out p);
      if(p!=expectedPid) return true;
      if(!string.IsNullOrEmpty(clsWant) && Cls(h)==clsWant){ found=h; return false; }
      return true;
    }, IntPtr.Zero);
    if(found==IntPtr.Zero){
      EnumChildWindows(parent,(h,l)=>{
        uint p; GetWindowThreadProcessId(h,out p);
        if(p!=expectedPid) return true;
        string c=Cls(h);
        if(c=="Edit" || c.IndexOf("Edit",StringComparison.OrdinalIgnoreCase)>=0){ found=h; return false; }
        return true;
      }, IntPtr.Zero);
    }
    if(found==IntPtr.Zero) return false;
    SendMessage(found, WM_SETTEXT, IntPtr.Zero, text);
    return true;
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
  // Forbidden shims — throw so any leftover caller fails closed.
  public static void Chord(params byte[] keys){
    throw new InvalidOperationException("FORBIDDEN Chord(): use MaguroCap.ChordTo(hwnd, pid, keys). Global keybd_event leaked into other apps.");
  }
  public static void Tap(byte k){
    throw new InvalidOperationException("FORBIDDEN Tap(): use MaguroCap.TapTo(hwnd, pid, vk).");
  }
  public static void Click(int x,int y){
    throw new InvalidOperationException("FORBIDDEN Click(screen): use MaguroCap.ClickClient(hwnd, clientX, clientY).");
  }
  public static int Dpi(IntPtr h){ IntPtr dc=GetDC(h); int dpi=GetDeviceCaps(dc,88); ReleaseDC(h,dc); return dpi>0?dpi:96; }
}
"@
}

# --- KEYBOARD SAFETY helpers (never SendKeys / SendInput / keybd_event) ---
function Assert-MaguroForeground([uint32]$ProcessId) {
  [MaguroCap]::AssertOwnMaguroForeground($ProcessId)
}
function Send-MaguroChord([IntPtr]$Hwnd, [uint32]$ProcessId, [byte[]]$Keys) {
  [MaguroCap]::ChordTo($Hwnd, $ProcessId, $Keys)
}
function Send-MaguroTap([IntPtr]$Hwnd, [uint32]$ProcessId, [byte]$Vk) {
  [MaguroCap]::TapTo($Hwnd, $ProcessId, $Vk)
}
function Send-MaguroText([IntPtr]$Hwnd, [uint32]$ProcessId, [string]$Text) {
  if (-not [MaguroCap]::SetChildText($Hwnd, $ProcessId, $null, $Text)) {
    [MaguroCap]::TypeChars($Hwnd, $ProcessId, $Text)
  }
}

function Test-MaguroEdges([Drawing.Bitmap]$bmp) {
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

function Crop-MaguroFrame([Drawing.Bitmap]$bmp) {
  if ((Test-MaguroEdges $bmp).Ok) { return $bmp }
  for ($inset=1; $inset -le 8; $inset++) {
    $nw = $bmp.Width - (2 * $inset)
    $nh = $bmp.Height - (2 * $inset)
    if ($nw -lt 80 -or $nh -lt 80) { break }
    $rect = New-Object Drawing.Rectangle $inset, $inset, $nw, $nh
    $crop = $bmp.Clone($rect, $bmp.PixelFormat)
    if ((Test-MaguroEdges $crop).Ok) {
      Write-Host ("frame-inset {0}px" -f $inset)
      $bmp.Dispose()
      return $crop
    }
    $crop.Dispose()
  }
  return $null
}

function Invoke-MaguroMenuNameUia([string]$pattern) {
  Add-Type -AssemblyName UIAutomationClient | Out-Null
  Add-Type -AssemblyName UIAutomationTypes | Out-Null
  $root = [System.Windows.Automation.AutomationElement]::RootElement
  $menuType = New-Object System.Windows.Automation.PropertyCondition (
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::Menu)
  $menus = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $menuType)
  $itemType = New-Object System.Windows.Automation.PropertyCondition (
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::MenuItem)
  foreach ($m in $menus) {
    foreach ($it in $m.FindAll([System.Windows.Automation.TreeScope]::Descendants, $itemType)) {
      $name = ''
      try { $name = $it.Current.Name } catch { continue }
      if ($name -and $name -match $pattern) {
        $inv = $it.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
        $inv.Invoke()
        return $true
      }
    }
  }
  return $false
}

function Cap-MaguroWindow([IntPtr]$h, [string]$path, [int]$retries=4, [switch]$Still) {
  if ($h -eq [IntPtr]::Zero) { throw "null hwnd $path" }
  $lastErr = $null
  for ($attempt=1; $attempt -le $retries; $attempt++) {
    try {
      $r = [MaguroCap]::Ext($h)
      if (-not $Still) {
        Open-MaguroBackdrop $r.Left $r.Top $r.Right $r.Bottom
        [void][MaguroCap]::SetWindowPos($h, [MaguroCap]::HWND_TOPMOST, 0,0,0,0, 0x0013)
        [void][MaguroCap]::SetForegroundWindow($h)
      }
      [void][MaguroCap]::RedrawWindow($h, [IntPtr]::Zero, [IntPtr]::Zero, 0x0105)
      Start-Sleep -Milliseconds 450
      $r = [MaguroCap]::Ext($h)
      $w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
      if ($w -lt 40 -or $hh -lt 40) { throw "bad rect $w x $hh" }
      $bmp = New-Object Drawing.Bitmap $w,$hh
      $g = [Drawing.Graphics]::FromImage($bmp)
      $g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object Drawing.Size $w,$hh))
      $g.Dispose()
      if (-not $Still) {
        [void][MaguroCap]::SetWindowPos($h, [MaguroCap]::HWND_NOTOPMOST, 0,0,0,0, 0x0003)
        Close-MaguroBackdrop
      }
      $painted = 0
      foreach ($fy in @(0.04, 0.12, 0.28, 0.45, 0.62, 0.82)) {
        foreach ($fx in @(0.08, 0.22, 0.40, 0.58, 0.75, 0.90)) {
          $px = $bmp.GetPixel([Math]::Min($w-1,[int]($w * $fx)), [Math]::Min($hh-1,[int]($hh * $fy)))
          if ($px.R -lt 250 -or $px.G -lt 250 -or $px.B -lt 250) { $painted++ }
        }
      }
      if ($painted -lt 2 -and $w -gt 500) {
        $bmp.Dispose(); throw ("solid_white_client painted={0} attempt={1}" -f $painted,$attempt)
      }
      $framed = Crop-MaguroFrame $bmp
      if ($null -eq $framed) {
        $dbg = Join-Path $env:TEMP ("maguro-edge-" + [IO.Path]::GetFileName($path))
        try { $bmp.Save($dbg, [Drawing.Imaging.ImageFormat]::Png) } catch {}
        $edge = Test-MaguroEdges $bmp
        $bmp.Dispose(); throw ("dark_edges L={0} attempt={1}" -f $edge.LeftBlack,$attempt)
      }
      $bmp = $framed
      $outW = $bmp.Width; $outH = $bmp.Height
      $dir = Split-Path $path -Parent
      if ($dir) { New-Item -ItemType Directory -Force $dir | Out-Null }
      $bmp.Save($path, [Drawing.Imaging.ImageFormat]::Png)
      $bmp.Dispose()
      Write-Host ("OK {0} {1}x{2} {3}KB" -f $path,$outW,$outH,[int]((Get-Item $path).Length/1KB))
      return
    } catch {
      $lastErr = $_; Write-Host "RETRY $path : $_"; Close-MaguroBackdrop; Start-Sleep -Milliseconds 400
    }
  }
  throw "Cap-MaguroWindow FAILED $path : $lastErr"
}

function Click-MaguroSettingsPage([IntPtr]$h, [int]$page) {
  $dpi = [MaguroCap]::Dpi($h)
  $x = [int](8*$dpi/96) + [int]((220*$dpi/96)/2)
  $y = [int]((12+32)*$dpi/96) + [int](8*$dpi/96) + $page*[int](36*$dpi/96) + [int]((36*$dpi/96)/2)
  [MaguroCap]::ClickClient($h, $x, $y)
}

# Win32 popup menu (#32768) fallback: UIA often sees no menu from a background pwsh.
# Finds the item by text via MN_GETHMENU and clicks its rect. Only touches a visible popup menu.
if (-not ('MaguroMenuW' -as [type])) {
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class MaguroMenuW {
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr p, IntPtr a, string c, string t);
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
 [DllImport("user32.dll")] public static extern int GetMenuItemCount(IntPtr m);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetMenuString(IntPtr m, uint id, StringBuilder s, int n, uint f);
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }
 [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr h, IntPtr m, uint i, out RECT r);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint f,int x,int y,uint d,UIntPtr e);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
"@
}
function Invoke-MaguroMenuNameWin32([string]$pattern) {
  $w = [IntPtr]::Zero
  while (($w = [MaguroMenuW]::FindWindowEx([IntPtr]::Zero, $w, '#32768', $null)) -ne [IntPtr]::Zero) {
    if (-not [MaguroMenuW]::IsWindowVisible($w)) { continue }
    $hm = [MaguroMenuW]::SendMessage($w, 0x1E1, [IntPtr]::Zero, [IntPtr]::Zero)  # MN_GETHMENU
    if ($hm -eq [IntPtr]::Zero) { continue }
    $n = [MaguroMenuW]::GetMenuItemCount($hm)
    for ($i = 0; $i -lt $n; $i++) {
      $sb = New-Object System.Text.StringBuilder 256
      [void][MaguroMenuW]::GetMenuString($hm, [uint32]$i, $sb, 256, 0x400)
      $s = $sb.ToString() -replace '&', ''
      if ($s -and $s -match $pattern) {
        $r = New-Object MaguroMenuW+RECT
        [void][MaguroMenuW]::GetMenuItemRect([IntPtr]::Zero, $hm, [uint32]$i, [ref]$r)
        [void][MaguroMenuW]::SetCursorPos([int](($r.L + $r.R) / 2), [int](($r.T + $r.B) / 2))
        Start-Sleep -Milliseconds 150
        [MaguroMenuW]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); [MaguroMenuW]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
        return $true
      }
    }
  }
  return $false
}
function Invoke-MaguroMenuName([string]$pattern) {
  if (Invoke-MaguroMenuNameUia $pattern) { return $true }
  return (Invoke-MaguroMenuNameWin32 $pattern)
}

# Optional full-screen debug frame (set MAGURO_DEBUG_DIR). Off by default.
function Save-MaguroDebugShot([string]$name) {
  if (-not $env:MAGURO_DEBUG_DIR) { return }
  Add-Type -AssemblyName System.Drawing | Out-Null
  $b = New-Object System.Drawing.Bitmap 1600, 1000
  $g = [System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen(0, 0, 0, 0, $b.Size); $g.Dispose()
  $b.Save((Join-Path $env:MAGURO_DEBUG_DIR "$name.png")); $b.Dispose()
}
