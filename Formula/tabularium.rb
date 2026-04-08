class Tabularium < Formula
  desc "AI-oriented markdown document store with CLI and HTTP server"
  homepage "https://github.com/eva-ics/tabularium"
  license "Apache-2.0"
  version "0.1.3"

  on_macos do
    url "https://github.com/eva-ics/tabularium/releases/download/v0.1.3/tb-v0.1.3-aarch64-apple-darwin.tar.gz"
    sha256 "df63d981f2be6450201abf5e560c3f0d03d51c3a4950cdd8f92a7830fa3dbcb0"

    resource "tabularium-server-bin" do
      url "https://github.com/eva-ics/tabularium/releases/download/v0.1.3/tabularium-server-v0.1.3-aarch64-apple-darwin.tar.gz"
      sha256 "2babbc41aaf6593aca03496d337976ab895943f27694be15a9567e2c7fc77e68"
    end
  end

  on_linux do
    url "https://github.com/eva-ics/tabularium/releases/download/v0.1.3/tb-v0.1.3-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "c0553e29f956f0490338a411112a877947c61249ea124d0dde264630c1a4ffc3"

    resource "tabularium-server-bin" do
      url "https://github.com/eva-ics/tabularium/releases/download/v0.1.3/tabularium-server-v0.1.3-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "a78cccdcd4a38e828a15110f1b0352be4d22e4062f2018b99e6b55e086c28c2a"
    end
  end

  resource "default-config" do
    url "https://raw.githubusercontent.com/eva-ics/tabularium/v0.1.3/config.toml.example"
    sha256 "c581ecebdc67c0b057f1920345a7eb99458741fbb45b8e840212dbd9beac096d"
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
