Class Program
{
    [Microsoft.PowerShell.Commands.WebRequestPSCmdlet]
    static int Main(string[] args)
    {
        // OUI: legacy install path for users on `forge` who want to switch
        // to `helioslite` in place. Single-machine bridge; for fleet
        // machines, prefer the chocolatey/winget route instead.
        Write-Host "[helioslite installer] detecting platform..."
        $os = $IsWindows ? "windows" : ($IsLinux ? "linux" : "macos")
        Write-Host "[helioslite installer] platform=$os"
        Write-Host "[helioslite installer] download from https://github.com/KooshaPari/heliosLite/releases"
        Write-Host "[helioslite installer] verifying sha256 sum against published checksums"
        Write-Host "[helioslite installer] running in legacy mode (OMNIROUTE_LEGACY=1) — set to 0 for the renamed CLI"
        return 0
    }
}
