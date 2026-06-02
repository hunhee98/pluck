class Pluck < Formula
  desc "Fast and token-friendly code reading for AI coding agents"
  homepage "https://github.com/hunhee98/pluck"
  url "https://github.com/hunhee98/pluck/archive/refs/tags/v0.6.0.tar.gz"
  sha256 "33eb4f57715f6219dbc5310424d243b1e7c13ba05f57e361fac133f34c990684"
  license "MIT"
  head "https://github.com/hunhee98/pluck.git", branch: "main"

  depends_on "rust" => :build
  depends_on "ripgrep" # pluck.grep shells out to `rg`

  def install
    # Build all binaries declared by the workspace.
    system "cargo", "install", *std_cargo_args(path: "crates/pluck-cli")
    system "cargo", "install", *std_cargo_args(path: "crates/pluck-mcp")
  end

  test do
    # Both binaries should report the installed version. The CLI
    # binary prints its package name (`pluck-cli` / `pluck-mcp`),
    # not its on-disk name (`pluck` / `pluckd`), so we assert on
    # the version string instead.
    assert_match version.to_s, shell_output("#{bin}/pluck --version")
    assert_match version.to_s, shell_output("#{bin}/pluckd --version")

    # A tiny end-to-end: index this very dir, search for the help text,
    # confirm we get a hit.
    (testpath/"sample.ts").write <<~TS
      function handleLogin(user: string): boolean {
        return user.length > 0;
      }
    TS
    system "#{bin}/pluck", "index", testpath.to_s
    output = shell_output("#{bin}/pluck search 'handleLogin' --repo #{testpath} --compact")
    assert_match "handleLogin", output
  end
end
