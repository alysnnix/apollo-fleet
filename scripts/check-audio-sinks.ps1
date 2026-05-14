# Lists active playback (render) devices on this host so you can copy the
# exact display name into seats.toml's `audio_sink` field.
#
# Apollo expects the *display name* exactly as Windows shows it (e.g.
# "Steam Streaming Speakers", "CABLE Input (VB-Audio Virtual Cable)").
#
# Recommended setup for 2 seats:
#   - Seat 1 -> "Steam Streaming Speakers"  (installed by Steam; auto-present)
#   - Seat 2 -> "CABLE Input (VB-Audio Virtual Cable)"
#       Install: https://vb-audio.com/Cable/  (free, restart required)
#
# For 3+ seats, install VB-Audio "Cable A+B" or the "Hi-Fi Cable" pack to get
# additional independent virtual sinks.
#
# After Windows recognizes both sinks, in Sound settings > "App volume and
# device preferences" route each game instance's output to a different sink.

Get-CimInstance -ClassName Win32_SoundDevice |
    Where-Object { $_.Status -eq 'OK' } |
    Select-Object Name, Manufacturer, Status |
    Format-Table -AutoSize

Write-Host ""
Write-Host "More detailed (render endpoints):" -ForegroundColor Cyan

# Try AudioDeviceCmdlets if installed, otherwise fall back to MMDeviceEnumerator via .NET
if (Get-Module -ListAvailable -Name AudioDeviceCmdlets) {
    Import-Module AudioDeviceCmdlets
    Get-AudioDevice -List | Where-Object { $_.Type -eq 'Playback' } |
        Select-Object Index, Name, Default | Format-Table -AutoSize
} else {
    Write-Host "(install with: Install-Module -Name AudioDeviceCmdlets -Scope CurrentUser)"
}
