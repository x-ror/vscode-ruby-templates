// Extension-host checks: commands and the language server through VS Code's own APIs.
import assert from "assert";
import fs from "fs";
import path from "path";
import * as vscode from "vscode";

const app = process.env.RT_APP;
const settings = () => vscode.workspace.getConfiguration("rubyTemplates");
/** The gem's executable, or undefined when it isn't installed (the check is skipped). */
const gemTool = (name) => {
  const file = path.join(process.env.RT_GEM_BIN ?? "", name);
  return process.env.RT_GEM_BIN && fs.existsSync(file) ? file : undefined;
};
const open = async (rel) =>
  vscode.window.showTextDocument(
    await vscode.workspace.openTextDocument(path.join(app, rel)),
  );
const until = async (what, check, ms = 15000) => {
  const deadline = Date.now() + ms;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
};

const checks = {
  async "toggle ERB tag"() {
    const editor = await open("app/views/users/toggle.html.erb");
    editor.selection = new vscode.Selection(0, 3, 0, 3);
    await vscode.commands.executeCommand("rubyTemplates.toggleErbTag");
    assert.strictEqual(editor.document.lineAt(0).text, "<%= x %>");
    await vscode.commands.executeCommand("workbench.action.files.revert");
  },

  async "language server diagnostics"() {
    const editor = await open("app/views/users/index.html.erb");
    const messages = await until("diagnostics", () => {
      const d = vscode.languages.getDiagnostics(editor.document.uri);
      return d.length && d.map((x) => x.message);
    });
    assert.ok(
      messages.some((m) => m.includes("never closed")),
      messages.join("\n"),
    );
  },

  async "go to partial"() {
    const editor = await open("app/views/users/index.html.erb");
    const locations = await until("definition", () =>
      vscode.commands.executeCommand(
        "vscode.executeDefinitionProvider",
        editor.document.uri,
        new vscode.Position(1, 14),
      ),
    );
    assert.strictEqual(
      path.basename((locations[0].targetUri ?? locations[0].uri).fsPath),
      "_card.html.erb",
    );
  },

  async "switch to related: mailer formats"() {
    await open("app/views/user_mailer/welcome.html.erb");
    await vscode.commands.executeCommand("rubyTemplates.switchRelated");
    const name = await until("related editor", () => {
      const file = vscode.window.activeTextEditor?.document.fileName;
      return file?.endsWith("welcome.text.erb") && file;
    });
    assert.ok(name);
  },

  async "ERB formatter: htmlbeautifier"() {
    const bin = gemTool("htmlbeautifier");
    if (!bin) return "skipped: htmlbeautifier not installed";
    const global = vscode.ConfigurationTarget.Global;
    await settings().update("commands.htmlbeautifier", bin, global);
    await settings().update("erb.formatter", "htmlbeautifier", global);
    const editor = await open("app/views/users/unformatted.html.erb");
    const edits = await until("format edits", () =>
      vscode.commands.executeCommand(
        "vscode.executeFormatDocumentProvider",
        editor.document.uri,
        { tabSize: 2, insertSpaces: true },
      ),
    );
    // VS Code turns the whole-document replacement into a minimal diff, so compare the result.
    const edit = new vscode.WorkspaceEdit();
    edit.set(editor.document.uri, edits);
    await vscode.workspace.applyEdit(edit);
    assert.strictEqual(
      editor.document.getText(),
      "<div>\n  <% if x %>\n    <p>a</p>\n  <% end %>\n</div>\n",
    );
    await vscode.commands.executeCommand("workbench.action.files.revert");
    await settings().update("erb.formatter", undefined, global);
  },

  async "erb_lint diagnostics"() {
    const bin = gemTool("erb_lint");
    if (!bin) return "skipped: erb_lint not installed";
    const global = vscode.ConfigurationTarget.Global;
    await settings().update("commands.erbLint", bin, global);
    await settings().update("erbLint.enabled", true, global);
    const editor = await open("app/views/users/lint.html.erb");
    const lints = await until(
      "erb_lint offenses",
      () =>
        vscode.languages
          .getDiagnostics(editor.document.uri)
          .filter((d) => d.source === "erb_lint").length,
      60000,
    );
    assert.strictEqual(lints, 2);
    await settings().update("erbLint.enabled", undefined, global);
  },

  async "switch to related: component class"() {
    await open("app/components/badge_component.html.erb");
    await vscode.commands.executeCommand("rubyTemplates.switchRelated");
    await until("component class", () =>
      vscode.window.activeTextEditor?.document.fileName.endsWith(
        "badge_component.rb",
      ),
    );
  },
};

export async function run() {
  const failures = [];
  for (const [name, check] of Object.entries(checks)) {
    try {
      const note = await check();
      console.log(`ok   ${name}${note ? ` (${note})` : ""}`);
    } catch (e) {
      failures.push(name);
      console.log(`FAIL ${name}: ${e.stack ?? e}`);
    }
  }
  if (failures.length) throw new Error(`${failures.length} check(s) failed`);
}
