$ErrorActionPreference='Stop'
$taskRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$env:TEMP=Join-Path $taskRoot '.build\tmp'
$env:TMP=$env:TEMP
$download=Join-Path $taskRoot '.build\downloads'
New-Item -ItemType Directory -Force $download,$env:TEMP | Out-Null
if (!(Test-Path (Join-Path $download 'android-repository.xml'))) { Invoke-WebRequest 'https://dl.google.com/android/repository/repository2-3.xml' -OutFile (Join-Path $download 'android-repository.xml') }
$sdk=Join-Path $taskRoot '.build\android-sdk'
New-Item -ItemType Directory -Force $sdk | Out-Null
[xml]$catalog=Get-Content (Join-Path $download 'android-repository.xml') -Raw
foreach ($package in @('platforms;android-36','build-tools;36.0.0','ndk;29.0.14206865')) {
 $entry=$catalog.'sdk-repository'.remotePackage | Where-Object path -EQ $package
 $archive=$entry.archives.archive | Where-Object { !$_. 'host-os' -or $_.'host-os' -eq 'windows' } | Select-Object -First 1
 $name=[string]$archive.complete.url
 $file=Join-Path $download $name
 $expected=$archive.complete.checksum.InnerText.Trim()
 if (!(Test-Path $file) -or (Get-FileHash $file -Algorithm SHA1).Hash.ToLowerInvariant() -ne $expected.ToLowerInvariant()) {
  Write-Output "Downloading $name (2 MB/s cap)"
  & curl.exe --fail --location --retry 3 --limit-rate 2M --silent --show-error --output $file "https://dl.google.com/android/repository/$name"
  if ($LASTEXITCODE) { throw "Download failed: $name" }
 }
 if ($expected -and (Get-FileHash $file -Algorithm SHA1).Hash.ToLowerInvariant() -ne $expected.ToLowerInvariant()) { throw "Checksum failed: $name" }
 $target=Join-Path $sdk ($package.Replace(';','\'))
 if (Test-Path $target) { Write-Output "Ready: $package"; continue }
 $extract=Join-Path $download ($name + '.extract')
 New-Item -ItemType Directory -Force $extract | Out-Null
 & tar.exe -xf $file -C $extract
 if ($LASTEXITCODE) { throw "Extraction failed: $name" }
 $source=Get-ChildItem $extract -Directory | Select-Object -First 1
 $target=Join-Path $sdk ($package.Replace(';','\'))
 New-Item -ItemType Directory -Force (Split-Path $target) | Out-Null
 if (!(Test-Path $target)) { Move-Item -LiteralPath $source.FullName -Destination $target }
 Write-Output "Ready: $package"
}
$hostSysroot=(& rustc --print sysroot).Trim()
if ($LASTEXITCODE) { throw 'Cannot locate the active Rust toolchain' }
$manifest=Join-Path $hostSysroot 'lib\rustlib\multirust-channel-manifest.toml'
$content=Get-Content $manifest -Raw
$match=[regex]::Match($content,'(?s)\[pkg\.rust-std\.target\.aarch64-linux-android\].*?url = "([^"]+)".*?hash = "([^"]+)"')
if (!$match.Success) { throw 'Rust Android target not in toolchain manifest' }
$url=$match.Groups[1].Value
$file=Join-Path $download 'rust-std-android.tar.gz'
if (!(Test-Path $file) -or (Get-FileHash $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $match.Groups[2].Value) {
 & curl.exe --fail --location --retry 3 --limit-rate 2M --silent --show-error --output $file $url
 if ($LASTEXITCODE) { throw 'Rust target download failed' }
}
if ((Get-FileHash $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $match.Groups[2].Value) { throw 'Rust target checksum failed' }
& tar.exe -xf $file -C $download
if ($LASTEXITCODE) { throw 'Rust target extraction failed' }
$root=Get-Item (Join-Path $download ([IO.Path]::GetFileName($url).Replace('.tar.gz','')))
$sysroot=Join-Path $taskRoot '.build\android-sysroot'
New-Item -ItemType Directory -Force $sysroot | Out-Null
# Different rustc releases cannot share target .rlib files. Preserve old target,
# then copy a clean library tree rather than leaving incompatible files mixed in.
$oldTarget=[IO.Path]::GetFullPath((Join-Path $sysroot 'lib/rustlib/aarch64-linux-android'))
if (!$oldTarget.StartsWith($taskRoot + '\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Target directory escaped project' }
if (Test-Path -LiteralPath $oldTarget) {
 $saved=Join-Path $taskRoot ('.build/android-target-backup-' + (Get-Date -Format 'yyyyMMddHHmmss'))
 Move-Item -LiteralPath $oldTarget -Destination $saved
}
Copy-Item -LiteralPath (Join-Path $root.FullName 'rust-std-aarch64-linux-android\lib') -Destination $sysroot -Recurse -Force
& rustc --version | Set-Content (Join-Path $sysroot 'rustc-version.txt')
Write-Output 'Android SDK, NDK and project-local Rust target ready.'




