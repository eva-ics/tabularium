class Tabularium < Formula
  desc "AI-oriented markdown document store with CLI and HTTP server"
  homepage "https://github.com/eva-ics/tabularium"
  license "Apache-2.0"
  version "0.1.3"

  url "https://github.com/eva-ics/tabularium/releases/download/v0.1.3/tb-v0.1.3-aarch64-apple-darwin.tar.gz"
  sha256 "9ec00250ac873ef7ae5119728bedbc7d3333f0f38b5ed4f7f01cc1d00c59e975"

  resource "tabularium-server-bin" do
    url "https://github.com/eva-ics/tabularium/releases/download/v0.1.3/tabularium-server-v0.1.3-aarch64-apple-darwin.tar.gz"
    sha256 "1e1a3d0f70fe666151545e4b51805155787e6598736e233d49007beeefcc464c"
  end

  resource "default-config" do
    url "https://raw.githubusercontent.com/eva-ics/tabularium/v0.1.3/config.toml.example"
    sha256 "c581ecebdc67c0b057f1920345a7eb99458741fbb45b8e840212dbd9beac096d"
  end

  def install
    odie "tabularium Homebrew formula currently supports only Apple Silicon (arm64) macOS" if Hardware::CPU.intel?

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
