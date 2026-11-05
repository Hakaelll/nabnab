$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
$executable = [IO.Path]::GetFullPath((Join-Path $workspace 'src-tauri/target/debug/nabnab.exe'))
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw 'Build the debug application first.' }
if (Get-Process -Name nabnab -ErrorAction SilentlyContinue | Where-Object Path -EQ $executable) {
    throw 'An existing debug application is open. Close that test session before measuring.'
}
$output = Join-Path $workspace ('artifacts/performance/debug-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss'))
[void](New-Item -ItemType Directory -Path $output)
$savedEnvironment = @{}
foreach ($name in 'NABNAB_TEST_DATA_DIR','WEBVIEW2_USER_DATA_FOLDER','WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS') {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
function Get-AppTree([int]$rootProcessId) {
    $rows = @(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId)
    $ids = [Collections.Generic.HashSet[uint32]]::new()
    [void]$ids.Add([uint32]$rootProcessId)
    do {
        $changed = $false
        foreach ($row in $rows) {
            if ($ids.Contains([uint32]$row.ParentProcessId) -and $ids.Add([uint32]$row.ProcessId)) { $changed = $true }
        }
    } while ($changed)
    foreach ($id in $ids) {
        $process = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($process) {
            [pscustomobject]@{ id=$id; name=$process.ProcessName; cpu=$process.CPU; workingSet=$process.WorkingSet64; privateBytes=$process.PrivateMemorySize64 }
        }
    }
}
$runs = @()
try {
    $env:NABNAB_TEST_DATA_DIR = Join-Path $output 'data'
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $output 'webview'
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $null
    foreach ($number in 1,2) {
        $watch = [Diagnostics.Stopwatch]::StartNew()
        $appProcess = Start-Process -FilePath $executable -WindowStyle Hidden -PassThru
        try {
            $ready = $false
            while ($watch.Elapsed.TotalSeconds -lt 30) {
                $appProcess.Refresh()
                if ($appProcess.HasExited) { throw ('The debug app exited with code ' + $appProcess.ExitCode) }
                if ($appProcess.MainWindowHandle -ne 0 -and $appProcess.MainWindowTitle -eq 'nabnab') { $ready = $true; break }
                Start-Sleep -Milliseconds 100
            }
            if (-not $ready) { throw 'The native nabnab window did not appear.' }
            $watch.Stop()
            Start-Sleep -Seconds 2
            $before = @(Get-AppTree $appProcess.Id)
            $idleWatch = [Diagnostics.Stopwatch]::StartNew()
            Start-Sleep -Seconds 5
            $after = @(Get-AppTree $appProcess.Id)
            $idleWatch.Stop()
            $stable = ($before.id | Sort-Object) -join ','
            $stable = $stable -eq (($after.id | Sort-Object) -join ',')
            $cpuDelta = [double]($after | Measure-Object cpu -Sum).Sum - [double]($before | Measure-Object cpu -Sum).Sum
            $cpu = [Math]::Max([double]0, $cpuDelta)
            $runs += [pscustomobject]@{
                launch=$number; windowObserved=$ready; nativeWindowMilliseconds=$watch.Elapsed.TotalMilliseconds
                sampleSeconds=$idleWatch.Elapsed.TotalSeconds; processSetStable=$stable; cpuSeconds=$cpu
                machineNormalizedCpuPercent=100*$cpu/$idleWatch.Elapsed.TotalSeconds/[Environment]::ProcessorCount
                processTreeWorkingSetBytes=($after | Measure-Object workingSet -Sum).Sum
                processTreePrivateBytes=($after | Measure-Object privateBytes -Sum).Sum
                before=$before; after=$after
            }
        } finally {
            $appProcess.Refresh()
            if (-not $appProcess.HasExited) {
                [void]$appProcess.CloseMainWindow()
                if (-not $appProcess.WaitForExit(3000)) {
                    $owned = Get-Process -Id $appProcess.Id -ErrorAction SilentlyContinue
                    if ($owned -and $owned.Path -eq $executable) { Stop-Process -Id $appProcess.Id }
                }
            }
            $appProcess.Dispose()
        }
    }
    $report = [pscustomobject]@{
        date=[DateTime]::UtcNow.ToString('o'); mode='Debug executable and Vite; isolated data and WebView profile'
        logicalCores=[Environment]::ProcessorCount; requestedLaunchStyle='Hidden'; runs=$runs
        appDataBytes=(Get-ChildItem -LiteralPath $env:NABNAB_TEST_DATA_DIR -File -Recurse | Measure-Object Length -Sum).Sum
        webViewProfileBytes=(Get-ChildItem -LiteralPath $env:WEBVIEW2_USER_DATA_FOLDER -File -Recurse | Measure-Object Length -Sum).Sum
        caveats='Debug builds use the console subsystem, so WaitForInputIdle is unavailable. Window appearance is not first paint. Two sequential launches are not controlled cold/warm runs. Shared WebView working sets include shared pages. GPU was not measured. Debug/Vite, startup signal, window size and cache state differ from the installed 0.1 baseline; no speedup or regression can be inferred.'
    }
    $report | ConvertTo-Json -Depth 7 | Set-Content -LiteralPath (Join-Path $output 'native.json') -Encoding utf8
    $report | Select-Object date,mode,runs,caveats | ConvertTo-Json -Depth 4
    Write-Output ('Artifact directory: ' + $output)
} finally {
    foreach ($name in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process') }
}
