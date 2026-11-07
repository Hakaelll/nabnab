param([ValidateSet('status','synthesize')][string]$Action,[string]$InputPath)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=New-Object System.Text.UTF8Encoding($false)
function Hash-Text([string]$Text) {
    $sha=[Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text)))).Replace('-','').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
function Describe-Voice($Installed,[string]$Engine) {
    $info=$Installed.VoiceInfo
    if($info.Id -notmatch '^TTS_MS_[A-Za-z0-9_.-]+$' -or $info.AdditionalInfo['Vendor'] -ne 'Microsoft' -or $info.Name -notlike 'Microsoft * Desktop') { return $null }
    $identity=Hash-Text ($info.Id+"`n"+$info.Name+"`n"+$info.Culture.Name)
    $details=($info.AdditionalInfo.Keys | Sort-Object | ForEach-Object { $_+"`t"+$info.AdditionalInfo[$_]+"`n" }) -join ''
    $fingerprint=Hash-Text ($Engine+"`n"+$identity+"`n"+$details)
    return [pscustomobject]@{id=$identity;name=$info.Name;language=$info.Culture.Name;enabled=[bool]$Installed.Enabled;fingerprint=$fingerprint}
}
$synth=$null
try {
    Add-Type -AssemblyName System.Speech
    $synth=New-Object System.Speech.Synthesis.SpeechSynthesizer
    $assembly=[System.Speech.Synthesis.SpeechSynthesizer].Assembly
    $assemblyVersion=[Diagnostics.FileVersionInfo]::GetVersionInfo($assembly.Location).FileVersion
    $sapi=Join-Path ([Environment]::SystemDirectory) 'Speech\Common\sapi.dll'
    $sapiVersion=if([IO.File]::Exists($sapi)){[Diagnostics.FileVersionInfo]::GetVersionInfo($sapi).FileVersion}else{'not reported'}
    $engine='System.Speech '+$assembly.GetName().Version.ToString()+' / '+$assemblyVersion+'; SAPI '+$sapiVersion+'; Windows '+[Environment]::OSVersion.Version.ToString()
    $voices=@($synth.GetInstalledVoices() | ForEach-Object { Describe-Voice $_ $engine } | Where-Object { $null -ne $_ })
    if($Action -eq 'status') {
        $status=[pscustomobject]@{available=[bool](@($voices | Where-Object {$_.enabled}).Count);engineVersion=$engine;voices=$voices;note='Uses installed Microsoft Desktop SAPI voices through System.Speech. No voice binaries are bundled or downloaded; Windows voice terms apply. Third-party and online engines are not listed.'}
        [Console]::WriteLine(($status | ConvertTo-Json -Depth 5 -Compress))
        exit 0
    }
    $request=[IO.File]::ReadAllText($InputPath,[Text.Encoding]::UTF8) | ConvertFrom-Json
    $voice=@($voices | Where-Object {$_.id -eq $request.voiceId -and $_.enabled})
    if($voice.Count -ne 1 -or $voice[0].fingerprint -ne $request.engineFingerprint) { throw 'The selected desktop voice changed.' }
    $synth.SelectVoice($voice[0].name)
    $synth.Rate=[int]$request.rate
    $synth.Volume=[int]$request.volume
    $format=New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(22050,[System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,[System.Speech.AudioFormat.AudioChannel]::Mono)
    $items=@($request.items)
    for($index=0;$index -lt $items.Count;$index++) {
        $item=$items[$index]
        if([IO.File]::Exists($item.output) -or [IO.File]::Exists($item.partial)) { throw 'A synthesis workspace output already exists.' }
        $synth.SetOutputToWaveFile([string]$item.partial,$format)
        $synth.Speak([string]$item.text)
        $synth.SetOutputToNull()
        [IO.File]::Move([string]$item.partial,[string]$item.output)
        [Console]::WriteLine('__PROGRESS__'+(@{downloaded_bytes=$index+1;total_bytes=$items.Count} | ConvertTo-Json -Compress))
    }
} catch {
    if($Action -eq 'status') {
        [Console]::WriteLine('{"available":false,"engineVersion":"","voices":[],"note":"Windows desktop speech is unavailable. Check System.Speech and installed Microsoft Desktop SAPI voices. No voice or service was downloaded."}')
        exit 0
    }
    [Console]::Error.WriteLine('Windows desktop speech could not finish. Verify that the selected Microsoft Desktop voice is still installed and available, then retry. Completed local segments were preserved.')
    exit 1
} finally {
    if($null -ne $synth) { $synth.Dispose() }
}
