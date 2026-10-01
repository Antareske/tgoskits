param(
  [Parameter(Mandatory=$true)][string]$Command,
  [int]$WaitMs = 4000,
  [int]$QuietMs = 1500,
  [string]$Port = "COM5",
  [int]$Baud = 115200,
  [string]$LogFile = "",
  [string]$Tag = "",
  [switch]$Raw
)

$sp = [System.IO.Ports.SerialPort]::new($Port, $Baud, [System.IO.Ports.Parity]::None, 8, [System.IO.Ports.StopBits]::One)
$sp.ReadTimeout = 200
$sp.WriteTimeout = 2000
$sp.Handshake = [System.IO.Ports.Handshake]::None

try {
  $sp.Open()
  Start-Sleep -Milliseconds 150
  $sp.DiscardInBuffer()
  $sp.Write($Command + "`r")

  $sb = New-Object System.Text.StringBuilder
  $start = Get-Date
  $lastData = Get-Date
  while ($true) {
    $chunk = $sp.ReadExisting()
    if ($chunk.Length -gt 0) {
      [void]$sb.Append($chunk)
      $lastData = Get-Date
    } else {
      Start-Sleep -Milliseconds 50
    }
    $now = Get-Date
    if (($now - $start).TotalMilliseconds -ge $WaitMs) { break }
    if (($now - $lastData).TotalMilliseconds -ge $QuietMs -and $sb.Length -gt 0) { break }
  }
} finally {
  if ($sp.IsOpen) { $sp.Close() }
}

$rawOut = $sb.ToString()
$out = $rawOut
if (-not $Raw) {
  $out = $out -replace "`e\[[0-9;?]*[a-zA-Z]", ""
}

if ($LogFile -ne "") {
  $dir = Split-Path -Parent $LogFile
  if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
  $stamp = (Get-Date).ToString("yyyy-MM-dd HH:mm:ss.fff")
  $hdr = "`r`n===== [$stamp] $Tag :: $Command`r`n"
  Add-Content -LiteralPath $LogFile -Value $hdr -NoNewline -Encoding UTF8
  Add-Content -LiteralPath $LogFile -Value $out -Encoding UTF8
}

Write-Output $out
