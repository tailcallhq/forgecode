class Forge < Formula
  desc "Fastest AI coding agent — 40+ models, local-first, zero data exfiltration"
  homepage "https://github.com/KooshaPari/forgecode"
  version "2.13.21-h.0.1.4"
  license "MIT"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/KooshaPari/forgecode/releases/download/v#{version}/forge-x86_64-apple-darwin"
      sha256 "0478799b41aa061cf68fa79b888819446463a89c4304ab4d1754f99564ffd3c6"
    else
      url "https://github.com/KooshaPari/forgecode/releases/download/v#{version}/forge-aarch64-apple-darwin"
      sha256 "6c0ca2ce016586502c5c5cc2c8dc394bfe814baaa47717e6810f8b86f72572cf"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url "https://github.com/KooshaPari/forgecode/releases/download/v#{version}/forge-x86_64-unknown-linux-gnu"
      sha256 "1bee062e167504271ac82a9f23f95e990d46c318dde9a5ab3bbe5891dc06729e"
    else
      url "https://github.com/KooshaPari/forgecode/releases/download/v#{version}/forge-aarch64-unknown-linux-gnu"
      sha256 "8eac115e2a8b679af8ad6c1403f56fc5262d6ea511138bfbb222a4b642b775fa"
    end
  end

  def install
    bin.install Dir["forge*"].first => "forge"
    bin.install Dir["forge_dbd*"].first => "forge_dbd" if Dir["forge_dbd*"].any?
  end

  test do
    assert_match "forge", shell_output("#{bin}/forge --version")
  end
end
