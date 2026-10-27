class Tabularium < Formula
  desc "AI-oriented markdown document store with CLI and HTTP server"
  homepage "https://github.com/velortechnologies/tabularium"
  license "Apache-2.0"
  version "0.1.8"

  on_macos do
    url "https://github.com/velortechnologies/tabularium/releases/download/v0.1.8/tb-v0.1.8-aarch64-apple-darwin.tar.gz"
    sha256 "0bbf96c446e05aac27af0d00a2e9a6bbdfad4718f12a5077639d768b4787fe1e"

    resource "tabularium-server-bin" do
      url "https://github.com/velortechnologies/tabularium/releases/download/v0.1.8/tabularium-server-v0.1.8-aarch64-apple-darwin.tar.gz"
      sha256 "edf4f70985a2229ac34df03dbdddf46b2e3b1e1a5521542c2c5c904999d27701"
    end
  end

  on_linux do
    url "https://github.com/velortechnologies/tabularium/releases/download/v0.1.8/tb-v0.1.8-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "e62d3eaeba84273f3bea1d16ab5caa74341c126ba0a60e71e0455ccad25e9551"

    resource "tabularium-server-bin" do
      url "https://github.com/velortechnologies/tabularium/releases/download/v0.1.8/tabularium-server-v0.1.8-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "443b23cccb492b294c924ffd14e6c511af514db6b0604e2a63c663ce9ef5757f"
    end
  end

  resource "default-config" do
    url "https://raw.githubusercontent.com/velortechnologies/tabularium/v0.1.8/config.toml.example"
    sha256 "7ee6d2cd04b536b815edda8ca8629b943cf6f94cdc6d807aeb4770d0fbae8758"
  end

  def install
    bin.install "tb"
    resource("tabularium-server-bin").stage do
      bin.install "tabularium-server"
    end
    (etc/"tabularium").mkpath
    unless (etc/"tabularium/config.toml").exist?
      resource("default-config").stage do
        cp "config.toml.example", etc/"tabularium/config.toml"
      end
      inreplace etc/"tabularium/config.toml" do |s|
        s.gsub!("./data/tabularium.db", "#{var}/tabularium/tabularium.db")
        s.gsub!("./data/tabularium.index", "#{var}/tabularium/tabularium.index")
      end
    end
  end

  def post_install
    (var/"tabularium").mkpath
  end

  service do
    run [opt_bin/"tabularium-server", "--config", etc/"tabularium/config.toml"]
    keep_alive true
    working_dir var/"tabularium"
    log_path var/"log/tabularium.log"
    error_log_path var/"log/tabularium.error.log"
  end

  test do
    assert_predicate bin/"tb", :exist?
    assert_predicate bin/"tabularium-server", :exist?
  end
end
