class Tabularium < Formula
  desc "AI-oriented markdown document store with CLI and HTTP server"
  homepage "https://github.com/velortechnologies/tabularium"
  license "Apache-2.0"
  version "0.1.9"

  on_macos do
    url "https://github.com/velortechnologies/tabularium/releases/download/v0.1.9/tb-v0.1.9-aarch64-apple-darwin.tar.gz"
    sha256 "9f44ef2e63412705df2639e2ed5e55fb59caced3e1e0534aa5ebdab92882f934"

    resource "tabularium-server-bin" do
      url "https://github.com/velortechnologies/tabularium/releases/download/v0.1.9/tabularium-server-v0.1.9-aarch64-apple-darwin.tar.gz"
      sha256 "cee93a1ce745782d35a6b1105f1ea50b7c69ea89c4fcee18c1d755c30b9232f0"
    end
  end

  on_linux do
    url "https://github.com/velortechnologies/tabularium/releases/download/v0.1.9/tb-v0.1.9-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "72652a05077ede8503ac71644c4eec1395fd46682e3b1a0f0a2f9fc906acddef"

    resource "tabularium-server-bin" do
      url "https://github.com/velortechnologies/tabularium/releases/download/v0.1.9/tabularium-server-v0.1.9-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "ba6243d27d21be377255d9c7edc873072dcfd0365080f6b6fb564914d00bd8ca"
    end
  end

  resource "default-config" do
    url "https://raw.githubusercontent.com/velortechnologies/tabularium/v0.1.9/config.toml.example"
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
