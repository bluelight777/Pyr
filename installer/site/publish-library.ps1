# Put a new Pyr Library (its signed list and the plugins' files) on pyr.gg,
# without a new Pyr release. Run publish-library.cmd. It asks for the VPS root password twice
# (once to upload, once to put it in place).
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$server = "root@2.24.121.231"
Write-Host "Uploading the Pyr Library to pyr.gg..."
Write-Host "Type your VPS root password when asked (the letters don't show while you type)."
Write-Host "If it asks 'Are you sure you want to continue connecting', type yes."
# Into a new folder with a random name in root's own home folder (as publish.ps1 does).
$folder = "pyr-library-" + [guid]::NewGuid().ToString("N").Substring(0, 16)
if (-not (Test-Path (Join-Path $here "upload\library.tar"))) { throw "upload\library.tar is missing." }
scp -r "$here\upload" "${server}:$folder"
if ($LASTEXITCODE -ne 0) { throw "The upload didn't work." }
ssh $server "bash ~/$folder/install-library.sh"
if ($LASTEXITCODE -ne 0) { throw "Putting the library on the website didn't work." }
$lib = Invoke-RestMethod -Uri "https://pyr.gg/library/index2.json" -Headers @{ "Cache-Control" = "no-cache" }
$index = $lib.index | ConvertFrom-Json
$names = ($index.plugins | ForEach-Object { $_.name }) -join ", "
Write-Host ""
Write-Host "Done: the Pyr Library now lists $names. Pyr shows it next time its Plugins page is opened (or Refresh is clicked)."
