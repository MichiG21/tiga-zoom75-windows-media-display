param(
    [switch]$Watch,
    [int]$IntervalMs = 1000
)

$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Runtime.WindowsRuntime

[void][Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media.Control, ContentType=WindowsRuntime]
[void][Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties, Windows.Media.Control, ContentType=WindowsRuntime]

$asTaskGeneric = (
    [System.WindowsRuntimeSystemExtensions].GetMethods() |
    Where-Object {
        $_.Name -eq "AsTask" -and
        $_.IsGenericMethod -and
        $_.GetParameters().Count -eq 1 -and
        $_.GetParameters()[0].ParameterType.Name -eq "IAsyncOperation`1"
    } |
    Select-Object -First 1
)

if (-not $asTaskGeneric) {
    throw "Could not find the WinRT IAsyncOperation -> Task bridge."
}

function Await-WinRT {
    param(
        [Parameter(Mandatory=$true)] $AsyncOperation,
        [Parameter(Mandatory=$true)] [Type] $ResultType
    )

    $asTask = $script:asTaskGeneric.MakeGenericMethod($ResultType)
    $task = $asTask.Invoke($null, @($AsyncOperation))
    $task.Wait()
    return $task.Result
}

function Get-CurrentWindowsMedia {
    $managerType = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media.Control, ContentType=WindowsRuntime]
    $propsType = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties, Windows.Media.Control, ContentType=WindowsRuntime]

    $manager = Await-WinRT ($managerType::RequestAsync()) $managerType
    $session = $manager.GetCurrentSession()

    if ($null -eq $session) {
        return $null
    }

    $props = Await-WinRT ($session.TryGetMediaPropertiesAsync()) $propsType
    if ($null -eq $props) {
        return $null
    }

    $playback = $session.GetPlaybackInfo()

    $thumbnailPresent = $false
    try {
        $thumbnailPresent = ($null -ne $props.Thumbnail)
    }
    catch {
        $thumbnailPresent = $false
    }

    [pscustomobject]@{
        SourceApp       = [string]$session.SourceAppUserModelId
        PlaybackStatus  = [string]$playback.PlaybackStatus
        Title           = [string]$props.Title
        Artist          = [string]$props.Artist
        AlbumArtist     = [string]$props.AlbumArtist
        Album           = [string]$props.AlbumTitle
        Subtitle        = [string]$props.Subtitle
        TrackNumber     = [int]$props.TrackNumber
        Thumbnail       = $thumbnailPresent
    }
}

function Show-Media {
    param($Media)

    if ($null -eq $Media) {
        Write-Host ""
        Write-Host "No current Windows media session."
        return
    }

    Write-Host ""
    Write-Host "============================================================"
    Write-Host "TIGA - WINDOWS CURRENT MEDIA SESSION"
    Write-Host "============================================================"
    Write-Host ("Source     : {0}" -f $Media.SourceApp)
    Write-Host ("State      : {0}" -f $Media.PlaybackStatus)
    Write-Host ("Title      : {0}" -f $(if ($Media.Title) { $Media.Title } else { "(empty)" }))
    Write-Host ("Artist     : {0}" -f $(if ($Media.Artist) { $Media.Artist } else { "(empty)" }))
    Write-Host ("Album      : {0}" -f $(if ($Media.Album) { $Media.Album } else { "(empty)" }))
    Write-Host ("AlbumArtist: {0}" -f $(if ($Media.AlbumArtist) { $Media.AlbumArtist } else { "(empty)" }))
    Write-Host ("Subtitle   : {0}" -f $(if ($Media.Subtitle) { $Media.Subtitle } else { "(empty)" }))
    Write-Host ("Track No.  : {0}" -f $Media.TrackNumber)
    Write-Host ("Thumbnail  : {0}" -f $(if ($Media.Thumbnail) { "YES" } else { "NO" }))
}

if (-not $Watch) {
    Show-Media (Get-CurrentWindowsMedia)
    exit 0
}

Write-Host "Watching Windows current media session every $IntervalMs ms."
Write-Host "Switch between Spotify / YouTube / browser media while this runs."
Write-Host "Ctrl+C to stop."

$lastKey = $null

while ($true) {
    try {
        $media = Get-CurrentWindowsMedia

        if ($null -eq $media) {
            $key = "<none>"
        }
        else {
            $key = @(
                $media.SourceApp,
                $media.PlaybackStatus,
                $media.Title,
                $media.Artist,
                $media.Album,
                $media.Subtitle,
                $media.TrackNumber,
                $media.Thumbnail
            ) -join "|"
        }

        if ($key -ne $lastKey) {
            Show-Media $media
            $lastKey = $key
        }
    }
    catch {
        Write-Host ""
        Write-Warning ("Media query failed: " + $_.Exception.Message)
    }

    Start-Sleep -Milliseconds $IntervalMs
}
