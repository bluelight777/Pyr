# Put a newer props feed maker (pyr-props) on the VPS, without a new Pyr release.
# Run publish-props.cmd. It asks for the VPS root password twice (once to upload, once to
# set it up). The feed's own key and the sports sites' keys stay where they are on the VPS:
# nothing secret is in this folder.
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$server = "root@2.24.121.231"
Write-Host "Uploading the props feed's maker to the VPS..."
Write-Host "Type your VPS root password when asked (the letters don't show while you type)."
Write-Host "If it asks 'Are you sure you want to continue connecting', type yes."
if (-not (Test-Path (Join-Path $here "upload\setup-props.sh"))) { throw "upload\setup-props.sh is missing." }
if (-not (Test-Path (Join-Path $here "upload\props\feed-cert.json"))) { throw "upload\props\feed-cert.json is missing." }
# Into a new folder with a random name in root's own home folder (as publish.ps1 does).
$folder = "pyr-props-" + [guid]::NewGuid().ToString("N").Substring(0, 16)
scp -r "$here\upload" "${server}:$folder"
if ($LASTEXITCODE -ne 0) { throw "The upload didn't work." }
# (The folder is taken away again whether the set-up went well or not.)
ssh $server "bash ~/$folder/setup-props.sh; code=`$?; rm -rf ~/$folder; exit `$code"
if ($LASTEXITCODE -ne 0) { throw "Setting the props feed's maker up didn't work (what it said is above)." }
$status = Invoke-RestMethod -Uri "https://pyr.gg/props/status.json" -Headers @{ "Cache-Control" = "no-cache" }
Write-Host ""
Write-Host "Done: the props feed is made by pyr-props $($status.version). What each source reads is above, and at https://pyr.gg/props/status.json."
