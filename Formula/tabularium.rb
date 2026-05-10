class Tabularium < Formula
  desc "AI-oriented markdown document store with CLI and HTTP server"
  homepage "https://github.com/eva-ics/tabularium"
  license "Apache-2.0"
  version "0.1.7"

  on_macos do
    url "https://github.com/eva-ics/tabularium/releases/download/v0.1.7/tb-v0.1.7-aarch64-apple-darwin.tar.gz"
    sha256 "0a98ce872323a4a86fcd8b4a909502789e989de90c620782007d2057faef5c60"

    resource "tabularium-server-bin" do
      url "https://github.com/eva-ics/tabularium/releases/download/v0.1.7/tabularium-server-v0.1.7-aarch64-apple-darwin.tar.gz"
      sha256 "892b1fd87e3c924d2857339b9b2b2bb19f0c38ab837223900bdfdb64f43e717b"
    end
  end

  on_linux do
    url "https://github.com/eva-ics/tabularium/releases/download/v0.1.7/tb-v0.1.7-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "0242a9a485fdfa3280e4965dc2dff7affce64dc2fe2524cd9a47acd57aa8d2fd"

    resource "tabularium-server-bin" do
      url "https://github.com/eva-ics/tabularium/releases/download/v0.1.7/tabularium-server-v0.1.7-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "87796d61dfeb17a57540d279d78bfdb81312c6b000cd62337eaa77130e2e5fa9"
    end
  end

  resource "default-config" do
    url "https://raw.githubusercontent.com/eva-ics/tabularium/v0.1.7/config.toml.example"
    sha256 "e1981dd27d3c7d408da800b523a2ce1f6d3cbd55878794da4d405d7bc4edc51c"
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
