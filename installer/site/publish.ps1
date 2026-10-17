# Upload this Pyr release to pyr.gg (your Hostinger VPS).
# Run publish.cmd. It asks for the VPS root password twice (once to upload, once to install).
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$server = "root@2.24.121.231"
Write-Host "Uploading Pyr to pyr.gg..."
Write-Host "Type your VPS root password when asked (the letters don't show while you type)."
Write-Host "If it asks 'Are you sure you want to continue connecting', type yes."
# Into a new folder with a random name in root's own home folder (not /tmp, where another
# account on the VPS could put its own files in the way before they run as root).
$folder = "pyr-release-" + [guid]::NewGuid().ToString("N").Substring(0, 16)
# Big downloads can arrive in parts (name.part1, name.part2, ...): put each back together,
# check it against latest.json, and add the copies under the names that never change.
$site = Join-Path $here "upload\pyr"
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
# (And under the names they had before Pyr was called Pyr, for links to them out there.)
$downloads = @(
    @{ Item = $manifest.windows; Stable = "Pyr-Setup.exe" },
    @{ Item = $manifest.windows; Stable = "Ingle-Setup.exe" },
    @{ Item = $manifest.linux_deb; Stable = "pyr_amd64.deb" },
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
$f = Invoke-RestMethod -Uri "https://pyr.gg/latest.json" -Headers @{ "Cache-Control" = "no-cache" }
$m = $f.manifest | ConvertFrom-Json
# Copies from before the rename look here (the same folder).
$o = Invoke-RestMethod -Uri "https://firelitgames.com/ingle/latest.json" -Headers @{ "Cache-Control" = "no-cache" }
$om = $o.manifest | ConvertFrom-Json
Write-Host ""
Write-Host "Done: pyr.gg now offers Pyr $($m.version) (and firelitgames.com/ingle offers $($om.version) to copies from before the rename). Running copies will offer the update within a few hours (or right away when reopened)."
