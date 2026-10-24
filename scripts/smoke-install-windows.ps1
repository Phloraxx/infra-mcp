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
$PairProcess = $null
$PairedAgentProcess = $null

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

  # Validate the real Windows scheduled startup, not only a manually launched
  # process: the user task must be registered, started and signed online.
  & (Join-Path $Install "infra.exe") repair --state-dir $State
  if ($LASTEXITCODE -ne 0) {
    Write-Host "Windows scheduled startup diagnostics:"
    $Task = Get-ScheduledTask -TaskName "Infra MCP Agent" -ErrorAction SilentlyContinue
    if ($null -ne $Task) {
      $Task.Actions | Select-Object Execute, Arguments, WorkingDirectory | Format-List | Out-String | Write-Host
      $Task.Principal | Select-Object UserId, LogonType, RunLevel | Format-List | Out-String | Write-Host
      Get-ScheduledTaskInfo -TaskName "Infra MCP Agent" | Select-Object LastTaskResult, LastRunTime, NumberOfMissedRuns | Format-List | Out-String | Write-Host
    } else { Write-Host "Task does not exist after repair." }
    $RuntimePath = Join-Path $State "runtime.json"
    if (Test-Path -LiteralPath $RuntimePath) {
      Get-Content -LiteralPath $RuntimePath -Raw | Write-Host
    } else { Write-Host "Agent runtime.json was not created." }
    throw "Windows Scheduled Task startup repair failed"
  }
  $Online = $false
  for ($i = 0; $i -lt 70; $i++) {
    Start-Sleep -Milliseconds 150
    if (-not (Get-ScheduledTask -TaskName "Infra MCP Agent" -ErrorAction SilentlyContinue)) { throw "Windows startup task disappeared" }
    $Devices = Invoke-RestMethod -Uri "$Base/api/devices" -Headers $Headers
    if (@($Devices | Where-Object { $_.device_name -eq "Windows Install Smoke" -and $_.online }).Count -eq 1) {
      $Online = $true
      break
    }
  }
  if (-not $Online) { throw "Windows signed agent never came online" }
  Write-Host "Windows bootstrap smoke PASS: verified preflight, spaces in paths, signed enrollment, Scheduled Task launch, CLI status and live agent handshake"

  # New happy path: no code or friendly name. Pair a second local test profile
  # through the same compiled native CLI and an authenticated owner action.
  $PairState = Join-Path $TempRoot "New Pair Profile With Spaces"
  $PairExe = Join-Path $Install "infra.exe"
  $PairAgent = Join-Path $Install "infra-agent.exe"
  $PairArguments = 'setup --gateway "ws://127.0.0.1:' + $Port +
    '/agent" --agent "' + $PairAgent + '" --state-dir "' + $PairState + '" --no-startup'
  $env:INFRA_NO_BROWSER = "1"
  $PairProcess = Start-Process -FilePath $PairExe -ArgumentList $PairArguments -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $TempRoot "pair.out") -RedirectStandardError (Join-Path $TempRoot "pair.err")
  $PendingPair = $null
  for ($i = 0; $i -lt 100; $i++) {
    Start-Sleep -Milliseconds 150
    if ($PairProcess.HasExited -and $null -ne $PairProcess.ExitCode -and $PairProcess.ExitCode -ne 0) {
      throw "Native Windows owner pairing exited early: $(Get-Content (Join-Path $TempRoot 'pair.err') -Raw)"
    }
    $Pending = @(Invoke-RestMethod -Uri "$Base/api/pair/requests" -Headers $Headers)
    if ($Pending.Count -gt 0) { $PendingPair = $Pending[0]; break }
  }
  if ($null -eq $PendingPair) { throw "Native Windows agent did not request owner pairing" }
  if ([string]::IsNullOrWhiteSpace($PendingPair.device_name)) { throw "Device did not detect its computer name" }
  if ($PendingPair.device_name -eq "Windows Install Smoke") { throw "Device reused the legacy manually supplied name" }
  if ($PendingPair.device_name -eq "unknown") { throw "Agent failed to detect the actual Windows device name" }
  if (-not [string]::IsNullOrWhiteSpace($env:COMPUTERNAME) -and $PendingPair.device_name -ne $env:COMPUTERNAME) {
    throw "Agent name does not match the Windows computer name"
  }
  $Code = $PendingPair.verification_code
  $RequestId = $PendingPair.request_id
  $Approval = Invoke-RestMethod -Uri "$Base/api/pair/$RequestId/approve" -Method Post -Headers $Headers -ContentType "application/json" -Body (ConvertTo-Json @{ verification_code = $Code } -Compress)
  if ($Approval.state -ne "approved" -or [string]::IsNullOrWhiteSpace($Approval.device_id)) { throw "Authenticated approval did not create a paired identity" }
  if (-not $PairProcess.WaitForExit(20000)) { throw "Native Windows pairing process did not complete after owner approval" }
  # Windows PowerShell 5.1 may expose a null ExitCode for a completed
  # redirected Start-Process handle. Prefer explicit proof of completion.
  $PairProcess.Refresh()
  if ($null -ne $PairProcess.ExitCode -and $PairProcess.ExitCode -ne 0) {
    Write-Host "Native setup exit code: $($PairProcess.ExitCode)"
    Write-Host "Native setup stdout:"
    Get-Content -LiteralPath (Join-Path $TempRoot "pair.out") -Raw | Write-Host
    Write-Host "Native setup stderr:"
    Get-Content -LiteralPath (Join-Path $TempRoot "pair.err") -Raw | Write-Host
    Write-Host "Identity present: $(Test-Path -LiteralPath (Join-Path $PairState 'identity.json'))"
    Write-Host "Config present: $(Test-Path -LiteralPath (Join-Path $PairState 'config.json'))"
    throw "Native Windows owner pairing command failed"
  }
  $IdentityPath = Join-Path $PairState "identity.json"
  if (-not (Test-Path -LiteralPath $IdentityPath)) { throw "Native Windows pairing identity missing" }
  if (-not (Select-String -Path (Join-Path $TempRoot "pair.out") -SimpleMatch "Infra paired and configured" -Quiet)) {
    throw "Native setup did not report successful owner pairing"
  }
  $OriginalHash = (Get-FileHash -LiteralPath $IdentityPath -Algorithm SHA256).Hash
  & $PairExe setup --gateway "ws://127.0.0.1:$Port/agent" --agent $PairAgent --state-dir $PairState --no-startup | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "Repeat Windows setup failed" }
  if ($OriginalHash -ne (Get-FileHash -LiteralPath $IdentityPath -Algorithm SHA256).Hash) {
    throw "Repeat Windows setup changed the signed device identity"
  }
  $PairedAgentProcess = Start-Process -FilePath $PairAgent -ArgumentList ('--state-dir "' + $PairState + '"') -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $TempRoot "paired-agent.out") -RedirectStandardError (Join-Path $TempRoot "paired-agent.err")
  $NewOnline = $false
  for ($i = 0; $i -lt 90; $i++) {
    Start-Sleep -Milliseconds 150
    if ($PairedAgentProcess.HasExited) {
      throw "Newly paired Windows agent exited: $(Get-Content (Join-Path $TempRoot 'paired-agent.err') -Raw)"
    }
    $Devices = @(Invoke-RestMethod -Uri "$Base/api/devices" -Headers $Headers)
    if (@($Devices | Where-Object { $_.device_id -eq $Approval.device_id -and $_.online -and $_.connection_generation }).Count -eq 1) {
      $NewOnline = $true
      break
    }
  }
  if (-not $NewOnline) { throw "New Windows device did not connect with its approved signed identity" }
  Write-Host "Windows new pairing PASS: OS hostname, owner approval, stable setup retry, signed agent connection"
}
finally {
  if (Test-Path -LiteralPath (Join-Path $Install "infra.exe")) {
    & (Join-Path $Install "infra.exe") uninstall --state-dir $State 2>$null | Out-Null
  }
  if ($null -ne $PairedAgentProcess -and -not $PairedAgentProcess.HasExited) { Stop-Process -Id $PairedAgentProcess.Id -Force -ErrorAction SilentlyContinue }
  if ($null -ne $PairProcess -and -not $PairProcess.HasExited) { Stop-Process -Id $PairProcess.Id -Force -ErrorAction SilentlyContinue }
  if ($null -ne $AgentProcess -and -not $AgentProcess.HasExited) { Stop-Process -Id $AgentProcess.Id -Force -ErrorAction SilentlyContinue }
  if ($null -ne $GatewayProcess -and -not $GatewayProcess.HasExited) { Stop-Process -Id $GatewayProcess.Id -Force -ErrorAction SilentlyContinue }
  Remove-Item -LiteralPath $TempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
