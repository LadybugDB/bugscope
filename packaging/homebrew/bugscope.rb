# Homebrew formula for bugscope (native LadybugDB graph visualizer).
#
# Install from this tap:
#   brew tap <you>/bugscope  # with this file at Formula/bugscope.rb
#   brew install bugscope
#
# Layout: the binary lives in libexec next to icebug/lib + liblbug, so the
# baked @executable_path rpaths (.cargo/config.toml) resolve with no env
# vars; `bin` gets a symlink. The runtime-downloaded algo extension
# (INSTALL algo on first GDS_* use) resolves its @rpath deps via the
# binary's LC_RPATHs, which also cover HOMEBREW_PREFIX lib dirs below.
#
# TODO: fill in `url`/`sha256` per release and the repo license.
class Bugscope < Formula
  desc "Native graph visualizer for LadybugDB"
  homepage "https://github.com/LadybugDB/bugscope"
  url "https://github.com/LadybugDB/bugscope/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "REPLACE_WITH_RELEASE_TARBALL_SHA256"
  # license "REPLACE_WITH_REPO_LICENSE"

  depends_on "cmake" => :build
  depends_on "pkg-config" => :build
  depends_on "rust" => :build
  depends_on "apache-arrow"
  depends_on "libomp"
  depends_on "openssl@3"

  def install
    # Prebuilt Networkit + shared liblbug (the algo extension dlopens
    # against the shared ladybug symbols at LOAD time).
    system "bash", "scripts/download_icebug.sh"
    system "bash", "scripts/download-liblbug.sh"
    # Vendor brew arrow/omp next to libnetworkit for version-pinned loads.
    system "bash", "scripts/vendor_arrow.sh"

    system "cargo", "build", "--release", "--locked"

    libexec.install "target/release/bugscope"
    libexec.install "icebug" => "icebug"
    libexec.install "liblbug" => "liblbug"
    # Belt and braces next to the baked-in rpaths: the install prefix and
    # the brewed arrow/omp locations, so relocation never breaks @rpath.
    for rpath in [libexec/"icebug/lib", libexec/"liblbug", HOMEBREW_PREFIX/"lib",
                  HOMEBREW_PREFIX/"opt/apache-arrow/lib", HOMEBREW_PREFIX/"opt/libomp/lib"] do
      quiet_system "install_name_tool", "-add_rpath", rpath, libexec/"bugscope"
    end
    bin.install_symlink libexec/"bugscope"
  end

  test do
    assert_match "Usage", shell_output("#{bin}/bugscope --help")
  end
end
