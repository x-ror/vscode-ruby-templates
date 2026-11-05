// Feeding each Rails app's routes to the server for `*_path` / `*_url` completion, hover and definition.
import * as vscode from "vscode";
import path from "path";

import { log, notifyServer, onServerStarted } from "./client.js";
import { ROUTES_SCRIPT, routesFromRun } from "./routes.js";
import { SECTION, settings, commandPath } from "./workspace.js";
import { RAILS_BIN, findUp, railsRunner, firstLine } from "./run.js";
import { isTemplateLanguage } from "./languages.js";

const routesEnabled = () => settings().get("routes.enabled", true);

/** The app holding `dir`; the disk is searched once per folder, as views open often. */
function railsRootOf({ roots }, dir) {
  if (!roots.has(dir)) roots.set(dir, findUp(dir, RAILS_BIN));
  return roots.get(dir);
}

// Booting runs the app's own code: only for an app inside a trusted workspace folder, never for a file
// opened on its own (an empty window is trusted by default) or a bin/rails found above the folder.
function trustedRailsRoot(state, file) {
  if (!routesEnabled() || !vscode.workspace.isTrusted) return undefined;
  const root = railsRootOf(state, path.dirname(file));
  const folder =
    root && vscode.workspace.getWorkspaceFolder(vscode.Uri.file(root));
  return folder && !path.relative(folder.uri.fsPath, root).startsWith("..")
    ? root
    : undefined;
}

async function sendRoutes(root, routes) {
  try {
    await notifyServer("rubyTemplates/routes", { root, routes });
  } catch (e) {
    log(`routes: sending routes for ${root}: ${e?.message ?? e}`);
  }
}

async function loadRoutes(state, root) {
  const r = await railsRunner(root, ROUTES_SCRIPT, {
    ruby: commandPath("ruby"),
    useBundler: false,
  });
  if (!routesEnabled()) return;
  const routes = routesFromRun(r);
  if (!routes) {
    log(
      `routes: bin/rails runner failed in ${root}: ${firstLine(r) || `exit ${r.code}`}`,
    );
    state.failed.add(root);
    return;
  }
  state.failed.delete(root);
  state.cache.set(root, routes);
  await sendRoutes(root, routes);
}

// A change while a load runs reloads once it finishes, so the last edit to routes.rb always counts.
async function refreshRoutes(state, root) {
  const { busy, again } = state;
  if (busy.has(root)) {
    again.add(root);
    return;
  }
  busy.add(root);
  try {
    do {
      again.delete(root);
      await loadRoutes(state, root);
    } while (again.has(root));
  } finally {
    busy.delete(root);
  }
}

/** Loads the routes of the app an opened view belongs to, unless they're loaded, loading or failed. */
function viewOpened(state, doc) {
  if (doc.uri.scheme !== "file") return;
  if (!isTemplateLanguage(doc.languageId)) return;
  const root = trustedRailsRoot(state, doc.uri.fsPath);
  const { cache, failed, busy } = state;
  if (root && !cache.has(root) && !failed.has(root) && !busy.has(root))
    void refreshRoutes(state, root);
}

function routesFileChanged(state, uri) {
  const root = trustedRailsRoot(state, uri.fsPath);
  if (root && (state.cache.has(root) || state.failed.has(root)))
    void refreshRoutes(state, root);
}

function settingsChanged(state, e) {
  if (
    !e.affectsConfiguration(`${SECTION}.routes`) &&
    !e.affectsConfiguration(`${SECTION}.commands.ruby`)
  )
    return;
  state.failed.clear();
  if (routesEnabled())
    vscode.workspace.textDocuments.forEach((doc) => viewOpened(state, doc));
  else {
    for (const root of state.cache.keys()) void sendRoutes(root, []);
    state.cache.clear();
  }
}

/**
 * Read each Rails app's routes with `bin/rails runner` once one of its views is open, again when its routes
 * files change, and hand them to the server.
 */
function registerRoutes(context) {
  const state = {
    cache: new Map(), // Rails root → routes
    // Roots whose boot failed: retried on a routes file change or a settings change, not on every view opened.
    failed: new Set(),
    busy: new Set(),
    again: new Set(),
    roots: new Map(), // folder → nearest Rails root above it, or undefined
  };
  const opened = (doc) => viewOpened(state, doc);
  const routesChanged = (uri) => routesFileChanged(state, uri);
  const watcher = vscode.workspace.createFileSystemWatcher(
    "**/config/{routes.rb,routes/**/*.rb}",
  );
  // Moving a whole bin/ folder reports the folder only.
  const appsWatcher =
    vscode.workspace.createFileSystemWatcher("**/{bin,bin/rails}");
  const appsChanged = () => state.roots.clear();
  context.subscriptions.push(
    appsWatcher,
    appsWatcher.onDidCreate(appsChanged),
    appsWatcher.onDidDelete(appsChanged),
    watcher,
    watcher.onDidChange(routesChanged),
    watcher.onDidCreate(routesChanged),
    watcher.onDidDelete(routesChanged),
    vscode.workspace.onDidOpenTextDocument(opened),
    vscode.workspace.onDidGrantWorkspaceTrust(() =>
      vscode.workspace.textDocuments.forEach(opened),
    ),
    onServerStarted(() => {
      for (const [root, routes] of state.cache) void sendRoutes(root, routes);
    }),
    vscode.workspace.onDidChangeConfiguration((e) => settingsChanged(state, e)),
  );
  vscode.workspace.textDocuments.forEach(opened);
}

export { registerRoutes };
