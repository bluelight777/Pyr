# Upload this Ingle release to firelitgames.com/ingle (your Hostinger VPS).
# Run publish.cmd. It asks for the VPS root password twice (once to upload, once to install).
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$server = "root@2.24.121.231"
Write-Host "Uploading Ingle to firelitgames.com..."
Write-Host "Type your VPS root password when asked (the letters don't show while you type)."
Write-Host "If it asks 'Are you sure you want to continue connecting', type yes."
# Into a new folder with a random name in root's own home folder (not /tmp, where another
# account on the VPS could put its own files in the way before they run as root).
$folder = "ingle-release-" + [guid]::NewGuid().ToString("N").Substring(0, 16)
scp -r "$here\upload" "${server}:$folder"
if ($LASTEXITCODE -ne 0) { throw "The upload didn't work." }
ssh $server "bash ~/$folder/install-site.sh"
if ($LASTEXITCODE -ne 0) { throw "Putting the files on the website didn't work." }
$f = Invoke-RestMethod -Uri "https://firelitgames.com/ingle/latest.json" -Headers @{ "Cache-Control" = "no-cache" }
$m = $f.manifest | ConvertFrom-Json
Write-Host ""
Write-Host "Done: firelitgames.com/ingle now offers Ingle $($m.version). Running copies of Ingle will offer the update within a few hours (or right away when reopened)."
