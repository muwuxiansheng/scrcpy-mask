param(
    [string]$Adb,
    [string]$DeviceId,
    [string]$Jar
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
if (-not $Adb) { $Adb = Join-Path $projectRoot 'assets\platform-tools\adb.exe' }
if (-not $Jar) { $Jar = Join-Path $projectRoot 'assets\mask-pointer.jar' }
$adbPath = (Resolve-Path -LiteralPath $Adb).Path
if (-not $DeviceId) {
    $devices = @(& $adbPath devices | Where-Object { $_ -match '^\S+\s+device$' } | ForEach-Object { ($_ -split '\s+')[0] })
    if ($devices.Count -ne 1) { throw 'Connect one authorized phone or specify -DeviceId.' }
    $DeviceId = $devices[0]
}
& $adbPath -s $DeviceId forward tcp:27821 localabstract:mask-device-pointer
if ($LASTEXITCODE -ne 0) { throw 'ADB forwarding failed.' }
# Do not probe the single-client socket while the app owns an active pointer connection.
$running = & $adbPath -s $DeviceId shell ps -A -o ARGS
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect phone processes.' }
if ($running | Where-Object { $_ -match '^app_process\s+/\s+MaskPointer\s*$' }) {
    Write-Host 'Phone pointer helper is already running.'
    return
}
& $adbPath -s $DeviceId push (Resolve-Path -LiteralPath $Jar).Path /data/local/tmp/mask-pointer.jar
if ($LASTEXITCODE -ne 0) { throw 'Phone pointer upload failed.' }
$logDir = Join-Path $projectRoot 'target\phone-pointer'
New-Item -ItemType Directory -Path $logDir -Force | Out-Null
Start-Process -FilePath $adbPath -ArgumentList @('-s', $DeviceId, 'shell', '"CLASSPATH=/data/local/tmp/mask-pointer.jar app_process / MaskPointer"') -WindowStyle Hidden -RedirectStandardOutput (Join-Path $logDir 'service.log') -RedirectStandardError (Join-Path $logDir 'service-error.log')
Start-Sleep -Milliseconds 1000
$running = & $adbPath -s $DeviceId shell ps -A -o ARGS
if (-not ($running | Where-Object { $_ -match '^app_process\s+/\s+MaskPointer\s*$' })) {
    throw "Phone pointer helper did not start; check $logDir."
}
Write-Host 'Phone pointer helper is ready on TCP 27821.'
