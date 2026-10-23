# Run in an isolated Windows CI VM. No Scheduled Task, real user profile, or
# live agent is changed. Tests Windows PowerShell 5.1 compatibility.
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$TempRoot = Join-Path ([IO.Path]::GetTempPath()) ("infra-win-smoke-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $TempRoot -Force | Out-Null
$Install = Join-Path $TempRoot "Installed Native Binaries"
$State = Join-Path $TempRoot "Agent Profile With Spaces"
$GatewayProcess = $null
$AgentProcess = $null

try {
  # Failure must not mutate an installation. Missing agent in a source folder
  # must be detected before the CLI executable is installed.
  $Incomplete = Join-Path $TempRoot "Incomplete Binaries"
  New-Item -ItemType Directory -Path $Incomplete | Out-Null
  Copy-Item -LiteralPath (Join-Path $Root "target\release\infra.exe") -Destination (Join-Path $Incomplete "infra.exe")
  $FailedCorrectly = $false
  try {
    & (Join-Path $Root "scripts\install.ps1") -Gateway "ws://127.0.0.1:9/agent" -Code "not-a-real-code" -InstallDir $Install -StateDir $State -SourceDir $Incomplete -NoStartup
  } catch {
    $FailedCorrectly = $_.Exception.Message -match "Missing local development binary"
  }
  if (-not $FailedCorrectly) { throw "Incomplete-binary preflight did not fail" }
  if ((Test-Path -LiteralPath $Install) -or (Test-Path -LiteralPath $State)) {
    throw "A failed preflight mutated installation/state"
  }

  # An ephemeral loopback Rust gateway, unique enrollment and temp state.
  $Tcp = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
  $Tcp.Start()
  $Port = $Tcp.LocalEndpoint.Port
  $Tcp.Stop()

  $Admin = ([guid]::NewGuid().ToString("N") + [guid]::NewGuid().ToString("N"))
  $env:INFRA_GATEWAY_BIND = "127.0.0.1:$Port"
  $env:INFRA_DB_PATH = Join-Path $TempRoot "gateway.db"
  $env:INFRA_ADMIN_TOKEN = $Admin
  $GatewayProcess = Start-Process -FilePath (Join-Path $Root "target\debug\infra-gateway.exe") -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $TempRoot "gateway.out") -RedirectStandardError (Join-Path $TempRoot "gateway.err")
  $Base = "http://127.0.0.1:$Port"
  $Started = $false
  for ($i = 0; $i -lt 80; $i++) {
    Start-Sleep -Milliseconds 150
    if ($GatewayProcess.HasExited) { throw "Test gateway exited before becoming healthy" }
    try {
      $Check = Invoke-WebRequest -UseBasicParsing -Uri "$Base/healthz" -TimeoutSec 2
      if ($Check.StatusCode -eq 200) { $Started = $true; break }
    } catch {}
  }
  if (-not $Started) { throw "Test gateway did not start" }

  $Headers = @{ Authorization = "Bearer $Admin"; "X-Infra-CSRF" = "1" }
  $Enroll = Invoke-RestMethod -Method Post -Uri "$Base/api/enrollments" -Headers $Headers -ContentType "application/json" -Body '{"ttl_seconds":120}'
  if ([string]::IsNullOrWhiteSpace($Enroll.code)) { throw "Enrollment code missing" }

  & (Join-Path $Root "scripts\install.ps1") -Gateway "ws://127.0.0.1:$Port/agent" -Code $Enroll.code -Name "Windows Install Smoke" -InstallDir $Install -StateDir $State -SourceDir (Join-Path $Root "target\release") -NoStartup
  if (-not (Test-Path -LiteralPath (Join-Path $Install "infra-agent.exe"))) { throw "Agent missing" }
  if (-not (Test-Path -LiteralPath (Join-Path $State "identity.json"))) { throw "Enrollment identity missing" }
  $StatusText = & (Join-Path $Install "infra.exe") status --json --state-dir $State
  if ($LASTEXITCODE -ne 0) { throw "Native Windows CLI status failed" }
  $Status = $StatusText | ConvertFrom-Json
  if (-not $Status.configured -or -not $Status.enrolled) { throw "Native Windows enrollment/config failed" }

  $AgentProcess = Start-Process -FilePath (Join-Path $Install "infra-agent.exe") -ArgumentList @("--state-dir", ('"' + $State + '"')) -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $TempRoot "agent.out") -RedirectStandardError (Join-Path $TempRoot "agent.err")
  $Online = $false
  for ($i = 0; $i -lt 70; $i++) {
    Start-Sleep -Milliseconds 150
    if ($AgentProcess.HasExited) { throw "Windows native agent exited unexpectedly" }
    $Devices = Invoke-RestMethod -Uri "$Base/api/devices" -Headers $Headers
    if (@($Devices | Where-Object { $_.device_name -eq "Windows Install Smoke" -and $_.online }).Count -eq 1) {
      $Online = $true
      break
    }
  }
  if (-not $Online) { throw "Windows signed agent never came online" }
  Write-Host "Windows bootstrap smoke PASS: both-binary preflight, spaces in paths, source install, signed enrollment, CLI status and live agent handshake"
}
finally {
  if ($null -ne $AgentProcess -and -not $AgentProcess.HasExited) { Stop-Process -Id $AgentProcess.Id -Force -ErrorAction SilentlyContinue }
  if ($null -ne $GatewayProcess -and -not $GatewayProcess.HasExited) { Stop-Process -Id $GatewayProcess.Id -Force -ErrorAction SilentlyContinue }
  Remove-Item -LiteralPath $TempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
