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
      Invoke-WebRequest -UseBasicParsing -TimeoutSec 90 -Uri "$Base/$Asset" -OutFile $Staged -ErrorAction Stop
      Invoke-WebRequest -UseBasicParsing -TimeoutSec 90 -Uri "$Base/$Asset.sha256" -OutFile $ChecksumPath -ErrorAction Stop
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

try {
  if ([string]::IsNullOrWhiteSpace($Gateway) -and -not (Test-Path -LiteralPath (Join-Path $StateDir "config.json"))) {
    $Gateway = "wss://infra.drbijoy.in/agent"
  }
  $ExistingCli = Join-Path $InstallDir "infra.exe"
  $ExistingAgent = Join-Path $InstallDir "infra-agent.exe"
  $CanRepair = $false
  if (-not $PSBoundParameters.ContainsKey("Version") -and
      [string]::IsNullOrWhiteSpace($Code) -and [string]::IsNullOrWhiteSpace($SourceDir) -and
      (Test-Path -LiteralPath $ExistingCli) -and (Test-Path -LiteralPath $ExistingAgent)) {
    try {
      $Info = (& $ExistingCli --build-info 2>$null | Out-String | ConvertFrom-Json)
      $CanRepair = $LASTEXITCODE -eq 0 -and $Info.activation_protocol -eq 1
    } catch { $CanRepair = $false }
  }
  if ($CanRepair) {
    $Arguments = @("setup", "--agent", $ExistingAgent, "--state-dir", $StateDir)
    $Cli = $ExistingCli
  } else {
    $Cli = Stage-InfraBinary "infra"
    $Agent = Stage-InfraBinary "infra-agent"
    $Info = (& $Cli --build-info | Out-String | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0 -or $Info.activation_protocol -ne 1) {
      throw "This release predates safe native activation; use a compatible candidate release."
    }
    $Arguments = @("activate", "--source-dir", $TempDir, "--install-dir", $InstallDir, "--state-dir", $StateDir)
  }
  if (-not [string]::IsNullOrWhiteSpace($Gateway)) { $Arguments += @("--gateway", $Gateway) }
  if (-not [string]::IsNullOrWhiteSpace($Code)) { $Arguments += @("--code", $Code) }
  if (-not [string]::IsNullOrWhiteSpace($Name) -and -not [string]::IsNullOrWhiteSpace($Code)) { $Arguments += @("--name", $Name) }
  if ($NoStartup) { $Arguments += "--no-startup" }
  & $Cli @Arguments
  if ($LASTEXITCODE -ne 0) { throw "Native activation/setup failed (exit $LASTEXITCODE). Identity and recovery files were retained; rerun the verified installer." }
  Write-Host "Infra native setup completed. State: $StateDir"
} finally {
  Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}
