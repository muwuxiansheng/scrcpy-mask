param(
    [Parameter(Mandatory=$true)][string]$Version,
    [Parameter(Mandatory=$true)][string]$AdbDirectory,
    [string]$RuntimeDirectory
)
$ErrorActionPreference='Stop'
$projectRoot=Split-Path $PSScriptRoot -Parent
if ($Version -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]+$') { throw 'Invalid version label.' }
$releaseRoot=Join-Path $projectRoot 'target\release'
$packageName="scrcpy-mask-$Version-windows-x64"
$packageRoot=Join-Path $releaseRoot $packageName
if (Test-Path -LiteralPath $packageRoot) { throw "Package staging directory already exists: $packageRoot" }
foreach($required in @('target\release\scrcpy-mask.exe','assets\mask-pointer.jar','assets\web\index.html')) {
    if (-not (Test-Path -LiteralPath (Join-Path $projectRoot $required))) { throw "Missing build artifact: $required" }
}
foreach($required in @('adb.exe','AdbWinApi.dll','AdbWinUsbApi.dll','NOTICE.txt')) {
    if (-not (Test-Path -LiteralPath (Join-Path $AdbDirectory $required))) { throw "Missing ADB artifact: $required" }
}
New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $releaseRoot 'scrcpy-mask.exe') -Destination $packageRoot
Copy-Item -LiteralPath (Join-Path $projectRoot 'assets') -Destination $packageRoot -Recurse
$adbTarget=Join-Path $packageRoot 'assets\platform-tools'
New-Item -ItemType Directory -Path $adbTarget -Force | Out-Null
foreach($file in @('adb.exe','AdbWinApi.dll','AdbWinUsbApi.dll','NOTICE.txt','source.properties')) {
    if (Test-Path -LiteralPath (Join-Path $AdbDirectory $file)) { Copy-Item -LiteralPath (Join-Path $AdbDirectory $file) -Destination $adbTarget -Force }
}
if ($RuntimeDirectory) {
    # App-local runtime from the MSVC redistributable directory, never from System32.
    foreach($runtime in @('vcruntime140.dll','vcruntime140_1.dll')) {
        $path=Join-Path $RuntimeDirectory $runtime
        if (-not (Test-Path -LiteralPath $path)) { throw "Missing redistributable runtime: $path" }
        Copy-Item -LiteralPath $path -Destination $packageRoot
    }
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'start-portable.ps1') -Destination $packageRoot
$starter=Join-Path $packageRoot 'start-portable.ps1'
[IO.File]::WriteAllText($starter,(Get-Content -LiteralPath $starter -Raw -Encoding UTF8),[Text.UTF8Encoding]::new($true))
Copy-Item -LiteralPath (Join-Path $projectRoot 'examples') -Destination $packageRoot -Recurse
foreach($file in @('LICENSE','CUSTOMIZATION-zh.md','ROADMAP-zh.md','scripts-help-zh.md')) {
    Copy-Item -LiteralPath (Join-Path $projectRoot $file) -Destination $packageRoot
}
$command=@'
@echo off
cd /d "%~dp0"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-portable.ps1"
if errorlevel 1 pause
'@
[IO.File]::WriteAllText((Join-Path $packageRoot 'Start.cmd'),$command,[Text.ASCIIEncoding]::new())
$readme=@"
# Scrcpy Mask $Version Windows x64 修改版

完整解压到有写入权限的目录，双击 Start.cmd。使用便携启动方式时，配置存放在同目录 data 文件夹。
连接安卓手机，开启 USB 调试并授权，再在打开的设备页面点击控制。首次连接可关闭视频和音频。
不要只把 exe 单独复制出去：assets、ADB 和手机指针组件必须一起保留。

此包适用于 Windows 10/11 x64。已附带 ADB、手机指针组件和应用本地 VC 运行库（若打包时提供）。
如果系统仍提示缺少 VC 运行库，可安装微软官方 x64 运行库：https://aka.ms/vs/17/release/vc_redist.x64.exe

修改内容和构建说明见 CUSTOMIZATION-zh.md，脚本说明见 scripts-help-zh.md，后续计划见 ROADMAP-zh.md。
M14 测试曲线见 examples/recoil，未实测校准；新安装默认关闭压枪。
这个示例导入会替换压枪方案列表，先导出已有方案可保留原配置。

不包含个人设备序列号、截图、日志、个人游戏布局及第三方压枪配置原文。
在当前电脑测试过启动与连接；其他电脑的游戏手感和设备兼容性仍需实际验证。
"@
[IO.File]::WriteAllText((Join-Path $packageRoot '使用说明.md'),$readme,[Text.UTF8Encoding]::new($false))
$zipPath=Join-Path $releaseRoot "$packageName.zip"
Compress-Archive -LiteralPath $packageRoot -DestinationPath $zipPath -Force
$hash=Get-FileHash -LiteralPath $zipPath -Algorithm SHA256
[IO.File]::WriteAllText("$zipPath.sha256", "$($hash.Hash.ToLowerInvariant())  $([IO.Path]::GetFileName($zipPath))`n",[Text.ASCIIEncoding]::new())
Write-Output $zipPath
