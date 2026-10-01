param(
  [string]$Os = "linux",
  [string]$Mode = "sta",
  [string]$Role = "board-c",
  [string]$Dir = "tx",                # tx | rx | bidir   (relative to the BOARD)
  [int]$Streams = 4,                  # per-direction number of parallel single-stream processes
  [int]$Duration = 30,
  [int]$Omit = 3,
  [string]$BoardIp = "192.168.137.68",
  [string]$PcIp = "192.168.137.1",
  [string]$BoardPort = "COM5",
  [string]$UdpBandwidth = "0",
  [int]$BasePort = 5201,
  [string]$Root = "D:\1\Documents\iperf3-tests",
  [string]$Iperf3 = "D:\1\Documents\iperf3-tests\tools\iperf3-win\iperf3.17.1_64\iperf3.exe",
  [string]$SerialPs1 = "D:\1\Documents\iperf3-tests\tools\serial.ps1"
)

$ErrorActionPreference = "Continue"
Set-ExecutionPolicy -Scope Process Bypass -Force | Out-Null

$case = "$Os-$Mode-$Role-udp-$Dir-P$Streams-mproc"
$dirRoot = Join-Path $Root "$Os\$Mode"
$caseDir = Join-Path $dirRoot "cases"
New-Item -ItemType Directory -Force -Path $caseDir | Out-Null
$boardSerialLog = Join-Path $dirRoot "board_serial.log"
$csv = Join-Path $dirRoot "cases.csv"
$boardClientLog = Join-Path $caseDir "$case.board_client.log"

function Invoke-Board([string]$cmd, [int]$waitMs, [int]$quietMs = 2500, [string]$tag = "") {
  & $SerialPs1 -Command $cmd -WaitMs $waitMs -QuietMs $quietMs -Port $BoardPort -LogFile $boardSerialLog -Tag $tag
}
function Write-Row([string]$status, [string]$note, [string]$result) {
  $row = [pscustomobject]@{
    case=$case; os=$Os; mode=$Mode; role=$Role; proto="udp"; dir=$Dir; streams=$Streams
    duration=$Duration; udp_bw=$UdpBandwidth; status=$status; note=$note; result=$result
    timestamp=(Get-Date).ToString("s")
  }
  if (-not (Test-Path $csv)) {
    "case,os,mode,role,proto,dir,streams,duration,udp_bw,status,note,result,timestamp" | Set-Content -LiteralPath $csv -Encoding UTF8
  }
  $row | ConvertTo-Csv -NoTypeInformation | Select-Object -Skip 1 | Add-Content -LiteralPath $csv -Encoding UTF8
}

# build stream list: each entry = @{ port=; flags=; label= }
$streamsList = @()
$p = $BasePort
if ($Dir -eq "tx") {
  for ($i=0; $i -lt $Streams; $i++) { $streamsList += [pscustomobject]@{ port=$p; flags=""; label="tx" }; $p++ }
}
elseif ($Dir -eq "rx") {
  for ($i=0; $i -lt $Streams; $i++) { $streamsList += [pscustomobject]@{ port=$p; flags="-R"; label="rx" }; $p++ }
}
else { # bidir: Streams tx + Streams rx
  for ($i=0; $i -lt $Streams; $i++) { $streamsList += [pscustomobject]@{ port=$p; flags=""; label="tx" }; $p++ }
  for ($i=0; $i -lt $Streams; $i++) { $streamsList += [pscustomobject]@{ port=$p; flags="-R"; label="rx" }; $p++ }
}

Write-Output "==================== CASE=$case  boardDir=$Dir  streams=$Streams/dir  procs=$($streamsList.Count)  dur=$Duration  omit=$Omit  udpBw=$UdpBandwidth"

# ---- start PC servers ------------------------------------------------------
$srvs = @()
foreach ($st in $streamsList) {
  $slog = Join-Path $caseDir "$case.pc_server_$($st.port).log"
  if (Test-Path $slog) { Remove-Item -LiteralPath $slog -Force }
  $srvs += Start-Process -FilePath $Iperf3 -ArgumentList @("-s","-1","-p","$($st.port)","--logfile",$slog) -PassThru -WindowStyle Hidden
}
Start-Sleep -Seconds 2

# ---- launch board clients --------------------------------------------------
$cmd = "killall iperf3 2>/dev/null; sleep 1;"
foreach ($st in $streamsList) {
  $cmd += " rm -f /tmp/cli_$($st.port).log; iperf3 -c $PcIp -p $($st.port) -u -P 1 -b $UdpBandwidth -t $Duration -O $Omit -i 1 $($st.flags) > /tmp/cli_$($st.port).log 2>&1 &"
}
$cmd += " sleep 2; echo -n N=; pidof iperf3 | wc -w; echo LAUNCHED"
Invoke-Board $cmd 14000 3000 "start board clients $case" | Out-Null

$maxWait = $Duration + 30
$elapsed = 0
$stillRunning = $false
while ($elapsed -lt $maxWait) {
  Start-Sleep -Seconds 5; $elapsed += 5
  $poll = Invoke-Board 'echo -n P=; pidof iperf3 | wc -w; echo .' 4000 1200 "poll $case"
  if ($poll -match "P=0") { break }
}
if ($elapsed -ge $maxWait) { $stillRunning = $true }
if ($stillRunning) { Invoke-Board 'killall iperf3; echo KILLED' 6000 2500 "kill hung $case" | Out-Null }

# ---- collect logs ----------------------------------------------------------
$collect = ""
foreach ($st in $streamsList) { $collect += "echo ===PORT $($st.port) $($st.label)===; cat /tmp/cli_$($st.port).log; echo;" }
$collect += "echo ALLDONE"
$boardOut = Invoke-Board $collect 25000 3000 "cat board client logs $case"
$boardOut | Set-Content -LiteralPath $boardClientLog -Encoding UTF8

# ---- parse: sum final 'receiver' throughputs ------------------------------
$sumMbps = 0.0; $parts = @(); $err = $false
foreach ($line in ($boardOut -split "`n")) {
  if ($line -match "error") { $err = $true }
  if ($line -match "receiver" -and $line -notmatch "omitted" -and $line -match "([0-9]+\.?[0-9]*)\s+(K|M|G)bits/sec") {
    $v = [double]$Matches[1]
    switch ($Matches[2]) { "K" { $v = $v / 1000 } "G" { $v = $v * 1000 } }
    $sumMbps += $v
    $parts += $line.Trim()
  }
}
$summary = "sum_receiver=$([math]::Round($sumMbps,1)) Mbits/sec over $($streamsList.Count) procs; " + ($parts -join " ; ")
$status = "OK"; $note = "multi-process P1 x$($streamsList.Count) (Cygwin UDP -P server limitation)"
if ($err) { $status = "FAIL"; $note += "; client error present" }
if ($stillRunning) { $status = "FAIL"; $note += "; hung and killed" }

foreach ($s in $srvs) { try { if (-not $s.HasExited) { $s.WaitForExit(15000) | Out-Null } } catch {}; if (-not $s.HasExited) { try { $s.Kill() } catch {} } }

Write-Row $status $note $summary
Write-Output "STATUS=$status  RESULT: $summary"
