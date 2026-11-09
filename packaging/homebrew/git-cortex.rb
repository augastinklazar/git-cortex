class GitCortex < Formula
  desc "Git-native Epistemic Memory Engine & Protocol for AI Coding Agents"
  homepage "https://github.com/augastinklazar/git-cortex"
  url "https://github.com/augastinklazar/git-cortex/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "REPLACE_WITH_RELEASE_TARBALL_SHA256"
  license any_of: ["MIT", "Apache-2.0"]
  head "https://github.com/augastinklazar/git-cortex.git", branch: "main"

  depends_on "rust" => :build
  depends_on "git"

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/cortex-cli")
  end

  test do
    system "#{bin}/git-cortex", "--version"
    system "#{bin}/cortex", "--version"
  end
end
