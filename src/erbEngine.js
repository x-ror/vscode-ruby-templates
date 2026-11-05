// Pure part of ERB preview/compile (no vscode dependency; tested in test/clientPure.js).
import path from "node:path";

// A script file, not `-e`: Windows runs commands through cmd.exe, which can't pass its quotes and newlines.
const SCRIPT = path.join(import.meta.dirname, "..", "ruby", "erb_engine.rb");

/** `ruby` arguments that compile (`"compile"`) or render (`"render"`) the ERB on stdin; `percentLines` for `trim_mode: "%"`. */
function erbRubyArgs(mode, percentLines = false) {
  return percentLines ? [SCRIPT, mode, "percent"] : [SCRIPT, mode];
}

export { erbRubyArgs };
