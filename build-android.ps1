param([ValidateSet('check','clippy','release')][string]$Action='release')
$ErrorActionPreference='Stop'

$taskRoot = $PSScriptRoot
$nativeCmake='C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake'
if (Test-Path (Join-Path $nativeCmake 'CMake/bin/cmake.exe')) {
 $env:CMAKE=Join-Path $nativeCmake 'CMake/bin/cmake.exe'
 $env:PATH="$(Join-Path $nativeCmake 'Ninja');$(Join-Path $nativeCmake 'CMake/bin');$env:PATH"
}
$env:CMAKE_BUILD_PARALLEL_LEVEL='1'
$env:CARGO_HOME=Join-Path $taskRoot '.build\cargo-home'
$env:CARGO_TARGET_DIR=Join-Path $taskRoot '.build\target'
$env:TEMP=Join-Path $taskRoot '.build\tmp'
$env:TMP=$env:TEMP
$env:RTK_TEE_DIR=Join-Path $taskRoot '.build\rtk\tee'
$env:RTK_DB_PATH=Join-Path $taskRoot '.build\rtk\history.db'
$env:RTK_TELEMETRY_DISABLED='1'
$env:CARGO_BUILD_JOBS='1'
$env:RAYON_NUM_THREADS='1'
$env:ANDROID_HOME=Join-Path $taskRoot '.build\android-sdk'
$env:ANDROID_SDK_ROOT=$env:ANDROID_HOME
$env:ANDROID_USER_HOME=Join-Path $taskRoot '.build\android-user'
$env:ANDROID_NDK_HOME=Join-Path $env:ANDROID_HOME 'ndk\29.0.14206865'
$env:JAVA_HOME=Split-Path (Split-Path (Get-Command javac.exe).Source)
$env:JAVA_TOOL_OPTIONS="-Xmx256m -Djava.io.tmpdir=$env:TEMP -Duser.home=$env:ANDROID_USER_HOME"
$tools=Join-Path $env:ANDROID_NDK_HOME 'toolchains\llvm\prebuilt\windows-x86_64\bin'
$sysroot=Join-Path $taskRoot '.build\android-sysroot'
if (!(Test-Path $tools) -or !(Test-Path (Join-Path $sysroot 'lib\rustlib\aarch64-linux-android'))) {
 throw 'Android tools missing. Run .\tools\setup-android.ps1 first.'
}
$versionFile=Join-Path $sysroot 'rustc-version.txt'
if ((Test-Path $versionFile) -and ((Get-Content $versionFile -Raw).Trim() -ne (& rustc --version).Trim())) {
 throw 'Rust changed since Android setup. Run .\tools\setup-android.ps1 to refresh the project-local target.'
}
$env:ANDROID_PLATFORM='android-26'
$env:ANDROID_STL='c++_static'
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=Join-Path $tools 'aarch64-linux-android26-clang.cmd'
$env:CC_aarch64_linux_android=$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER
$env:CXX_aarch64_linux_android=Join-Path $tools 'aarch64-linux-android26-clang++.cmd'
$env:AR_aarch64_linux_android=Join-Path $tools 'llvm-ar.exe'
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="--sysroot=$sysroot -C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
New-Item -ItemType Directory -Force $env:ANDROID_USER_HOME,$env:TEMP,$env:RTK_TEE_DIR | Out-Null
function Invoke-Checked([string]$Program,[string[]]$Arguments) {
 & $Program @Arguments
 if ($LASTEXITCODE -ne 0) { throw "$Program failed ($LASTEXITCODE)" }
}
Push-Location $taskRoot
try {
 $args=@($Action,'-p','p2pshare-android','--target','aarch64-linux-android','--locked','-j','1')
 if ($Action -eq 'release') { $args[0]='build'; $args+='--release' }
 if ($Action -eq 'clippy') { $args+=@('--','-D','warnings') }
 Invoke-Checked 'cargo' $args
 if ($Action -ne 'release') { return }
 $buildTools=Join-Path $env:ANDROID_HOME 'build-tools\36.0.0'
 $artifacts=Join-Path $taskRoot '.build\android-apk'
 New-Item -ItemType Directory -Force $artifacts | Out-Null
 $unsigned=Join-Path $artifacts 'unsigned.apk'
 $aligned=Join-Path $artifacts 'aligned.apk'
 $releaseDir=Join-Path $taskRoot 'releases/android'
 New-Item -ItemType Directory -Force $releaseDir | Out-Null
 $result=Join-Path $releaseDir 'P2PShare-android-arm64.apk'
 $resources=Join-Path $artifacts 'resources.zip'
 Invoke-Checked (Join-Path $buildTools 'aapt2.exe') @('compile','--dir',(Join-Path $taskRoot 'apps\android\res'),'-o',$resources)
 Invoke-Checked (Join-Path $buildTools 'aapt2.exe') @('link','-o',$unsigned,'-I',(Join-Path $env:ANDROID_HOME 'platforms\android-36\android.jar'),'--manifest',(Join-Path $taskRoot 'apps\android\AndroidManifest.xml'),'-A',(Join-Path $taskRoot 'apps\android\assets'),$resources)
 $classes=Join-Path $artifacts 'classes'
 $dex=Join-Path $artifacts 'dex'
 New-Item -ItemType Directory -Force $classes,$dex | Out-Null
 $androidJar=Join-Path $env:ANDROID_HOME 'platforms\android-36\android.jar'
 # Compile only classes from current sources; remove obsolete capture classes.
 Get-ChildItem -LiteralPath $classes -Filter '*.class' -Recurse | Remove-Item -Force
 $javaSources=@(Get-ChildItem -LiteralPath (Join-Path $taskRoot 'apps\android\java') -Filter '*.java' -Recurse | ForEach-Object FullName)
 Invoke-Checked (Join-Path $env:JAVA_HOME 'bin\javac.exe') (@('-encoding','UTF-8','-source','8','-target','8','-classpath',$androidJar,'-d',$classes)+$javaSources)
 $classFiles=@(Get-ChildItem -LiteralPath $classes -Filter '*.class' -Recurse | ForEach-Object FullName)
 Invoke-Checked (Join-Path $env:JAVA_HOME 'bin\java.exe') (@('-cp',(Join-Path $buildTools 'lib\d8.jar'),'com.android.tools.r8.D8','--min-api','26','--lib',$androidJar,'--output',$dex)+$classFiles)
 Add-Type -AssemblyName System.IO.Compression
 $stream=[IO.File]::Open($unsigned,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite)
 $zip=[IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Update)
 try {
  # Add nested assets with APK-standard forward slashes; Windows aapt2 emits backslashes.
  $catalogEntry=$zip.CreateEntry('assets/i18n/pt-BR.json',[IO.Compression.CompressionLevel]::Optimal)
  $catalogDest=$catalogEntry.Open(); $catalogSource=[IO.File]::OpenRead((Join-Path $taskRoot 'crates/media/src/i18n/pt_BR.json'))
  try { $catalogSource.CopyTo($catalogDest) } finally { $catalogSource.Dispose(); $catalogDest.Dispose() }
  $dexEntry=$zip.CreateEntry('classes.dex',[IO.Compression.CompressionLevel]::Optimal)
  $dexDest=$dexEntry.Open(); $dexSource=[IO.File]::OpenRead((Join-Path $dex 'classes.dex'))
  try { $dexSource.CopyTo($dexDest) } finally { $dexSource.Dispose(); $dexDest.Dispose() }
  $entry=$zip.CreateEntry('lib/arm64-v8a/libp2pshare_android.so',[IO.Compression.CompressionLevel]::NoCompression)
  $dest=$entry.Open()
  $source=[IO.File]::OpenRead((Join-Path $env:CARGO_TARGET_DIR 'aarch64-linux-android\release\libp2pshare_android.so'))
  try { $source.CopyTo($dest) } finally { $source.Dispose(); $dest.Dispose() }
 } finally { $zip.Dispose(); $stream.Dispose() }
 Invoke-Checked (Join-Path $buildTools 'zipalign.exe') @('-P','16','-f','4',$unsigned,$aligned)
 $key=Join-Path $artifacts 'test-signing.p12'
 if (!(Test-Path $key)) {
  Invoke-Checked (Join-Path $env:JAVA_HOME 'bin\keytool.exe') @('-genkeypair','-keystore',$key,'-storetype','PKCS12','-storepass','p2pshare-test','-alias','p2pshare-test','-keyalg','RSA','-keysize','2048','-validity','3650','-dname','CN=P2PShare Local Test')
 }
 Invoke-Checked (Join-Path $env:JAVA_HOME 'bin\java.exe') @('-jar',(Join-Path $buildTools 'lib\apksigner.jar'),'sign','--v4-signing-enabled','false','--ks',$key,'--ks-pass','pass:p2pshare-test','--ks-key-alias','p2pshare-test','--out',$result,$aligned)
 Invoke-Checked (Join-Path $env:JAVA_HOME 'bin\java.exe') @('-jar',(Join-Path $buildTools 'lib\apksigner.jar'),'verify','--verbose',$result)
 Invoke-Checked (Join-Path $buildTools 'zipalign.exe') @('-c','-P','16','4',$result)
 Write-Output "APK ready: $result"
} finally { Pop-Location }
