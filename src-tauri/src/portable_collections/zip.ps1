param([ValidateSet('Inspect','Extract','PersonalCatalog','PersonalExtract')][string]$Action,[string]$Archive,[string]$Destination,[string]$Report,[string]$Names,[string]$PartRoot)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$utf8 = New-Object System.Text.UTF8Encoding($false,$true)
$personal = $Action.StartsWith('Personal',[StringComparison]::Ordinal)
$zip=[IO.Compression.ZipFile]::OpenRead($Archive)
try {
    if($zip.Entries.Count -gt 10000){throw 'The ZIP exceeds 10000 entries.'}
    $known=New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $entries=New-Object 'System.Collections.Generic.List[object]'
    $total=[UInt64]0
    foreach($entry in $zip.Entries){
        $name=$entry.FullName
        if($name.EndsWith('/')){
            if($personal -and ($name -match '[\\:\x00-\x1f<>"|?*]' -or $name.StartsWith('/') -or $name.Split('/').Count -gt 25 -or $name.Split('/') -contains '..')){throw 'The personal ZIP contains an unsafe directory path.'}
            continue
        }
        if($name.Length -eq 0 -or $name.Length -gt 1024 -or $name -match '[\\:\x00-\x1f<>"|?*]' -or $name.StartsWith('/') -or !$known.Add($name)){throw 'The ZIP contains unsafe or colliding paths.'}
        foreach($part in $name.Split('/')){if($part -eq '' -or $part -eq '.' -or $part -eq '..' -or $part.EndsWith('.') -or $part.EndsWith(' ') -or $part -match '^(?i:CON|PRN|AUX|NUL|COM[0-9]|LPT[0-9])(?:\.|$)'){throw 'The ZIP contains a reserved or traversal path.'}}
        if((($entry.ExternalAttributes -shr 16) -band 61440) -eq 40960){throw 'ZIP symbolic links are not allowed.'}
        if($personal){
            $type=($entry.ExternalAttributes -shr 16) -band 61440
            if(($type -ne 0 -and $type -ne 32768) -or ($entry.ExternalAttributes -band 1024) -ne 0 -or $name.Split('/').Count -gt 24){throw 'The personal ZIP contains an unsupported entry or exceeds its depth limit.'}
            if($entry.Length -gt 268435456 -or $entry.Length -gt ($entry.CompressedLength*200L+1048576L)){throw 'The personal ZIP exceeds the per-file or compression-ratio limit.'}
        }
        if($entry.Length -lt 0 -or $entry.Length -gt 137438953472 -or $entry.Length -gt ($entry.CompressedLength*1000L+1048576L)){throw 'The ZIP exceeds its size or expansion-ratio budget.'}
        $total+=[UInt64]$entry.Length
        if($total -gt 137438953472){throw 'The ZIP expands beyond 128 GiB.'}
        if($personal -and $total -gt 2147483648){throw 'The personal ZIP expands beyond 2 GiB.'}
        $entries.Add($entry)
    }
    if($Action -eq 'PersonalCatalog'){
        $result=@($entries | ForEach-Object {@{name=$_.FullName;size=[UInt64]$_.Length}})
        [IO.File]::WriteAllText($Report,(ConvertTo-Json -InputObject $result -Depth 3 -Compress),$utf8)
    } elseif($Action -eq 'Inspect'){
        $manifestEntry=$zip.GetEntry('manifest.json')
        if($null -eq $manifestEntry -or $manifestEntry.Length -gt 16777216){throw 'A portable ZIP needs a bounded manifest.json.'}
        $stream=$manifestEntry.Open();$memory=New-Object IO.MemoryStream;$buffer=New-Object byte[] 65536;$actual=[UInt64]0
        $limit=[Math]::Min([Int64]$manifestEntry.Length,16777216L)
        try {
            while(($count=$stream.Read($buffer,0,[int][Math]::Min($buffer.Length,$limit-$actual+1))) -gt 0){
                $actual+=[UInt64]$count
                if($actual -gt $limit){throw 'The ZIP manifest expanded beyond its declared or 16 MiB size limit.'}
                $memory.Write($buffer,0,$count)
            }
            if($actual -ne $manifestEntry.Length){throw 'The ZIP manifest is shorter than its declared size.'}
            $manifest=$utf8.GetString($memory.GetBuffer(),0,[int]$actual)
        } finally {$memory.Dispose();$stream.Dispose()}
        $result=New-Object 'System.Collections.Generic.List[object]'
        foreach($entry in $entries){
            $sha=[Security.Cryptography.SHA256]::Create();$stream=$entry.Open();$buffer=New-Object byte[] 65536;$actual=[UInt64]0
            try {
                while(($count=$stream.Read($buffer,0,$buffer.Length)) -gt 0){$actual+=[UInt64]$count;if($actual -gt $entry.Length){throw 'A ZIP entry expanded beyond its declared size.'};$sha.TransformBlock($buffer,0,$count,$buffer,0)|Out-Null}
                if($actual -ne $entry.Length){throw 'A ZIP entry is shorter than its declared size.'}
                $sha.TransformFinalBlock((New-Object byte[] 0),0,0)|Out-Null
                $digest=[BitConverter]::ToString($sha.Hash).Replace('-','').ToLowerInvariant()
            }finally {$stream.Dispose();$sha.Dispose()}
            $result.Add(@{name=$entry.FullName;size=[UInt64]$entry.Length;sha256=$digest})
        }
        [IO.File]::WriteAllText($Report,(@{manifest=$manifest;entries=@($result.ToArray())}|ConvertTo-Json -Depth 5 -Compress),$utf8)
    } else {
        $selected=ConvertFrom-Json ([IO.File]::ReadAllText($Names,$utf8))
        $root=[IO.Path]::GetFullPath($Destination).TrimEnd('\')+'\'
        $privateRoot=[IO.Path]::GetFullPath($PartRoot).TrimEnd('\')+'\'
        if($privateRoot.StartsWith($root,[StringComparison]::OrdinalIgnoreCase) -or ([IO.File]::GetAttributes($privateRoot.TrimEnd('\')) -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'ZIP partials need a private directory outside the extraction namespace.'}
        $partIndex=0
        $copied=[UInt64]0
        foreach($name in $selected){
            if(!$known.Contains([string]$name)){throw 'A selected ZIP entry is unavailable.'}
            $entry=$zip.GetEntry([string]$name)
            if($personal){$copied+=[UInt64]$entry.Length;if($copied -gt 268435456){throw 'The selected personal files exceed 256 MiB.'}}
            $target=[IO.Path]::GetFullPath([IO.Path]::Combine($root,([string]$name).Replace('/','\')))
            if(!$target.StartsWith($root,[StringComparison]::OrdinalIgnoreCase)){throw 'A ZIP path escaped the owned extraction folder.'}
            [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))|Out-Null
            if([IO.File]::Exists($target)){continue}
            $part=[IO.Path]::Combine($privateRoot,('{0:D5}.part' -f $partIndex));$partIndex++
            $parent=[IO.Path]::GetDirectoryName($target)
            while($parent.Length -ge $root.TrimEnd('\').Length){if(([IO.File]::GetAttributes($parent) -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Extraction cannot follow a reparse point.'};$parent=[IO.Path]::GetDirectoryName($parent);if(!$parent){break}}
            $stream=$entry.Open();$output=[IO.File]::Open($part,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
            try {$buffer=New-Object byte[] 65536;$actual=[UInt64]0;while(($count=$stream.Read($buffer,0,$buffer.Length)) -gt 0){$actual+=[UInt64]$count;if($actual -gt $entry.Length){throw 'A ZIP entry expanded beyond its declared size.'};$output.Write($buffer,0,$count)};if($actual -ne $entry.Length){throw 'A ZIP entry is shorter than its declared size.'};$output.Flush($true)}finally {$output.Dispose();$stream.Dispose()}
            [IO.File]::Move($part,$target)
        }
        [IO.File]::WriteAllText($Report,'true',$utf8)
    }
} finally {$zip.Dispose()}
Write-Output ('__FILE__'+$Report)
