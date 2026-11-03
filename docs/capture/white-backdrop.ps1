# Temporary white window covering [L,T,R,B] so CopyFromScreen has no desktop chrome bleed.
param([int]$L,[int]$T,[int]$R,[int]$B,[string]$Marker)
Add-Type -AssemblyName System.Windows.Forms
$form = New-Object Windows.Forms.Form
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
