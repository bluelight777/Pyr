# Put a new copy of pyr.gg's page on the website, without a new Pyr release (the downloads, the
# release files, the Library and the server stay as they are). Run publish-page.cmd. It asks for
# the VPS root password twice (once to upload, once to put it in place).
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$server = "root@2.24.121.231"
Write-Host "Uploading pyr.gg's page..."
Write-Host "Type your VPS root password when asked (the letters don't show while you type)."
Write-Host "If it asks 'Are you sure you want to continue connecting', type yes."
# Into a new folder with a random name in root's own home folder (as publish.ps1 does).
$folder = "pyr-page-" + [guid]::NewGuid().ToString("N").Substring(0, 16)
if (-not (Test-Path (Join-Path $here "upload\pyr\index.html"))) { throw "upload\pyr\index.html is missing." }
scp -r "$here\upload" "${server}:$folder"
if ($LASTEXITCODE -ne 0) { throw "The upload didn't work." }
ssh $server "bash ~/$folder/install-page.sh"
if ($LASTEXITCODE -ne 0) { throw "Putting the page on the website didn't work." }
Write-Host ""
Write-Host "Done: https://pyr.gg has the new page (reload it to see it)."
