param([ValidateSet('Install','Check','Test','Login','Whoami','Deploy','Dev')][string]$Action='Check')
$ErrorActionPreference='Stop'
$taskRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$toolRoot=Join-Path $taskRoot '.build/worker-tools'
$env:TEMP=Join-Path $taskRoot '.build/tmp'
$env:TMP=$env:TEMP
$env:npm_config_cache=Join-Path $taskRoot '.build/worker-npm'
$env:npm_config_update_notifier='false'
$env:XDG_CONFIG_HOME=Join-Path $toolRoot 'config'
$env:APPDATA=Join-Path $toolRoot 'appdata'
$env:LOCALAPPDATA=Join-Path $toolRoot 'localappdata'
$env:WRANGLER_LOG_PATH=Join-Path $toolRoot 'logs'
$env:WRANGLER_SEND_METRICS='false'
$env:NODE_OPTIONS='--max-old-space-size=384'
New-Item -ItemType Directory -Force -Path $env:TEMP,$env:npm_config_cache,$env:XDG_CONFIG_HOME,$env:APPDATA,$env:LOCALAPPDATA,$env:WRANGLER_LOG_PATH | Out-Null
Push-Location $PSScriptRoot
try {
  if($Action -eq 'Install') {
    if(Test-Path package-lock.json) {& npm.cmd ci --no-audit --no-fund} else {& npm.cmd install --no-audit --no-fund}
  } elseif($Action -eq 'Test') {
    & npm.cmd test
  } else {
    $cli=Join-Path $PSScriptRoot 'node_modules/wrangler/bin/wrangler.js'
    if(!(Test-Path $cli)) {throw 'Run ./worker.ps1 -Action Install first.'}
    switch($Action) {
      'Check' {& node.exe $cli deploy --dry-run --outdir (Join-Path $toolRoot 'dry-run')}
      'Login' {& node.exe $cli login}
      'Whoami' {& node.exe $cli whoami}
      'Deploy' {& node.exe $cli deploy}
      'Dev' {& node.exe $cli dev --local}
    }
  }
  if($LASTEXITCODE -ne 0) {throw "Worker action $Action failed ($LASTEXITCODE)."}
} finally {Pop-Location}
