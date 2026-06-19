# Homebrew formula for dev-secrets.
#
# Until tagged release bottles are published, this installs the latest from
# the main branch and builds from source:
#
#   brew install --HEAD https://raw.githubusercontent.com/peterkracik/localenvs/main/Formula/devsecrets.rb
#
# Or, once dropped into a tap (e.g. peterkracik/homebrew-tap):
#
#   brew install --HEAD peterkracik/tap/devsecrets
#
class Devsecrets < Formula
  desc "Telescope-style TUI and CLI for managing local development secrets"
  homepage "https://github.com/peterkracik/localenvs"
  license "MIT"
  head "https://github.com/peterkracik/localenvs.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "devsecrets", shell_output("#{bin}/devsecrets --help")
  end
end
