# Installs or updates CoBox on Windows:
#   irm https://raw.githubusercontent.com/erfjab/CoBox/master/install.ps1 | iex
# Only the program file is replaced. History and settings in %APPDATA%\cobox are never touched.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$dir = "$env:LOCALAPPDATA\Programs\CoBox"
$exe = "$dir\cobox.exe"
$tmp = Join-Path $env:TEMP "cobox-install-$PID"
$update = Test-Path $exe

# Download and unpack first, so a failed download leaves the current install as it is.
Write-Host $(if ($update) { 'Updating CoBox...' } else { 'Installing CoBox...' })
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
    Invoke-WebRequest 'https://github.com/erfjab/CoBox/releases/latest/download/cobox-windows-x64.zip' -OutFile "$tmp\cobox.zip"
    Expand-Archive "$tmp\cobox.zip" "$tmp\new" -Force
    if (-not (Test-Path "$tmp\new\cobox.exe")) { throw 'The download did not contain cobox.exe.' }

    # Close the running copy (the database survives this) and wait until the file is free.
    Get-Process cobox -ErrorAction SilentlyContinue | Stop-Process -Force
    Get-Process cobox -ErrorAction SilentlyContinue | Wait-Process -Timeout 10

    # Swap the program file, putting the old one back if anything goes wrong.
    New-Item -ItemType Directory -Force $dir | Out-Null
    if ($update) { Move-Item $exe "$exe.old" -Force }
    try {
        Copy-Item "$tmp\new\cobox.exe" $exe
    } catch {
        if ($update) { Move-Item "$exe.old" $exe -Force }
        throw
    }
    Remove-Item "$exe.old" -ErrorAction SilentlyContinue
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

$lnk = (New-Object -ComObject WScript.Shell).CreateShortcut("$env:APPDATA\Microsoft\Windows\Start Menu\Programs\CoBox.lnk")
$lnk.TargetPath = $exe
$lnk.Save()

Start-Process $exe
if ($update) { Write-Host 'CoBox updated. Your history and settings are kept.' }
else { Write-Host "CoBox installed to $dir. Press Alt+V to open it." }
