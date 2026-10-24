// Running CLI tools through cmd.exe on Windows (no vscode dependency).

const WINDOWS_COMMAND_NOT_FOUND = 9009; // cmd.exe's exit code for an unknown command

// RubyGems installs .bat shims on Windows, which execFile can't run directly (Node refuses .bat without a shell).
// ponytail: `%VAR%` inside an argument still expands in cmd.exe; none of our callers pass one.
function windowsShellCommand(bin, args) {
  const quote = (a) => `"${a.replace(/"/g, '\\"')}"`;
  return [
    "cmd.exe",
    ["/d", "/s", "/c", `"${[bin, ...args].map(quote).join(" ")}"`],
    { windowsVerbatimArguments: true },
  ];
}

export { WINDOWS_COMMAND_NOT_FOUND, windowsShellCommand };
