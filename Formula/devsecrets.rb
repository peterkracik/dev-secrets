# Homebrew formula for dev-secrets.
#
# This is the canonical formula. To publish the tap, copy this file into a
# repo named `peterkracik/homebrew-tap` (path `Formula/devsecrets.rb`). Then:
#
#   brew install peterkracik/tap/devsecrets
#
# It builds from the tagged source with the Rust toolchain. Bump `url`'s tag
# (and `version`) on each release. For a no-build install, download a prebuilt
# binary from the GitHub Releases page instead.
class Devsecrets < Formula
  desc "Telescope-style TUI and CLI for managing local development secrets"
  homepage "https://github.com/peterkracik/dev-secrets"
  version "0.1.0"
  url "https://github.com/peterkracik/dev-secrets.git", tag: "v#{version}"
  license "MIT"
  head "https://github.com/peterkracik/dev-secrets.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "devsecrets", shell_output("#{bin}/devsecrets --help")
  end
end
