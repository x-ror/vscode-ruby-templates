// Pure part of the route helpers (no vscode dependency; tested in test/clientPure.js).
import path from "node:path";

const ROUTES_SCRIPT = path.join(import.meta.dirname, "..", "ruby", "routes.rb");
const MARKER = "__RUBY_TEMPLATES_ROUTES__";

/** The routes `routes.rb` printed after its marker, or undefined when the output has none. */
function routesFromOutput(stdout) {
  const lines = stdout.split(/\r?\n/);
  const at = lines.lastIndexOf(MARKER);
  if (at === -1) return undefined;
  const routes = JSON.parse(lines[at + 1]);
  return Array.isArray(routes) ? routes : undefined;
}

export { ROUTES_SCRIPT, routesFromOutput };
