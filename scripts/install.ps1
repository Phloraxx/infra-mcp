param(
  [Parameter(Mandatory=$true)][string]$Gateway,
  [string]$Code = "",
  [string]$Name = "",
  [string]$Version = "latest",
  [string]$InstallDir = "$env:LOCALAPPDATA\Infra\bin",
  [string]$StateDir = "$env:USERPROFILE\.infra-mcp-v1",
  [string]$SourceDir = "",
  [switch]$NoStartup
)

$ErrorActionPreference = "Stop"
$Repo = "Phloraxx/infra-mcp"

$Arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
  "X64" { "x86_64" }
  "Arm64" { "aarch64" }
  default { throw "Unsupported Windows architecture: $_" }
}

# Download and verify BOTH assets before modifying a working installation.
# This also means a missing GitHub release cannot leave a half-installed agent.
$TempDir = Join-Path ([IO.Path]::GetTempPath()) ("infra-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

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
    $Parts = ((Get-Content -LiteralPath $ChecksumPath -Raw).Trim() -split '\s+')
    $Expected = $Parts[0].ToLowerInvariant()
    if ($Expected -notmatch '^[0-9a-f]{64}$') {
      throw "Release checksum format is invalid for $Asset"
    }
    if ($Parts.Length -ge 2 -and ($Parts[1] -ne $Asset -and $Parts[1] -ne "*$Asset")) {
      throw "Release checksum references a different asset, refusing $Asset"
    }
    $Actual = (Get-FileHash -LiteralPath $Staged -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Expected -ne $Actual) { throw "SHA-256 verification failed for $Asset" }
    # SHA-256 is only an integrity check. First-install publisher authentication
    # needs a signed bootstrap trust root (tracked separately).
  }
  return $Staged
}

function Activate-InfraBinary([string]$Name, [string]$Staged) {
  $Destination = Join-Path $InstallDir "$Name.exe"
  $TempDestination = "$Destination.$([guid]::NewGuid().ToString('N')).tmp"
  try {
    Copy-Item -LiteralPath $Staged -Destination $TempDestination
    Move-Item -LiteralPath $TempDestination -Destination $Destination -Force
  } finally {
    Remove-Item -LiteralPath $TempDestination -Force -ErrorAction SilentlyContinue
  }
}

try {
  Write-Host "Infra · preparing verified native binaries"
  $CliStage = Stage-InfraBinary "infra"
  $AgentStage = Stage-InfraBinary "infra-agent"
  New-Item -ItemType Directory -Force -Path $InstallDir, $StateDir | Out-Null
  Activate-InfraBinary "infra-agent" $AgentStage
  Activate-InfraBinary "infra" $CliStage

  $Cli = Join-Path $InstallDir "infra.exe"
  $Agent = Join-Path $InstallDir "infra-agent.exe"
  # No enrollment code means device-initiated, owner-approved pairing.
  # Older devices can still provide -Code explicitly for legacy enrollment.
  $CommandName = if ([string]::IsNullOrWhiteSpace($Code)) { "setup" } else { "install" }
  $Arguments = @($CommandName, "--gateway", $Gateway, "--agent", $Agent, "--state-dir", $StateDir)
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
}
finally {
  Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}
