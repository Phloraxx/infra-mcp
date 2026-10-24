param(
  [string]$Gateway = "",
  [string]$Code = "",
  [string]$Name = "",
  [string]$Version = "v0.1.0-rc.1", # Published test tag; GitHub latest excludes prereleases.
  [string]$InstallDir = "$env:LOCALAPPDATA\Infra\bin",
  [string]$StateDir = "$env:USERPROFILE\.infra-mcp-v1",
  [string]$SourceDir = "",
  [switch]$NoStartup
)

$ErrorActionPreference = "Stop"
$Repo = "Phloraxx/infra-mcp"

# Avoid invoking .ToString() on a potentially missing environment/API value.
$DetectedArchitecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($null -eq $DetectedArchitecture) {
  throw "Infra preflight: Windows OS architecture was unavailable. Run in Windows PowerShell 5.1 or PowerShell 7."
}
$Arch = switch ([string]$DetectedArchitecture) {
  "X64" { "x86_64" }
  "Arm64" { "aarch64" }
  default { throw "Infra preflight: unsupported Windows architecture '$DetectedArchitecture'." }
}

# Download and verify BOTH assets before modifying a working installation.
# This also means a missing GitHub release cannot leave a half-installed agent.
$TempDir = Join-Path ([IO.Path]::GetTempPath()) ("infra-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

# These helpers are deliberately strict: PowerShell 5.1 can return $null from
# file reads instead of an empty string. Never call methods on those results.
function Get-InfraExpectedHash([string]$ChecksumPath, [string]$Asset) {
  if (-not (Test-Path -LiteralPath $ChecksumPath -PathType Leaf)) {
    throw "Infra download: missing checksum file for $Asset. Check GitHub access or proxy filtering."
  }
  $Content = Get-Content -LiteralPath $ChecksumPath -Raw -ErrorAction Stop
  if ([string]::IsNullOrWhiteSpace($Content)) {
    throw "Infra download: checksum file for $Asset is empty. Check GitHub access or proxy filtering."
  }
  $Parts = $Content.Trim() -split '\s+'
  if ($Parts.Count -lt 1 -or [string]::IsNullOrWhiteSpace($Parts[0])) {
    throw "Infra download: checksum for $Asset is missing."
  }
  $Expected = [string]$Parts[0]
  if ($Expected -notmatch '^[0-9a-fA-F]{64}$') {
    throw "Infra download: checksum format is invalid for $Asset."
  }
  if ($Parts.Count -ge 2 -and $Parts[1] -ne $Asset -and $Parts[1] -ne "*$Asset") {
    throw "Infra download: checksum references another asset, refusing $Asset."
  }
  return $Expected.ToLowerInvariant()
}

function Get-InfraActualHash([string]$FilePath, [string]$Asset) {
  if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
    throw "Infra download: missing binary $Asset after download."
  }
  $FileInfo = Get-Item -LiteralPath $FilePath -ErrorAction Stop
  if ($null -eq $FileInfo -or $FileInfo.Length -le 0) {
    throw "Infra download: empty binary $Asset after download."
  }
  $Digest = Get-FileHash -LiteralPath $FilePath -Algorithm SHA256 -ErrorAction Stop
  if ($null -eq $Digest -or [string]::IsNullOrWhiteSpace($Digest.Hash)) {
    throw "Infra verification: could not calculate the SHA-256 of $Asset."
  }
  return ([string]$Digest.Hash).ToLowerInvariant()
}

function Stage-InfraBinary([string]$Name) {
  $Staged = Join-Path $TempDir "$Name.exe"
  if (-not [string]::IsNullOrWhiteSpace($SourceDir)) {
    $Source = Join-Path $SourceDir "$Name.exe"
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
      throw "Missing local development binary $Source. Build infra-agent and infra-cli first."
    }
    Copy-Item -LiteralPath $Source -Destination $Staged
  } else {
    $Asset = "$Name-windows-$Arch.exe"
    if ($Version -eq "latest") {
      $Base = "https://github.com/$Repo/releases/latest/download"
    } else {
      $Base = "https://github.com/$Repo/releases/download/$Version"
    }
    $ChecksumPath = "$Staged.sha256"
    try {
      Invoke-WebRequest -UseBasicParsing -Uri "$Base/$Asset" -OutFile $Staged -ErrorAction Stop
      Invoke-WebRequest -UseBasicParsing -Uri "$Base/$Asset.sha256" -OutFile $ChecksumPath -ErrorAction Stop
    } catch {
      $Status = $null
      if ($null -ne $_.Exception.Response) {
        $Status = [int]$_.Exception.Response.StatusCode
      }
      if ($Status -eq 404) {
        throw "Infra $Version has no published $Asset release asset. This repository may not have published a release yet; do not use an unverified third-party binary. For development, build locally and use -SourceDir."
      }
      throw "Unable to download $Asset from GitHub ($($_.Exception.Message)). Check network access, proxy/TLS and the release tag."
    }
    $Expected = Get-InfraExpectedHash -ChecksumPath $ChecksumPath -Asset $Asset
    $Actual = Get-InfraActualHash -FilePath $Staged -Asset $Asset
    if ($Expected -ne $Actual) { throw "SHA-256 verification failed for $Asset" }
    # SHA-256 is only an integrity check. First-install publisher authentication
    # needs a signed bootstrap trust root (tracked separately).
  }
  return $Staged
}

function Activate-InfraBinary([string]$Name, [string]$Staged) {
  $Destination = Join-Path $InstallDir "$Name.exe"
  # Repair/re-run of this same release must not try to overwrite a running
  # Windows executable; that raises access-denied and can damage a healthy setup.
  if (Test-Path -LiteralPath $Destination -PathType Leaf) {
    $InstalledHash = Get-InfraActualHash -FilePath $Destination -Asset "$Name (installed)"
    $StagedHash = Get-InfraActualHash -FilePath $Staged -Asset "$Name (staged)"
    if ($InstalledHash -eq $StagedHash) {
      Write-Host "Infra $Name is already installed; leaving the existing binary intact."
      return
    }
  }
  $TempDestination = "$Destination.$([guid]::NewGuid().ToString('N')).tmp"
  try {
    Copy-Item -LiteralPath $Staged -Destination $TempDestination
    try { Move-Item -LiteralPath $TempDestination -Destination $Destination -Force }
    catch {
      throw "Cannot replace $Name while it is running or locked. Stop the Infra Agent task for a version upgrade, then retry. ($($_.Exception.Message))"
    }
  } finally {
    Remove-Item -LiteralPath $TempDestination -Force -ErrorAction SilentlyContinue
  }
}

try {
  Write-Host "Infra · downloading native binaries"
  # Reuse existing config on repair instead of replacing an enrolled endpoint.
  if ([string]::IsNullOrWhiteSpace($Gateway) -and -not (Test-Path -LiteralPath (Join-Path $StateDir "config.json"))) {
    $Gateway = "wss://infra.drbijoy.in/agent"
  }
  # A repair reuses running binaries instead of trying to replace locked Windows EXEs.
  $ExistingCli = Join-Path $InstallDir "infra.exe"
  $ExistingAgent = Join-Path $InstallDir "infra-agent.exe"
  if ([string]::IsNullOrWhiteSpace($Code) -and [string]::IsNullOrWhiteSpace($SourceDir) -and
      (Test-Path -LiteralPath $ExistingCli) -and (Test-Path -LiteralPath $ExistingAgent) -and
      ((& $ExistingCli help 2>&1 | Out-String) -match 'infra setup')) {
    Write-Host "Infra is already installed. Checking pairing and startup..."
    # Idempotency: do not restart a correctly configured, healthy agent.
    # On Windows /Run on an already-running scheduled task may do nothing;
    # waiting for a newer generation would then fail after 35 seconds.
    $CurrentStatus = $null
    try {
      $StatusJson = & $ExistingCli status --json --state-dir $StateDir 2>$null
      if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace(($StatusJson | Out-String))) {
        $CurrentStatus = $StatusJson | Out-String | ConvertFrom-Json -ErrorAction Stop
      }
    } catch {
      Write-Host "Infra status unavailable; attempting native startup repair."
    }
    $GatewayMatches = [string]::IsNullOrWhiteSpace($Gateway) -or
      ($null -ne $CurrentStatus -and $null -ne $CurrentStatus.config -and
       $CurrentStatus.config.gateway_url -eq $Gateway)
    if ($null -ne $CurrentStatus -and $CurrentStatus.configured -and $CurrentStatus.enrolled -and
        $CurrentStatus.process_alive -and $CurrentStatus.startup.installed -and
        $CurrentStatus.runtime.state -eq "online" -and $GatewayMatches) {
      Write-Host "Infra is already paired, running and connected; nothing to repair."
      return
    }
    $RepairArgs = @("setup", "--agent", $ExistingAgent, "--state-dir", $StateDir)
    if (-not [string]::IsNullOrWhiteSpace($Gateway)) { $RepairArgs += @("--gateway", $Gateway) }
    if ($NoStartup) { $RepairArgs += "--no-startup" }
    & $ExistingCli @RepairArgs
    if ($LASTEXITCODE -ne 0) { throw "Existing Infra setup needs repair. Run $ExistingCli doctor." }
    return
  }
  $CliStage = Stage-InfraBinary "infra"
  $AgentStage = Stage-InfraBinary "infra-agent"
  New-Item -ItemType Directory -Force -Path $InstallDir, $StateDir | Out-Null
  # A re-run must not destroy a previous working installation.
  # Stage and keep rollback copies of BOTH binaries before invoking setup.
  $BackupCli = Join-Path $TempDir "previous-infra.exe"
  $BackupAgent = Join-Path $TempDir "previous-infra-agent.exe"
  $CliPath = Join-Path $InstallDir "infra.exe"
  $AgentPath = Join-Path $InstallDir "infra-agent.exe"
  $HadCli = Test-Path -LiteralPath $CliPath
  $HadAgent = Test-Path -LiteralPath $AgentPath
  if ($HadCli) { Copy-Item -LiteralPath $CliPath -Destination $BackupCli }
  if ($HadAgent) { Copy-Item -LiteralPath $AgentPath -Destination $BackupAgent }
  try {
    Activate-InfraBinary "infra-agent" $AgentStage
    Activate-InfraBinary "infra" $CliStage

  $Cli = Join-Path $InstallDir "infra.exe"
  $Agent = Join-Path $InstallDir "infra-agent.exe"
  # No enrollment code means device-initiated, owner-approved pairing.
  # Older devices can still provide -Code explicitly for legacy enrollment.
  $CommandName = if ([string]::IsNullOrWhiteSpace($Code)) { "setup" } else { "install" }
  $Arguments = @($CommandName, "--agent", $Agent, "--state-dir", $StateDir)
  if (-not [string]::IsNullOrWhiteSpace($Gateway)) { $Arguments += @("--gateway", $Gateway) }
  if (-not [string]::IsNullOrWhiteSpace($Code)) { $Arguments += @("--code", $Code) }
  if (-not [string]::IsNullOrWhiteSpace($Name) -and $CommandName -eq "install") { $Arguments += @("--name", $Name) }
  if ($NoStartup) { $Arguments += "--no-startup" }

  & $Cli @Arguments
  if ($LASTEXITCODE -ne 0) { throw "Infra $CommandName did not complete (exit code $LASTEXITCODE). Your existing identity was preserved. Run Infra setup again or Infra doctor to diagnose." }

  Write-Host ""
  Write-Host "Installed:"
  Write-Host "  CLI:   $Cli"
  Write-Host "  Agent: $Agent"
  Write-Host "  State: $StateDir"
  Write-Host "Run: $Cli doctor"
  } catch {
    # Windows will not replace a running/locked executable. Fail with the
    # existing installation intact whenever the OS allows restoration.
    foreach ($item in @(@($CliPath,$BackupCli,$HadCli),@($AgentPath,$BackupAgent,$HadAgent))) {
      $dest = [string]$item[0]; $backup = [string]$item[1]; $existed = [bool]$item[2]
      if (Test-Path -LiteralPath $backup) {
        Copy-Item -LiteralPath $backup -Destination $dest -Force -ErrorAction Stop
      }
      # If this was a first install, keep the verified native binaries.
      # Pairing may merely have timed out: retry should not download again.
    }
    throw "Infra setup did not finish; previous versions were preserved and new-device pairing can be resumed. $($_.Exception.Message)"
  }
}
catch {
  $Line = $_.InvocationInfo.ScriptLineNumber
  throw "Infra installation failed at installer line ${Line}: $($_.Exception.Message)"
}
finally {
  Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}
