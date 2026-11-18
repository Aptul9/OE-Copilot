# Prova il download preparato da pacchetto-windows.ps1 come lo riceve chi lo scarica: il JSON contro l'exe (campi, sha256, byte,
# nome, commit, versione uguale a quella dentro l'exe, exe x64), poi l'exe copiato come "OE Copilot.exe" in una cartella nuova e
# avviato: dopo 10 secondi deve essere ancora vivo, con la sua WebView2. Poi lo ferma e aspetta che lui e la WebView2 siano usciti.
# Non tocca i dati dell'app (%LOCALAPPDATA%\com.oecopilot.desktop): sul PC di chi la usa si rifiuta se OE Copilot è aperto.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Exe)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$name = [IO.Path]::GetFileNameWithoutExtension($Exe)
$meta = Get-Content (Join-Path (Split-Path $Exe -Parent) "$name.json") -Raw | ConvertFrom-Json

$keys = ($meta.PSObject.Properties.Name | Sort-Object) -join ','
if ($keys -ne 'bytes,commit,file,sha256,version') { throw "campi del JSON: $keys" }
$hash = (Get-FileHash $Exe).Hash.ToLowerInvariant()
if ($meta.sha256 -ne $hash) { throw "sha256 del JSON $($meta.sha256) diverso da quello dell'exe $hash" }
if ([long]$meta.bytes -ne (Get-Item $Exe).Length) { throw "byte del JSON $($meta.bytes) diversi dall'exe $((Get-Item $Exe).Length)" }
if ($meta.file -ne [IO.Path]::GetFileName($Exe)) { throw "file del JSON $($meta.file) non è il nome dell'exe" }
if ($meta.commit -notmatch '^[0-9a-f]{40}$') { throw "commit del JSON $($meta.commit) non è uno sha" }
if ($meta.version -notmatch '^\d+\.\d+\.\d+\.[1-9]\d*$') { throw "versione del JSON $($meta.version) non è major.minor.patch.build" }
$info = (Get-Item $Exe).VersionInfo
$inExe = '{0}.{1}.{2}.{3}' -f $info.FileMajorPart, $info.FileMinorPart, $info.FileBuildPart, $info.FilePrivatePart
if ($inExe -ne $meta.version) { throw "versione dell'exe $inExe diversa dal JSON $($meta.version)" }
if ($info.ProductName -ne 'OE Copilot') { throw "ProductName dell'exe '$($info.ProductName)'" }
# intestazione PE: Machine 0x8664 = x64
$pe = [IO.File]::ReadAllBytes($Exe)
$offset = [BitConverter]::ToInt32($pe, 0x3C)
$machine = [BitConverter]::ToUInt16($pe, $offset + 4)
if ($machine -ne 0x8664) { throw ('exe non x64: Machine 0x{0:X4}' -f $machine) }

if (Get-Process -Name 'OE Copilot', 'oe-copilot-app' -ErrorAction SilentlyContinue) { throw 'OE Copilot è aperto: chiuderlo prima della prova' }

function Discendenti([int]$Id) {
    $tutti = @(Get-CimInstance Win32_Process)
    $trovati = @()
    $coda = [Collections.Generic.Queue[int]]::new(); $coda.Enqueue($Id)
    while ($coda.Count) {
        $p = $coda.Dequeue()
        foreach ($figlio in $tutti | Where-Object { $_.ParentProcessId -eq $p -and $_.ProcessId -ne $p }) { $trovati += $figlio; $coda.Enqueue([int]$figlio.ProcessId) }
    }
    $trovati
}

$log = Join-Path $env:LOCALAPPDATA 'com.oecopilot.desktop\errore.log'
$logPrima = if (Test-Path $log) { (Get-Item $log).Length } else { 0 }
$work = Join-Path ([IO.Path]::GetTempPath()) ('oe-copilot-prova-' + [guid]::NewGuid().ToString('N'))
$installato = Join-Path $work 'OE Copilot\OE Copilot.exe'
$app = $null; $figli = @()
try {
    New-Item -ItemType Directory -Force (Split-Path $installato -Parent) | Out-Null
    Copy-Item $Exe $installato
    $app = Start-Process $installato -PassThru
    $null = $app.Handle
    for ($i = 0; $i -lt 20 -and -not $app.HasExited; $i++) { Start-Sleep -Milliseconds 500 }
    if ($app.HasExited) { throw "l'app è uscita subito, codice $($app.ExitCode)" }
    $figli = @(Discendenti $app.Id)
    $webview = @($figli | Where-Object Name -eq 'msedgewebview2.exe')
    if (-not $webview) { throw "l'app è viva ma senza WebView2: processi figli $(($figli.Name | Sort-Object -Unique) -join ', ')" }
    $finestra = (Get-Process -Id $app.Id).MainWindowTitle
    $logOra = if (Test-Path $log) { (Get-Item $log).Length } else { 0 }
    if ($logOra -gt $logPrima) { throw "errore.log scritto durante l'avvio: $((Get-Content $log -Raw).Substring($logPrima))" }

    $vivoDopo = [int]((Get-Date) - $app.StartTime).TotalSeconds
    Stop-Process -Id $app.Id -Force
    if (-not $app.WaitForExit(30000)) { throw "l'app non è uscita 30 secondi dopo lo stop" }
    # la WebView2 esce da sola quando muore il processo che la ospita: si aspetta che lo faccia, prima di cancellare la cartella
    $restano = @()
    for ($i = 0; $i -lt 60; $i++) {
        $restano = @($figli | Where-Object { Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue })
        if (-not $restano) { break }
        Start-Sleep -Milliseconds 500
    }
    if ($restano) { throw "processi dell'app ancora vivi 30 secondi dopo lo stop: $(($restano | ForEach-Object { "$($_.Name) $($_.ProcessId)" }) -join ', ')" }

    [pscustomobject]@{ version = $meta.version; pid = $app.Id; vivo_secondi = $vivoDopo; webview2 = $webview.Count; finestra = $finestra } | ConvertTo-Json -Compress
    "pacchetto ok: $($meta.file) $($meta.version) $hash"
}
finally {
    if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue; $null = $app.WaitForExit(30000) }
    foreach ($p in $figli) { Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue }
    foreach ($p in $figli) { Wait-Process -Id $p.ProcessId -Timeout 30 -ErrorAction SilentlyContinue }
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    if (Test-Path $work) { Write-Warning "cartella di prova non cancellata: $work" }
}
