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
# Big downloads can arrive in parts (name.part1, name.part2, ...): put each back together,
# check it against latest.json, and add the copies under the names that never change.
$site = Join-Path $here "upload\ingle"
foreach ($first in @(Get-ChildItem -Path $site -Filter "*.part1")) {
    $whole = Join-Path $site ($first.Name -replace '\.part1$', '')
    $out = [System.IO.File]::Create($whole)
    try {
        $i = 1
        while (Test-Path "$whole.part$i") {
            $bytes = [System.IO.File]::ReadAllBytes("$whole.part$i")
            $out.Write($bytes, 0, $bytes.Length)
            $i++
        }
    } finally {
        $out.Close()
    }
    Get-ChildItem -Path $site -Filter "$(Split-Path -Leaf $whole).part*" | Remove-Item
}
$manifest = (Get-Content -Raw (Join-Path $site "latest.json") | ConvertFrom-Json).manifest | ConvertFrom-Json
$downloads = @(
    @{ Item = $manifest.windows; Stable = "Ingle-Setup.exe" },
    @{ Item = $manifest.linux_deb; Stable = "ingle_amd64.deb" }
)
foreach ($d in $downloads) {
    if ($null -eq $d.Item) { continue }
    $file = Join-Path $site (($d.Item.url -split '/')[-1])
    if (-not (Test-Path $file)) { throw "$file is missing." }
    if ((Get-FileHash -Algorithm SHA256 $file).Hash.ToLower() -ne $d.Item.sha256) {
        throw "$file isn't the file latest.json describes (a part missing or damaged?)."
    }
    Copy-Item $file (Join-Path $site $d.Stable) -Force
}
scp -r "$here\upload" "${server}:$folder"
if ($LASTEXITCODE -ne 0) { throw "The upload didn't work." }
ssh $server "bash ~/$folder/install-site.sh"
if ($LASTEXITCODE -ne 0) { throw "Putting the files on the website didn't work." }
$f = Invoke-RestMethod -Uri "https://firelitgames.com/ingle/latest.json" -Headers @{ "Cache-Control" = "no-cache" }
$m = $f.manifest | ConvertFrom-Json
Write-Host ""
Write-Host "Done: firelitgames.com/ingle now offers Ingle $($m.version). Running copies of Ingle will offer the update within a few hours (or right away when reopened)."
