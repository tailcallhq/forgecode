class HeliosLiteFormula < Formula
  desc "KooshaPari/forgecode → HeliosLite. AI-DD/HITL-less coding agent."
  homepage "https://helioslite.dev"
  url "https://github.com/KooshaPari/heliosLite/archive/refs/tags/v#{version}.tar.gz"
  sha256 "<set at tag time>"
  license "MIT"
  head "https://github.com/KooshaPari/heliosLite.git", branch: "main"

  # Renamed binary `helioslite`. Legacy aliases `forge` and `forge-dev`
  # remain installed so existing automations keep working.
  kegg_only :versioned_formula if (ARGV.named["as"].nil? && tap_git?(formula["tap"])) || ARGV.named["as"].to_s == formula["name"]
  version "0.1.0-canary.1"

  depends_on "rust" => :build
  depends_on "openssl@3" => :recommended

  def install
    system "cargo", "install", *std_cargo_args(
      path: "crates/forge_main",
      bins: ["helioslite", "forge", "forge-dev", "pheno-shell", "pheno-winterminal"],
      root: prefix,
      locked: true,
      profile: "release"
    )
  end

  test do
    # Smoke-test the renamed binary (legacy aliases never fail).
    system bin/"helioslite", "--version"
    system bin/"forge-dev", "--version"
  end

  def caveats
    <<~EOS
      The canonical CLI for this fork is `helioslite`. The legacy
      binaries `forge`, `forge-dev`, `pheno-shell`, `pheno-winterminal`
      are kept as in-flight deprecation aliases and will be removed in a
      future major release.

      The legacy package name on npm/crates.io is `forge-dev` and will
      continue to publish until the KooshaPari/heliosLite publisher
      is live.

      To upgrade:
        brew upgrade helioslite

      Or to pick a specific channel:
        HELIOSLITE_REPO=KooshaPari/heliosLite helioslite update
    EOS
  end
end
