# Winget manifest for `helioslite`
#
# KooshaPari's fork of tailcallhq/forgecode. Renamed binary, same
# MIT-licensed source. See docs/NOTICE.md for fork attribution.
#
# Required by `winget install --id KooshaPari.HeliosLite`
#
# Validated against schemar: Microsoft.Winget.Manifest.Locale v1.6.0
# and Microsoft.Winget.Manifest.Singletons v1.6.0.

PackageIdentifier: KooshaPari.HeliosLite
PackageVersion: 0.1.0
PackageLocale: en-US
Publisher: KooshaPari
PackageName: HeliosLite
License: MIT
ShortDescription: AI-DD/HITL-less coding agent (renamed from forge-dev)
PublisherUrl: https://kooshapari.com
PublisherSupportUrl: https://github.com/KooshaPari/heliosLite/issues
Author: KooshaPari
ManifestType: versioned
InstallerType: zip
Installers:
  - Architecture: x64
    InstallerUrl: https://github.com/KooshaPari/heliosLite/releases/download/v0.1.0/helioslite-windows-x64.zip
    InstallerSha256: <set at release>
  - Architecture: arm64
    InstallerUrl: https://github.com/KooshaPari/heliosLite/releases/download/v0.1.0/helioslite-windows-arm64.zip
    InstallerSha256: <set at release>
Commands:
  - helioslite
  - forge-dev
FileExtensions: []
ManifestType: singleton
Version: 0.1.0
DefaultLocale: en-US
