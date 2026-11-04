# Temporary white window covering [L,T,R,B] so CopyFromScreen has no desktop chrome bleed.
# Must not activate: Quick Look closes itself on WA_INACTIVE.
param([int]$L,[int]$T,[int]$R,[int]$B,[string]$Marker)
Add-Type -AssemblyName System.Windows.Forms
if (-not ('MaguroWhiteBd' -as [type])) {
Add-Type -TypeDefinition @"
using System;
using System.Windows.Forms;
public class MaguroWhiteBd : Form {
  protected override bool ShowWithoutActivation { get { return true; } }
  protected override CreateParams CreateParams {
    get {
      CreateParams cp = base.CreateParams;
      cp.ExStyle |= 0x08000000 | 0x00000080; // WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW
      return cp;
    }
  }
}
"@
}
$form = New-Object MaguroWhiteBd
$form.FormBorderStyle = 'None'
$form.BackColor = [Drawing.Color]::White
$form.TopMost = $false
$form.ShowInTaskbar = $false
$form.StartPosition = 'Manual'
$form.Bounds = New-Object Drawing.Rectangle $L,$T,($R-$L),($B-$T)
if ($Marker) { Set-Content -Path ($Marker + '.pspid') -Value $PID -Encoding ascii }
$form.Show(); [Windows.Forms.Application]::DoEvents()
while ($true) {
  if ($Marker -and -not (Test-Path $Marker) -and -not (Test-Path ($Marker + '.keep'))) { break }
  Start-Sleep -Milliseconds 200
  [Windows.Forms.Application]::DoEvents()
}
$form.Close()
