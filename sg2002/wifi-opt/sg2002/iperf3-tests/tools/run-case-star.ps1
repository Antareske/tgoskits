param(
  [string]$Os = "starryos",
  [string]$Mode = "sta",
  [string]$Role = "board-s",          # board-s | board-c
  [string]$Proto = "tcp",
  [string]$Dir = "tx",                # tx | rx | bidir (relative to BOARD)
  [int]$Streams = 1,
  [int]$Duration = 30,
  [int]$Omit = 3,
  [string]$BoardIp = "192.168.137.17",
  [string]$PcIp = "192.168.137.1",
  [string]$BoardPort = "COM5",
  [string]$UdpBandwidth = "0",
  [string]$Root = "D:\1\Documents\iperf3-tests",
  [string]$Iperf3 = "D:\1\Documents\iperf3-tests\tools\iperf3-win\iperf3.17.1_64\iperf3.exe",
  [string]$SerialPs1 = "D:\1\Documents\iperf3-tests\tools\serial.ps1",
  [switch]$DryRun
)

$ErrorActionPreference = "Continue"
Set-ExecutionPolicy -Scope Process Bypass -Force | Out-Null

$case = "$Os-$Mode-$Role-$Proto-$Dir-P$Streams"
$dirRoot = Join-Path $Root "$Os\$Mode"
$caseDir = Join-Path $dirRoot "cases"
New-Item -ItemType Directory -Force -Path $caseDir | Out-Null
$boardSerialLog = Join-Path $dirRoot "board_serial.log"
$csv = Join-Path $dirRoot "cases.csv"
$boardClientLog = Join-Path $caseDir "$case.board_client.log"
$pcClientLog    = Join-Path $caseDir "$case.pc_client.log"
$boardServerLog = Join-Path $caseDir "$case.board_server.log"
$pcServerLog    = Join-Path $caseDir "$case.pc_server.log"

function Invoke-Board([string]$cmd, [int]$waitMs, [int]$quietMs = 3000, [string]$tag = "") {
  & $SerialPs1 -Command $cmd -WaitMs $waitMs -QuietMs $quietMs -Port $BoardPort -LogFile $boardSerialLog -Tag $tag
}
function Write-Row([string]$status, [string]$note, [string]$result) {
  $row = [pscustomobject]@{
    case=$case; os=$Os; mode=$Mode; role=$Role; proto=$Proto; dir=$Dir; streams=$Streams
    duration=$Duration; udp_bw=$UdpBandwidth; status=$status; note=$note; result=$result
    timestamp=(Get-Date).ToString("s")
  }
  if (-not (Test-Path $csv)) {
    "case,os,mode,role,proto,dir,streams,duration,udp_bw,status,note,result,timestamp" | Set-Content -LiteralPath $csv -Encoding UTF8
  }
  $row | ConvertTo-Csv -NoTypeInformation | Select-Object -Skip 1 | Add-Content -LiteralPath $csv -Encoding UTF8
}
function Summarize([string]$text) {
  return (($text -split "`n") | Select-String -Pattern "sender|receiver" | Where-Object { $_.Line -notmatch "omitted" } | ForEach-Object { $_.Line.Trim() }) -join " | "
}

$common = @("-t","$Duration","-O","$Omit","-i","1","-P","$Streams","--get-server-output")
if ($Proto -eq "udp") { $common += @("-u","-b","$UdpBandwidth") }

$clientIsPc = ($Role -eq "board-s")
if ($clientIsPc) {
  switch ($Dir) { "rx" {$df=@()} "tx" {$df=@("-R")} "bidir" {$df=@("--bidir")} }
} else {
  switch ($Dir) { "tx" {$df=@()} "rx" {$df=@("-R")} "bidir" {$df=@("--bidir")} }
}
$clientArgs = $common + $df

Write-Output "==================== CASE=$case  boardDir=$Dir  role=$Role  proto=$Proto  P=$Streams  dur=$Duration  omit=$Omit"
if ($DryRun) { Write-Output ("client-args: " + ($clientArgs -join ' ')); exit 0 }

$status = "OK"; $note = ""; $summary = ""

if ($Role -eq "board-s") {
  # 1 board command: start one-off daemon server
  Invoke-Board "killall iperf3 2>/dev/null; sleep 2; rm -f /tmp/iperf3_s.log; iperf3 -s -1 -D --logfile /tmp/iperf3_s.log; sleep 2; echo SRV_LAUNCHED" 14000 4500 "start starry server $case" | Out-Null
  # PC client
  $pcArgs = @("-c", $BoardIp) + $clientArgs
  Write-Output ("PC client: iperf3 " + ($pcArgs -join ' '))
  $pcOut = & $Iperf3 @pcArgs 2>&1 | Tee-Object -FilePath $pcClientLog
  $summary = Summarize ($pcOut -join "`n")
  # 1 board command: cat server log
  $srvOut = Invoke-Board "cat /tmp/iperf3_s.log; echo SRVLOG_DONE" 10000 4000 "cat starry server log $case"
  $srvOut | Set-Content -LiteralPath $boardServerLog -Encoding UTF8
}
else {
  if (Test-Path $pcServerLog) { Remove-Item -LiteralPath $pcServerLog -Force }
  $srv = Start-Process -FilePath $Iperf3 -ArgumentList @("-s","-1","--logfile",$pcServerLog) -PassThru -WindowStyle Hidden
  Start-Sleep -Seconds 2
  # 1 board command: foreground client (blocks until test done)
  $boardCmd = "iperf3 -c $PcIp " + ($clientArgs -join ' ')
  $boardOut = Invoke-Board $boardCmd ($Duration*1000 + 30000) 6000 "starry client $case"
  $boardOut | Set-Content -LiteralPath $boardClientLog -Encoding UTF8
  $summary = Summarize $boardOut
  try { if (-not $srv.HasExited) { $srv.WaitForExit(20000) | Out-Null } } catch {}
  if (-not $srv.HasExited) { try { $srv.Kill() } catch {} }
}

if ([string]::IsNullOrWhiteSpace($summary)) { $status = "FAIL"; if (-not $note) { $note = "no summary parsed" } }
elseif ($summary -match "(?i)error|unable to connect|Connection reset|timed out") { $status = "FAIL"; $note = "client reported error" }

Write-Row $status $note $summary
Write-Output "STATUS=$status  RESULT: $summary"
