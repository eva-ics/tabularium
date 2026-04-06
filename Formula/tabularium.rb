class Tabularium < Formula
  desc "AI-oriented markdown document store with CLI and HTTP server"
  homepage "https://github.com/eva-ics/tabularium"
  url "__TABULARIUM_TARBALL__"
  version "__TABULARIUM_VERSION__"
  sha256 "__TABULARIUM_SHA256__"
  license "Apache-2.0"

  def install
    bin.install ".brew-dist/tabularium-server"
    bin.install ".brew-dist/tb"

    (etc/"tabularium").mkpath
    unless (etc/"tabularium/config.toml").exist?
      cp "config.toml.example", etc/"tabularium/config.toml"
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
    run [opt_bin/"tabularium-server", etc/"tabularium/config.toml"]
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
