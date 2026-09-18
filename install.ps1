# Compile Toolbelt en release et l'installe dans %LOCALAPPDATA%\Toolbelt.
#
# Pourquoi ne pas juste lancer depuis target\release : le chemin enregistré
# pour le démarrage automatique doit être stable. Un cargo clean ou un
# déplacement du dossier casserait le démarrage en silence.

$ErrorActionPreference = "Stop"

$dest = Join-Path $env:LOCALAPPDATA "Toolbelt"
$exe  = Join-Path $dest "toolbelt.exe"

Write-Host "Compilation en release..." -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "la compilation a échoué" }

$built = Join-Path $PSScriptRoot "target\release\toolbelt.exe"
if (-not (Test-Path $built)) { throw "binaire introuvable : $built" }

# Une instance déjà lancée verrouille le fichier.
Get-Process toolbelt -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item $built $exe -Force

$size = [math]::Round((Get-Item $exe).Length / 1MB, 1)
Write-Host "Installé : $exe ($size Mo)" -ForegroundColor Green

Start-Process $exe
Write-Host ""
Write-Host "Lancé. Coche « Démarrer avec Windows » depuis cette instance." -ForegroundColor Yellow
