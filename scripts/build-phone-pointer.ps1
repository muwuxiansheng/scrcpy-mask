param(
    [Parameter(Mandatory = $true)][string]$R8Jar,
    [string]$JavaHome = $env:JAVA_HOME
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$classDir = Join-Path $projectRoot 'target\phone-pointer\classes'
$dexDir = Join-Path $projectRoot 'target\phone-pointer\dex'
New-Item -ItemType Directory -Path $classDir, $dexDir -Force | Out-Null
$javac = if ($JavaHome) { Join-Path $JavaHome 'bin\javac.exe' } else { (Get-Command javac -ErrorAction Stop).Source }
$java = if ($JavaHome) { Join-Path $JavaHome 'bin\java.exe' } else { (Get-Command java -ErrorAction Stop).Source }
$r8Path = (Resolve-Path -LiteralPath $R8Jar).Path
& $javac --release 8 -d $classDir (Join-Path $projectRoot 'phone-pointer\MaskPointer.java')
if ($LASTEXITCODE -ne 0) { throw 'Java compilation failed.' }
& $java -cp $r8Path com.android.tools.r8.D8 --min-api 26 --output $dexDir (Join-Path $classDir 'MaskPointer.class')
if ($LASTEXITCODE -ne 0) { throw 'D8 compilation failed.' }
Add-Type -AssemblyName System.IO.Compression
$jarPath = Join-Path $projectRoot 'assets\mask-pointer.jar'
$file = [IO.File]::Open($jarPath, [IO.FileMode]::Create)
try {
    $zip = [IO.Compression.ZipArchive]::new($file, [IO.Compression.ZipArchiveMode]::Create)
    try {
        $entry = $zip.CreateEntry('classes.dex')
        $output = $entry.Open()
        $input = [IO.File]::OpenRead((Join-Path $dexDir 'classes.dex'))
        try { $input.CopyTo($output) } finally { $input.Dispose(); $output.Dispose() }
    } finally { $zip.Dispose() }
} finally { $file.Dispose() }
Write-Host "Phone pointer built: $jarPath"
