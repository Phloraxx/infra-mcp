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

if (-not (Test-Path "$StateDir\identity.json") -and [string]::IsNullOrWhiteSpace($Code)) {
  throw "-Code is required for a device that is not already enrolled"
}

$Arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
  "X64" { "x86_64" }
  "Arm64" { "aarch64" }
  default { throw "Unsupported Windows architecture: $_" }
}

New-Item -ItemType Directory -Force -Path $InstallDir, $StateDir | Out-Null
$TempDir = Join-Path ([IO.Path]::GetTempPath()) ("infra-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $TempDir | Out-Null

function Install-InfraBinary([string]$Name) {
  $Destination = Join-Path $InstallDir "$Name.exe"
  $TempDestination = "$Destination.tmp"
  if (-not [string]::IsNullOrWhiteSpace($SourceDir)) {
    $Source = Join-Path $SourceDir "$Name.exe"
    if (-not (Test-Path $Source)) { throw "Missing development binary $Source" }
    Copy-Item $Source $TempDestination -Force
  } else {
    $Asset = "$Name-windows-$Arch.exe"
    if ($Version -eq "latest") {
      $Base = "https://github.com/$Repo/releases/latest/download"
    } else {
      $Base = "https://github.com/$Repo/releases/download/$Version"
    }
    $AssetPath = Join-Path $TempDir $Asset
    $ChecksumPath = "$AssetPath.sha256"
    Invoke-WebRequest -UseBasicParsing -Uri "$Base/$Asset" -OutFile $AssetPath
    Invoke-WebRequest -UseBasicParsing -Uri "$Base/$Asset.sha256" -OutFile $ChecksumPath
    $Expected = ((Get-Content $ChecksumPath -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
    $Actual = (Get-FileHash -Algorithm SHA256 $AssetPath).Hash.ToLowerInvariant()
    if ($Expected -ne $Actual) { throw "SHA-256 verification failed for $Asset" }
    Copy-Item $AssetPath $TempDestination -Force
  }
  Move-Item $TempDestination $Destination -Force
}

try {
  Write-Host "Infra · installing native binaries"
  Install-InfraBinary "infra"
  Install-InfraBinary "infra-agent"

  $Cli = Join-Path $InstallDir "infra.exe"
  $Agent = Join-Path $InstallDir "infra-agent.exe"
  $Arguments = @("install", "--gateway", $Gateway, "--agent", $Agent, "--state-dir", $StateDir)
  if (-not [string]::IsNullOrWhiteSpace($Code)) { $Arguments += @("--code", $Code) }
  if (-not [string]::IsNullOrWhiteSpace($Name)) { $Arguments += @("--name", $Name) }
  if ($NoStartup) { $Arguments += "--no-startup" }

  & $Cli @Arguments
  if ($LASTEXITCODE -ne 0) { throw "Infra CLI installation failed with exit code $LASTEXITCODE" }

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
