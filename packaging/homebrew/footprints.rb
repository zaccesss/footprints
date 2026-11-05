# Homebrew formula for Footprints 0.2.0, for Formula/footprints.rb in the zaccesss/homebrew-tap
# repository. Each SHA256_ placeholder is the sum from the matching .sha256 file attached to the
# v0.2.0 release; packaging/fill-checksums.sh fills them in. The release workflow moves a copy to
# each new tag's version before filling it, so the version here can lag behind the latest release.
class Footprints < Formula
  desc "See and undo what AI agents do on your machine"
  homepage "https://github.com/zaccesss/footprints"
  version "0.2.0"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/zaccesss/footprints/releases/download/v0.2.0/footprints-v0.2.0-aarch64-apple-darwin.tar.gz"
      sha256 "SHA256_AARCH64_APPLE_DARWIN"
    end
    on_intel do
      url "https://github.com/zaccesss/footprints/releases/download/v0.2.0/footprints-v0.2.0-x86_64-apple-darwin.tar.gz"
      sha256 "SHA256_X86_64_APPLE_DARWIN"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/zaccesss/footprints/releases/download/v0.2.0/footprints-v0.2.0-aarch64-unknown-linux-musl.tar.gz"
      sha256 "SHA256_AARCH64_UNKNOWN_LINUX_MUSL"
    end
    on_intel do
      url "https://github.com/zaccesss/footprints/releases/download/v0.2.0/footprints-v0.2.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "SHA256_X86_64_UNKNOWN_LINUX_MUSL"
    end
  end

  def install
    bin.install "footprints"
    doc.install "README.md", "CHANGELOG.md", "ACCESSIBILITY.md", "NOTICE"
  end

  test do
    assert_match "footprints #{version}", shell_output("#{bin}/footprints --version")
    ENV["FOOTPRINTS_HOME"] = testpath/"ledger"
    (testpath/"work").mkpath
    system bin/"footprints", "run", "--dir", testpath/"work", "--", "sh", "-c", "echo hi > made.txt"
    assert_match "file_create", shell_output("#{bin}/footprints log")
  end
end
