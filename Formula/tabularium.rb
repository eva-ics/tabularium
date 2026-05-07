class Tabularium < Formula
  desc "AI-oriented markdown document store with CLI and HTTP server"
  homepage "https://github.com/eva-ics/tabularium"
  license "Apache-2.0"
  version "0.1.6"

  on_macos do
    url "https://github.com/eva-ics/tabularium/releases/download/v0.1.6/tb-v0.1.6-aarch64-apple-darwin.tar.gz"
    sha256 "a4411d27c07b8b6c33a425a220749cf6808fdaabc342569e7806df3464f10fe9"

    resource "tabularium-server-bin" do
      url "https://github.com/eva-ics/tabularium/releases/download/v0.1.6/tabularium-server-v0.1.6-aarch64-apple-darwin.tar.gz"
      sha256 "ba52332029b79fcc4564fa0bf96fa1af555088566acc53cfcae4c029de31ad8f"
    end
  end

  on_linux do
    url "https://github.com/eva-ics/tabularium/releases/download/v0.1.6/tb-v0.1.6-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "0d397bc62ac533ddf32cc0e6ce318886364a235eda1c578423d25f615e10e388"

    resource "tabularium-server-bin" do
      url "https://github.com/eva-ics/tabularium/releases/download/v0.1.6/tabularium-server-v0.1.6-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "4ee86159f9129798cdab85e3ff5bf60091be1ab84ea16766f11365e14d7c6697"
    end
  end

  resource "default-config" do
    url "https://raw.githubusercontent.com/eva-ics/tabularium/v0.1.6/config.toml.example"
    sha256 "93458e516c40271f309a776f602fe5cf7a6197abfe1a33e4c9e616802a601bd1"
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
