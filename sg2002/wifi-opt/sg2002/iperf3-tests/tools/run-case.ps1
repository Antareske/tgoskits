param(
  [string]$Os = "linux",
  [string]$Mode = "sta",
  [string]$Role = "board-s",          # board-s (board server, PC client) | board-c (board client, PC server)
  [string]$Proto = "tcp",             # tcp | udp
  [string]$Dir = "tx",                # tx | rx | bidir   (relative to the BOARD)
  [int]$Streams = 1,
  [int]$Duration = 30,
  [int]$Omit = 3,
  [string]$BoardIp = "192.168.137.68",
  [string]$PcIp = "192.168.137.1",
  [string]$BoardPort = "COM5",
  [string]$UdpBandwidth = "0",
  [string]$Root = "D:\1\Documents\iperf3-tests",
  [string]$Iperf3 = "D:\1\Documents\iperf3-tests\tools\iperf3-win\iperf3.17.1_64\iperf3.exe",
  [string]$SerialPs1 = "D:\1\Documents\iperf3-tests\tools\serial.ps1",
  [switch]$SkipPing,
  [switch]$NoServerOutput,
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
$srvRemoteLog = "/tmp/iperf3_srv_$case.log"

$boardClientLog = Join-Path $caseDir "$case.board_client.log"
$pcClientLog    = Join-Path $caseDir "$case.pc_client.log"
$boardServerLog = Join-Path $caseDir "$case.board_server.log"
$pcServerLog    = Join-Path $caseDir "$case.pc_server.log"

function Invoke-Board([string]$cmd, [int]$waitMs, [int]$quietMs = 2500, [string]$tag = "") {
  & $SerialPs1 -Command $cmd -WaitMs $waitMs -QuietMs $quietMs -Port $BoardPort -LogFile $boardSerialLog -Tag $tag
}

function Write-Row([string]$status, [string]$note, [string]$result) {
  $row = [pscustomobject]@{
    case      = $case
    os        = $Os
    mode      = $Mode
    role      = $Role
    proto     = $Proto
    dir       = $Dir
    streams   = $Streams
    duration  = $Duration
    udp_bw    = $UdpBandwidth
    status    = $status
    note      = $note
    result    = $result
    timestamp = (Get-Date).ToString("s")
  }
  if (-not (Test-Path $csv)) {
    "case,os,mode,role,proto,dir,streams,duration,udp_bw,status,note,result,timestamp" | Set-Content -LiteralPath $csv -Encoding UTF8
  }
  $row | ConvertTo-Csv -NoTypeInformation | Select-Object -Skip 1 | Add-Content -LiteralPath $csv -Encoding UTF8
}

# ---- build flags -----------------------------------------------------------
$common = @("-t", "$Duration", "-O", "$Omit", "-i", "1", "-P", "$Streams")
if (-not $NoServerOutput) { $common += @("--get-server-output") }
if ($Proto -eq "udp") { $common += @("-u", "-b", "$UdpBandwidth") }

function Dir-Flags([string]$client) {
  if ($client -eq "pc") {
    switch ($Dir) {
      "rx"    { return @() }          # PC sends -> board receives
      "tx"    { return @("-R") }      # board sends -> PC receives
      "bidir" { return @("--bidir") }
    }
  } else {
    switch ($Dir) {
      "tx"    { return @() }          # board sends -> PC receives
      "rx"    { return @("-R") }      # PC sends -> board receives
      "bidir" { return @("--bidir") }
    }
  }
}

$clientIsPc = ($Role -eq "board-s")
$clientArgs = $common + (Dir-Flags $(if ($clientIsPc) { "pc" } else { "board" }))

$header = "CASE=$case  boardDir=$Dir  role=$Role  proto=$Proto  P=$Streams  dur=$Duration  omit=$Omit  udpBw=$UdpBandwidth"
Write-Output "==================== $header"
if ($DryRun) { Write-Output ("client-args : " + ($clientArgs -join ' ')); exit 0 }

# ---- pre-check -------------------------------------------------------------
if (-not $SkipPing) {
  $pingOk = $false
  try { $pingOk = Test-Connection -ComputerName $BoardIp -Count 2 -Quiet } catch { $pingOk = $false }
  if (-not $pingOk) {
    Write-Output "PRECHECK FAILED: board $BoardIp not reachable"
    Write-Row "FAIL" "precheck: board unreachable" ""
    exit 2
  }
}

$clientSummary = ""
$status = "OK"
$note = ""

if ($Role -eq "board-s") {
  # ---- board is server, PC is client --------------------------------------
  $startCmd = "killall iperf3 2>/dev/null; sleep 1; rm -f $srvRemoteLog; iperf3 -s -1 --logfile $srvRemoteLog >/dev/null 2>&1 & sleep 2; echo -n SRVPID=; pidof iperf3; echo SRV_READY"
  Invoke-Board $startCmd 10000 2500 "start board server $case" | Out-Null

  $pcArgs = @("-c", $BoardIp) + $clientArgs
  Write-Output ("PC client: iperf3 " + ($pcArgs -join ' '))
  $pcOut = & $Iperf3 @pcArgs 2>&1 | Tee-Object -FilePath $pcClientLog
  $clientSummary = (($pcOut -split "`n") | Select-String -Pattern "sender|receiver" | Where-Object { $_.Line -notmatch "omitted" } | ForEach-Object { $_.Line.Trim() }) -join " | "

  for ($i = 0; $i -lt 12; $i++) {
    $p = Invoke-Board 'echo -n P=; pidof iperf3; echo .' 4000 1200 "poll board server $case"
    if ($p -notmatch "P=\d") { break }
    Start-Sleep -Milliseconds 500
  }
  Invoke-Board "cat $srvRemoteLog; echo SRVLOG_DONE" 8000 2500 "cat board server log $case" | Set-Content -LiteralPath $boardServerLog -Encoding UTF8
}
else {
  # ---- board is client, PC is server --------------------------------------
  if (Test-Path $pcServerLog) { Remove-Item -LiteralPath $pcServerLog -Force }
  $srvArgs = @("-s", "-1", "--logfile", $pcServerLog)
  $srv = Start-Process -FilePath $Iperf3 -ArgumentList $srvArgs -PassThru -WindowStyle Hidden
  Start-Sleep -Seconds 2

  # run board client in BACKGROUND (never blocks the console) with hard timeout
  $argStr = ($clientArgs -join ' ')
  $startCmd = "killall iperf3 2>/dev/null; sleep 1; rm -f /tmp/iperf3_cli.log; iperf3 -c $PcIp $argStr > /tmp/iperf3_cli.log 2>&1 & sleep 2; echo -n CPID=; pidof iperf3; echo CLI_STARTED"
  Invoke-Board $startCmd 10000 2500 "start board client $case" | Out-Null

  $maxWait = $Duration + 30
  $elapsed = 0
  $stillRunning = $false
  while ($elapsed -lt $maxWait) {
    Start-Sleep -Seconds 5
    $elapsed += 5
    $p = Invoke-Board 'echo -n P=; pidof iperf3; echo .' 4000 1200 "poll board client $case"
    if ($p -notmatch "P=\d") { break }
  }
  if ($elapsed -ge $maxWait) { $stillRunning = $true }

  if ($stillRunning) { Invoke-Board 'killall iperf3; echo KILLED' 6000 2500 "kill hung board client $case" | Out-Null }

  $boardOut = Invoke-Board "cat /tmp/iperf3_cli.log; echo CLILOG_DONE" 12000 3000 "cat board client log $case"
  $boardOut | Set-Content -LiteralPath $boardClientLog -Encoding UTF8
  $clientSummary = (($boardOut -split "`n") | Select-String -Pattern "sender|receiver" | Where-Object { $_.Line -notmatch "omitted" } | ForEach-Object { $_.Line.Trim() }) -join " | "
  if ($stillRunning) { $status = "FAIL"; $note = "board client hung (killed after ${maxWait}s)" }

  try { if (-not $srv.HasExited) { $srv.WaitForExit(15000) | Out-Null } } catch {}
  if (-not $srv.HasExited) { try { $srv.Kill() } catch {} }
}

if ([string]::IsNullOrWhiteSpace($clientSummary)) { $status = "FAIL"; if (-not $note) { $note = "no summary parsed" } }
elseif ($clientSummary -match "(?i)error|unable to connect|Connection reset|timed out") { $status = "FAIL"; $note = "client reported error" }

Write-Row $status $note $clientSummary
Write-Output "STATUS=$status  RESULT: $clientSummary"
