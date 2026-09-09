param(
    [switch]$Install,
    [switch]$Restore,
    [string]$Stream = ""
)

$ErrorActionPreference = "Stop"

$Width = 320
$Height = 172
$FrameBytes = $Width * $Height * 2

$FileStartInStream = 25
$DlxPrefixBytes = 8
$HeaderBytes = 452
$Frame1Start = $FileStartInStream + $DlxPrefixBytes + $HeaderBytes
$Frame2Start = $Frame1Start + $FrameBytes
$TrailerStart = $Frame2Start + $FrameBytes
$ExpectedStreamLen = $TrailerStart + 4

$KnownGoodSha256 = "a4be0a5df3e785a7cc4a13969d1b28b72312eda684627e487859ef11bdfae7d0"

$Here = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($Stream)) {
    $Stream = Join-Path $Here "extracted\session_01_bulk_stream.bin"
}

$Backup = Join-Path (Split-Path $Stream -Parent) "session_01_bulk_stream.redgreen-backup.bin"
$Generated = Join-Path (Split-Path $Stream -Parent) "session_01_bulk_stream.diagnostic.bin"

function HexBytes([string]$s) {
    $parts = $s -split '\s+'
    [byte[]]($parts | ForEach-Object { [Convert]::ToByte($_,16) })
}

function Sha256([string]$path) {
    (Get-FileHash -Algorithm SHA256 $path).Hash.ToLowerInvariant()
}

function Assert-Layout([byte[]]$data) {
    if ($data.Length -ne $ExpectedStreamLen) {
        throw "Unexpected stream size: $($data.Length); expected $ExpectedStreamLen"
    }

    $env = HexBytes "88 00 00 00 03 5d e1 00"
    for ($i=0; $i -lt 8; $i++) {
        if ($data[$i] -ne $env[$i]) { throw "Outer 0x88 envelope mismatch." }
    }

    $dlx = HexBytes "00 44 4c 58 fc ff 00 00"
    for ($i=0; $i -lt 8; $i++) {
        if ($data[25+$i] -ne $dlx[$i]) { throw "DLX prefix mismatch." }
    }

    $trail = HexBytes "fc ff 00 00"
    for ($i=0; $i -lt 4; $i++) {
        if ($data[$data.Length-4+$i] -ne $trail[$i]) { throw "DLX trailer mismatch." }
    }
}

function Rgb565Be([int]$r,[int]$g,[int]$b) {
    $v = (($r -band 0xF8) -shl 8) -bor (($g -band 0xFC) -shl 3) -bor ($b -shr 3)
    return [byte[]]@((($v -shr 8) -band 0xFF), ($v -band 0xFF))
}

if ($Restore) {
    if (-not (Test-Path $Backup)) { throw "No backup found: $Backup" }
    $restored = [IO.File]::ReadAllBytes($Backup)
    Assert-Layout $restored
    [IO.File]::WriteAllBytes($Stream, $restored)
    Write-Host "Restored: $Stream"
    Write-Host "SHA-256 : $(Sha256 $Stream)"
    exit 0
}

if (-not (Test-Path $Stream)) { throw "Stream not found: $Stream" }

$current = [IO.File]::ReadAllBytes($Stream)
Assert-Layout $current

if (Test-Path $Backup) {
    $pristine = [IO.File]::ReadAllBytes($Backup)
    Assert-Layout $pristine
} else {
    $hash = Sha256 $Stream
    if ($hash -ne $KnownGoodSha256) {
        throw "Current stream is not the exact captured red/green reference and no backup exists. Current SHA-256: $hash"
    }
    Copy-Item $Stream $Backup
    Write-Host "Created permanent known-good backup: $Backup"
    $pristine = [IO.File]::ReadAllBytes($Backup)
}

[byte[]]$frame = New-Object byte[] $FrameBytes

$top = @(
    @(255,0,0),
    @(0,255,0),
    @(0,0,255),
    @(255,255,255)
)
$bottom = @(
    @(0,0,0),
    @(128,128,128),
    @(255,255,0),
    @(0,255,255)
)

$p = 0
for ($y=0; $y -lt $Height; $y++) {
    $palette = if ($y -lt [int]($Height/2)) { $top } else { $bottom }
    for ($x=0; $x -lt $Width; $x++) {
        $idx = [Math]::Min(3, [int][Math]::Floor(($x * 4.0) / $Width))
        $c = $palette[$idx]
        $px = Rgb565Be $c[0] $c[1] $c[2]
        $frame[$p] = $px[0]
        $frame[$p+1] = $px[1]
        $p += 2
    }
}

[byte[]]$modified = New-Object byte[] $pristine.Length
[Array]::Copy($pristine, $modified, $pristine.Length)
[Array]::Copy($frame, 0, $modified, $Frame1Start, $FrameBytes)
[Array]::Copy($frame, 0, $modified, $Frame2Start, $FrameBytes)
Assert-Layout $modified

[IO.File]::WriteAllBytes($Generated, $modified)

Write-Host "Diagnostic stream generated."
Write-Host "Output     : $Generated"
Write-Host "Stream size: $($modified.Length) bytes"
Write-Host "Frame 1    : offset $Frame1Start, $FrameBytes bytes"
Write-Host "Frame 2    : offset $Frame2Start, $FrameBytes bytes"
Write-Host "Pattern    : TOP red | green | blue | white"
Write-Host "             BOTTOM black | gray | yellow | cyan"
Write-Host "Both frames are identical, so playback should LOOK STATIC."
Write-Host "SHA-256    : $((Get-FileHash -Algorithm SHA256 $Generated).Hash.ToLowerInvariant())"

if ($Install) {
    [IO.File]::WriteAllBytes($Stream, $modified)
    Write-Host ""
    Write-Host "INSTALLED into: $Stream"
    Write-Host "After the display test, restore with:"
    Write-Host "  .\tiga_make_diagnostic_stream.ps1 -Restore"
} else {
    Write-Host ""
    Write-Host "DRY GENERATION ONLY: active replay stream was NOT replaced."
    Write-Host "Run again with -Install when ready."
}
