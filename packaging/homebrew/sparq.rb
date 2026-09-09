# Homebrew formula TEMPLATE for the sparq CLI (T20).
#
# This file is NOT consumed from this repository: Homebrew formulae live in a tap
# (e.g. github.com/jeswr/homebrew-sparq, file Formula/sparq.rb). Creating that tap is the
# maintainer's repo decision — the release runbook (docs/release.md) covers copying this
# file there and filling in the sha256 placeholders from the release's SHA256SUMS asset.
#
# Artifact mapping (built by .github/workflows/release.yml on tag v<version>):
#   macOS arm64  -> sparq-cli-v<version>-arm64-darwin.tar.gz   (M1-M4)
#   macOS intel  -> sparq-cli-v<version>-x64-darwin.tar.gz     (x86-64-v3: any Intel Mac 2015+)
#   linux arm64  -> sparq-cli-v<version>-arm64-linux.tar.gz    (neoverse-n1 + LSE)
#   linux x86-64 -> sparq-cli-v<version>-x64-v2.tar.gz         (SSE4.2, ~2009+; a safe brew
#                   default — v3/v4 binaries exist on the release page for newer CPUs)
class Sparq < Formula
  desc "From-scratch RDF triplestore and SPARQL engine (dictionary-encoded, parallel)"
  homepage "https://github.com/sparq-org/sparq"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/sparq-org/sparq/releases/download/v#{version}/sparq-cli-v#{version}-arm64-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_arm64-darwin"
    end
    on_intel do
      url "https://github.com/sparq-org/sparq/releases/download/v#{version}/sparq-cli-v#{version}-x64-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_x64-darwin"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/sparq-org/sparq/releases/download/v#{version}/sparq-cli-v#{version}-arm64-linux.tar.gz"
      sha256 "REPLACE_WITH_SHA256_arm64-linux"
    end
    on_intel do
      url "https://github.com/sparq-org/sparq/releases/download/v#{version}/sparq-cli-v#{version}-x64-v2.tar.gz"
      sha256 "REPLACE_WITH_SHA256_x64-v2"
    end
  end

  def install
    # Archives unpack to sparq-cli-v<version>-<tier>/sparq-cli (brew strips the top dir).
    bin.install "sparq-cli"
    # Friendly alias: `sparq` on the PATH alongside the canonical binary name.
    bin.install_symlink "sparq-cli" => "sparq"
  end

  test do
    # No-args prints usage to stderr and exits 2.
    assert_match "usage", shell_output("#{bin}/sparq-cli 2>&1", 2)
  end
end
