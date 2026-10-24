// Running CLI tools through cmd.exe on Windows (no vscode dependency).

const WINDOWS_COMMAND_NOT_FOUND = 9009; // cmd.exe's exit code for an unknown command

// RubyGems installs .bat shims on Windows, which execFile can't run directly (Node refuses .bat without a shell).
// ponytail: `%VAR%` inside an argument still expands in cmd.exe; none of our callers pass one.
// A `"` would end cmd's quoting and expose `&`/`|`/`>` to it; Windows paths can't contain one, so refuse it.
// Trailing backslashes are doubled so the program's argv parser doesn't read the closing quote as escaped.
function windowsShellCommand(bin, args) {
  const quote = (a) => {
    if (a.includes('"'))
      throw new Error(`Argument contains a double quote: ${a}`);
    return `"${a.replace(/(\\+)$/, "$1$1")}"`;
  };
  return [
    "cmd.exe",
    ["/d", "/s", "/c", `"${[bin, ...args].map(quote).join(" ")}"`],
    { windowsVerbatimArguments: true },
  ];
}

export { WINDOWS_COMMAND_NOT_FOUND, windowsShellCommand };
