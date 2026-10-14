param([string]$Code=$env:INFRA_PAIR_CODE,[string]$Name='', [string]$Server='https://mcp.phloraxx.us.to',[switch]$System,[switch]$Uninstall)
$ErrorActionPreference='Stop'
[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12
$NodeVersion='v22.23.3'
$Base=Join-Path $env:USERPROFILE '.infra-mcp'
$TaskName='InfraMCPAgent'
if($System){
  $isAdmin=([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  if(-not $isAdmin){throw 'Use an elevated PowerShell window for -System. A user install needs no administrator access.'}
  $Base=Join-Path $env:ProgramData 'InfraMCP';$TaskName='InfraMCPSystemAgent'
}
$RunKey='HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
function Stop-InstalledAgent {
  Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
  Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -and ($_.CommandLine.Contains((Join-Path $Base 'agent.mjs')) -or $_.CommandLine.Contains((Join-Path $Base 'run-agent.ps1'))) } | ForEach-Object { if($_.ProcessId -ne $PID){Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue} }
}
if($Uninstall){Stop-InstalledAgent;Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue;Remove-ItemProperty -Path $RunKey -Name $TaskName -ErrorAction SilentlyContinue;Write-Host "Startup removed. Configuration and logs remain in $Base. Remove the device from the dashboard to revoke its credential.";return}
if(-not ($Server.StartsWith('https://') -or $Server.StartsWith('http://127.0.0.1:') -or $Server.StartsWith('http://localhost:'))){throw 'Use HTTPS for the gateway URL.'}
Write-Host 'Infra MCP - device agent'
New-Item -ItemType Directory -Force -Path $Base | Out-Null
# Credentials are readable only by this user, SYSTEM and administrators.
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
& icacls.exe $Base /inheritance:r /grant:r "*${sid}:(OI)(CI)F" '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' | Out-Null
if($LASTEXITCODE -ne 0){throw 'Could not secure the installation directory.'}
$node=Get-Command node.exe -ErrorAction SilentlyContinue
$NodePath=if($node){$node.Source}else{''}
$usePrivate=$true
if($NodePath){& $NodePath -e 'process.exit(Number(process.versions.node.split(".")[0])>=22?0:1)';$usePrivate=$LASTEXITCODE -ne 0}
$Work=Join-Path ([IO.Path]::GetTempPath()) ('infra-install-'+[Guid]::NewGuid())
New-Item -ItemType Directory $Work | Out-Null
try {
  if($usePrivate){
    $arch=if($env:PROCESSOR_ARCHITECTURE -eq 'ARM64'){'arm64'}else{'x64'}
    $archive="node-$NodeVersion-win-$arch.zip"
    Write-Host 'Installing a private Node runtime. No administrator access is needed.'
    Invoke-WebRequest "https://nodejs.org/dist/$NodeVersion/$archive" -OutFile (Join-Path $Work 'node.zip') -UseBasicParsing
    $sums=(Invoke-WebRequest "https://nodejs.org/dist/$NodeVersion/SHASUMS256.txt" -UseBasicParsing).Content
    $line=($sums -split "`n" | Where-Object { ($_ -split '\s+')[-1].Trim() -eq $archive } | Select-Object -First 1)
    if(-not $line){throw 'Runtime checksum is missing.'}
    $expected=($line.Trim() -split '\s+')[0]
    if((Get-FileHash (Join-Path $Work 'node.zip') -Algorithm SHA256).Hash.ToLower() -ne $expected.ToLower()){throw 'Node runtime checksum failed.'}
    Expand-Archive (Join-Path $Work 'node.zip') (Join-Path $Work 'runtime') -Force
    $private=Join-Path $Base 'node';New-Item -ItemType Directory -Force $private | Out-Null
    Copy-Item (Join-Path $Work "runtime\node-$NodeVersion-win-$arch\*") $private -Recurse -Force
    $NodePath=Join-Path $private 'node.exe'
  }
  $manifest=Invoke-RestMethod "$Server/downloads/manifest.json"
  foreach($file in @('agent.mjs','jobs.mjs')){
    $target=Join-Path $Work $file
    Invoke-WebRequest "$Server/downloads/$file" -OutFile $target -UseBasicParsing
    if((Get-FileHash $target -Algorithm SHA256).Hash.ToLower() -ne $manifest.files.$file){throw "Checksum failed for $file"}
    & $NodePath --check $target;if($LASTEXITCODE -ne 0){throw "Invalid agent file: $file"}
  }
  $ConfigPath=Join-Path $Base 'config.json'
  $NewConfig=$null
  if(-not (Test-Path $ConfigPath) -or $Code){
    if(-not $Code){$Code=Read-Host 'Pairing code from your dashboard'}
    if(-not $Name){$default=$env:COMPUTERNAME.ToLower();$Name=Read-Host "Device name [$default]";if(-not $Name){$Name=$default}}
    $body=@{code=$Code;device=$Name;hostname=$env:COMPUTERNAME;platform='win32'} | ConvertTo-Json
    try{$NewConfig=Invoke-RestMethod "$Server/pair" -Method POST -ContentType 'application/json' -Body $body}catch{throw "Pairing failed: $($_.ErrorDetails.Message)"}
  }
  Stop-InstalledAgent
  foreach($file in @('agent.mjs','jobs.mjs')){Copy-Item (Join-Path $Work $file) (Join-Path $Base $file) -Force}
  if($NewConfig){[IO.File]::WriteAllText($ConfigPath,($NewConfig | ConvertTo-Json -Compress),(New-Object Text.UTF8Encoding($false)))}
  $NodeLiteral=$NodePath.Replace("'","''");$BaseLiteral=$Base.Replace("'","''")
  $Startup=if($System){'system-boot'}else{'user-login'}
  $Runner=Join-Path $Base 'run-agent.ps1'
  $RunnerText=@"
`$env:INFRA_STARTUP='$Startup'
Set-Location '$BaseLiteral'
while (`$true) {
  if ((Test-Path '$BaseLiteral\agent.log') -and (Get-Item '$BaseLiteral\agent.log').Length -gt 5242880) { Move-Item '$BaseLiteral\agent.log' '$BaseLiteral\agent.previous.log' -Force }
  & '$NodeLiteral' '$BaseLiteral\agent.mjs' *>> '$BaseLiteral\agent.log'
  Start-Sleep -Seconds 10
}
"@
  [IO.File]::WriteAllText($Runner,$RunnerText,(New-Object Text.UTF8Encoding($false)))
  $arguments='-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "'+$Runner+'"'
  $action=New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $arguments -WorkingDirectory $Base
  $settings=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -MultipleInstances IgnoreNew
  try {
    if($System){$trigger=New-ScheduledTaskTrigger -AtStartup;$principal=New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest}
    else{$trigger=New-ScheduledTaskTrigger -AtLogOn -User ([Security.Principal.WindowsIdentity]::GetCurrent().Name);$principal=New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited}
    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Force | Out-Null
    Start-ScheduledTask -TaskName $TaskName
    Remove-ItemProperty -Path $RunKey -Name $TaskName -ErrorAction SilentlyContinue
  } catch {
    if($System){throw}
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    Write-Host 'Scheduled Task unavailable; using current-user login startup.'
    New-Item -Path $RunKey -Force | Out-Null
    Set-ItemProperty -Path $RunKey -Name $TaskName -Value ('powershell.exe '+$arguments)
    Start-Process powershell.exe -ArgumentList $arguments -WorkingDirectory $Base -WindowStyle Hidden
  }
  # Remove the legacy startup only after the replacement is installed.
  if(-not $System){
    Remove-ItemProperty -Path $RunKey -Name 'PhloraxxMCPAgent' -ErrorAction SilentlyContinue
    $oldBase=Join-Path $env:USERPROFILE '.phloraxx-mcp'
    Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -and ($_.CommandLine.Contains((Join-Path $oldBase 'agent.mjs')) -or $_.CommandLine.Contains((Join-Path $oldBase 'run-agent.cmd'))) } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  }
  $config=Get-Content $ConfigPath -Raw | ConvertFrom-Json
  $connected=$false
  for($i=0;$i -lt 8;$i++){Start-Sleep -Milliseconds 1500;$check=Invoke-RestMethod "$Server/agent-status" -Method POST -ContentType 'application/json' -Body (@{device=$config.device;token=$config.token}|ConvertTo-Json);if($check.online){$connected=$true;break}}
  if(-not $connected){throw "Installed, but connection is not confirmed. Check $Base\agent.log."}
  Write-Host "Connected as '$($config.device)'. Installed in $Base."
  Write-Host $(if($System){'Startup: system boot. System shell access; no interactive desktop.'}else{'Startup: after this Windows user signs in.'})
} finally {Remove-Item $Work -Recurse -Force -ErrorAction SilentlyContinue}
