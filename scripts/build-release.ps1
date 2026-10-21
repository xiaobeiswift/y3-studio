$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $ProjectRoot

# 1. Parse version from Cargo.toml
$CargoToml = Get-Content "Cargo.toml" -Raw -Encoding UTF8
if ($CargoToml -match '(?m)^version\s*=\s*"([^"]+)"') {
    $Version = $Matches[1]
} else {
    $Version = "0.0.1"
}

Write-Host "=====================================================" -ForegroundColor Cyan
Write-Host "  Building Y3 Studio v$Version Release (Windows x64) " -ForegroundColor Cyan
Write-Host "=====================================================" -ForegroundColor Cyan

# 2. Handle running binaries
$RunningExes = Get-Process y3-helper -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Path -ErrorAction SilentlyContinue
if ($RunningExes -contains (Join-Path $ProjectRoot "target\release\y3-helper.exe")) {
    $BakName = "target\release\y3-helper.$((Get-Date).Ticks).old"
    Move-Item -Force "target\release\y3-helper.exe" $BakName -ErrorAction SilentlyContinue
}
if ($RunningExes -contains (Join-Path $ProjectRoot "y3-helper.exe")) {
    $BakName = "y3-helper.$((Get-Date).Ticks).old"
    Move-Item -Force "y3-helper.exe" $BakName -ErrorAction SilentlyContinue
}

# 3. Build y3-helper.exe
Write-Host "[1/4] Building main binary y3-helper.exe ..." -ForegroundColor Yellow
cargo build --release --bin y3-helper
if ($LASTEXITCODE -ne 0) {
    throw "cargo build y3-helper failed"
}

Copy-Item -Force "target\release\y3-helper.exe" "y3-helper.exe" -ErrorAction SilentlyContinue

# 4. Assemble Portable ZIP
$DistDir = Join-Path $ProjectRoot "dist"
$PortableName = "Y3-Studio-v$Version-win-x64-portable"
$PortableDir = Join-Path $DistDir $PortableName
$PortableZip = Join-Path $DistDir "$PortableName.zip"
$SetupExe = Join-Path $DistDir "Y3-Studio-v$Version-win-x64-setup.exe"

if (Test-Path $PortableDir) { Remove-Item -Recurse -Force $PortableDir }
if (Test-Path $PortableZip) { Remove-Item -Force $PortableZip }
New-Item -ItemType Directory -Force -Path $PortableDir | Out-Null

Copy-Item -Force "target\release\y3-helper.exe" (Join-Path $PortableDir "y3-helper.exe")
Copy-Item -Force "README.md" (Join-Path $PortableDir "README.md")

Write-Host "[2/4] Packaging Portable ZIP: $PortableZip ..." -ForegroundColor Yellow
Compress-Archive -Path $PortableDir -DestinationPath $PortableZip -CompressionLevel Optimal -Force

# 5. Create embedded installer payload
$PayloadZip = Join-Path $ProjectRoot "target\installer-payload.zip"
if (Test-Path $PayloadZip) { Remove-Item -Force $PayloadZip }
Compress-Archive -Path "$PortableDir\*" -DestinationPath $PayloadZip -CompressionLevel Optimal -Force

# 6. Build Windows Installer EXE
Write-Host "[3/4] Building Windows Setup Installer: $SetupExe ..." -ForegroundColor Yellow
cargo build --release --bin y3-studio-setup
if ($LASTEXITCODE -ne 0) {
    throw "cargo build y3-studio-setup failed"
}
Copy-Item -Force "target\release\y3-studio-setup.exe" $SetupExe

# 7. Generate SHA256SUMS.txt
Write-Host "[4/4] Generating SHA256SUMS.txt ..." -ForegroundColor Yellow
$SetupHash = (Get-FileHash -Path $SetupExe -Algorithm SHA256).Hash.ToLower()
$ZipHash = (Get-FileHash -Path $PortableZip -Algorithm SHA256).Hash.ToLower()
$SumsContent = "$SetupHash  Y3-Studio-v$Version-win-x64-setup.exe`r`n$ZipHash  Y3-Studio-v$Version-win-x64-portable.zip`r`n"
[System.IO.File]::WriteAllText((Join-Path $DistDir "SHA256SUMS.txt"), $SumsContent, [System.Text.Encoding]::ASCII)

Write-Host ""
Write-Host "Release packages created successfully in dist\ :" -ForegroundColor Green
Get-ChildItem $DistDir -File | Select-Object Name, @{Name="Size(MB)";Expression={[math]::Round($_.Length / 1MB, 2)}}, LastWriteTime | Format-Table -AutoSize
