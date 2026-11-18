# Prepara il download di OE Copilot per Windows: l'exe costruito, copiato come OE-Copilot-Windows-x64.exe (pronto da copiare in
# %LOCALAPPDATA%\Programs\OE Copilot\OE Copilot.exe, README "Installare e aggiornare"), e accanto OE-Copilot-Windows-x64.json
# {version, file, sha256, bytes, commit}, il file che legge la pagina /prodotti/ di servizi.everbright.it, più SHA256SUMS.txt.
# La versione viene dall'exe stesso (0.1.0.<numero della build>, OE_BUILD di configura.mjs), così JSON ed exe non possono divergere.
[CmdletBinding()]
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\src-tauri\target\release\oe-copilot-app.exe'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\src-tauri\target\pacchetto')
)
$ErrorActionPreference = 'Stop'
$name = 'OE-Copilot-Windows-x64'

$Exe = (Resolve-Path $Exe).Path
$info = (Get-Item $Exe).VersionInfo
if ($info.ProductName -ne 'OE Copilot') { throw "l'exe $Exe non è OE Copilot (ProductName '$($info.ProductName)')" }
$version = '{0}.{1}.{2}.{3}' -f $info.FileMajorPart, $info.FileMinorPart, $info.FileBuildPart, $info.FilePrivatePart
if ($info.FilePrivatePart -eq 0) { throw "l'exe ha la versione $version, senza il numero della build: costruirlo con OE_BUILD (configura.mjs)" }
$commit = git -C $PSScriptRoot rev-parse HEAD
if ($LASTEXITCODE -ne 0 -or $commit -notmatch '^[0-9a-f]{40}$') { throw 'commit del sorgente non disponibile' }

New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$target = Join-Path $OutputDirectory "$name.exe"
if (Test-Path $target) { throw "il pacchetto c'è già, usare un'altra cartella: $target" }
Copy-Item $Exe $target

$hash = (Get-FileHash $target).Hash.ToLowerInvariant()
$bytes = (Get-Item $target).Length
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'SHA256SUMS.txt'), "$hash  $name.exe`n", [Text.Encoding]::ASCII)
$meta = [ordered]@{ version = $version; file = "$name.exe"; sha256 = $hash; bytes = $bytes; commit = $commit }
[IO.File]::WriteAllText((Join-Path $OutputDirectory "$name.json"), ($meta | ConvertTo-Json -Compress) + "`n", [Text.UTF8Encoding]::new($false))
$meta | ConvertTo-Json -Compress
