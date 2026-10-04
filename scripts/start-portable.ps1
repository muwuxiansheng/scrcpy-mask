# Distributed at the root of the Windows portable package.
$ErrorActionPreference = 'Stop'
$packageRoot = $PSScriptRoot
$exePath = Join-Path $packageRoot 'scrcpy-mask.exe'
$dataPath = Join-Path $packageRoot 'data'
$configPath = Join-Path $dataPath 'config.json'
$adbPath = Join-Path $packageRoot 'assets\platform-tools\adb.exe'
if (-not (Test-Path -LiteralPath $exePath)) { throw '请先完整解压下载包，再运行启动脚本。' }
New-Item -ItemType Directory -Path $dataPath -Force | Out-Null
$mappingPath = Join-Path $dataPath 'mapping\default.json'
if (-not (Test-Path -LiteralPath $mappingPath)) {
    New-Item -ItemType Directory -Path (Split-Path $mappingPath -Parent) -Force | Out-Null
    # Matches the application's native empty default; never overwrite user layouts.
    [IO.File]::WriteAllText($mappingPath, '{"version":"0.0.1","original_size":{"width":2560,"height":1440},"mappings":[]}', [Text.UTF8Encoding]::new($false))
}
if (-not (Test-Path -LiteralPath $configPath)) {
    $config = @{web_port=27809;controller_port=27808;web_bind_addr='127.0.0.1';language='zh-CN';adb_path='adb';vsync=$false}
} else {
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($config.adb_path -ne 'adb' -and -not (Test-Path -LiteralPath $config.adb_path)) {
        $config.adb_path = $adbPath
    }
}
[IO.File]::WriteAllText($configPath, ($config | ConvertTo-Json -Depth 64), [Text.UTF8Encoding]::new($false))
$env:SCRCPY_MASK_DATA_DIR = $dataPath
$serviceUrl = "http://127.0.0.1:$($config.web_port)"
$expectedAdb = if ($config.adb_path -eq 'adb') { $adbPath } else { $config.adb_path }
$running = @(Get-Process -Name 'scrcpy-mask' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exePath })
if ($running.Count -eq 0) {
    Start-Process -FilePath $exePath -WorkingDirectory $packageRoot -WindowStyle Hidden -RedirectStandardError (Join-Path $dataPath 'startup-error.log') | Out-Null
}
$ready = $false
for ($attempt=0; $attempt -lt 30; $attempt++) {
    try {
        $response = Invoke-RestMethod "$serviceUrl/api/config/get_config" -TimeoutSec 1
        if ($response.code -eq 200 -and $response.data.adb_path -eq $expectedAdb) { $ready=$true;break }
    } catch {}
    Start-Sleep -Milliseconds 500
}
if (-not $ready) { throw "服务启动失败或端口被另一份程序占用，请检查 $dataPath\startup-error.log。" }
Start-Process $serviceUrl | Out-Null
