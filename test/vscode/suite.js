// Extension-host checks: commands and the language server through VS Code's own APIs.
import assert from "assert";
import fs from "fs";
import path from "path";
import { spawnSync } from "child_process";
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

const select = (editor, startLine, startChar, endLine, endChar) => {
  editor.selection = new vscode.Selection(
    startLine,
    startChar,
    endLine,
    endChar,
  );
};
const revert = async (editor) => {
  await vscode.window.showTextDocument(editor.document);
  await vscode.commands.executeCommand("workbench.action.files.revert");
};
/** Runs `body` with showInputBox answering `name`; the real one waits for a human. */
const answeringInputBox = async (name, body) => {
  const original = vscode.window.showInputBox;
  try {
    vscode.window.showInputBox = async () => name;
  } catch {
    return "skipped: showInputBox can't be replaced";
  }
  if (vscode.window.showInputBox === original)
    return "skipped: showInputBox can't be replaced";
  try {
    await body();
  } finally {
    vscode.window.showInputBox = original;
  }
};
const applyEdits = async (editor, edits) => {
  const edit = new vscode.WorkspaceEdit();
  edit.set(editor.document.uri, edits);
  await vscode.workspace.applyEdit(edit);
};
const formatEdits = (editor) =>
  until("format edits", async () => {
    const edits = await vscode.commands.executeCommand(
      "vscode.executeFormatDocumentProvider",
      editor.document.uri,
      { tabSize: 2, insertSpaces: true },
    );
    return edits?.length && edits;
  });
/** Runs the command with `editor` selecting the range, returns the resulting text and reverts the file. */
const runOnSelection = async (rel, range, command) => {
  const editor = await open(rel);
  try {
    select(editor, ...range);
    await vscode.commands.executeCommand(command);
    return editor.document.getText();
  } finally {
    await revert(editor);
  }
};

const checks = {
  async "typing a whole ERB tag leaves one %>"() {
    const editor = await open("app/views/users/toggle.html.erb");
    const lines = [];
    for (const typed of ["<%= x %>", "<%# c %>", "<% if a %>", "<%"]) {
      await editor.edit((b) => b.insert(new vscode.Position(1, 0), "\n"));
      editor.selection = new vscode.Selection(1, 0, 1, 0);
      for (const ch of typed)
        await vscode.commands.executeCommand("type", { text: ch });
      // The fix runs after the change event, so give it a moment before reading the line.
      const line = () => editor.document.lineAt(1).text;
      await until("one %>", () => line().split("%>").length <= 2, 2000).catch(
        () => {},
      );
      lines.push(line());
      await vscode.commands.executeCommand("workbench.action.files.revert");
    }
    assert.deepStrictEqual(lines, [
      "<%= x %>",
      "<%# c %>",
      "<% if a %>",
      "<% %>",
    ]);
  },

  async "auto-close an HTML tag in ERB"() {
    const editor = await open("app/views/users/toggle.html.erb");
    await editor.edit((b) =>
      b.insert(new vscode.Position(1, 0), '<div class="<%= x %>"'),
    );
    editor.selection = new vscode.Selection(1, 21, 1, 21);
    await vscode.commands.executeCommand("type", { text: ">" });
    const line = await until("end tag", () => {
      const text = editor.document.lineAt(1).text;
      return text.endsWith("</div>") && text;
    });
    assert.strictEqual(line, '<div class="<%= x %>"></div>');
    assert.strictEqual(editor.selection.active.character, 22);
    await vscode.commands.executeCommand("workbench.action.files.revert");
  },

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

  async "switch to related: controller action"() {
    await open("app/views/orders/show.html.erb");
    await vscode.commands.executeCommand("rubyTemplates.switchRelated");
    const editor = await until("controller editor", () => {
      const e = vscode.window.activeTextEditor;
      return e?.document.fileName.endsWith("orders_controller.rb") && e;
    });
    await until("cursor on def show", () => editor.selection.active.line === 4);
  },

  async "route helpers from bin/rails"() {
    if (spawnSync("ruby", ["-v"]).status !== 0) return "skipped: no ruby";
    const editor = await open("app/views/users/routes.html.erb");
    const at = new vscode.Position(0, 22);
    const hover = await until("route hover", async () => {
      const hovers = await vscode.commands.executeCommand(
        "vscode.executeHoverProvider",
        editor.document.uri,
        at,
      );
      return hovers
        ?.flatMap((h) => h.contents.map((c) => c.value ?? c))
        .find((v) => v.includes("users#index"));
    });
    assert.ok(hover.includes("`/users`"), hover);
    const [def] = await vscode.commands.executeCommand(
      "vscode.executeDefinitionProvider",
      editor.document.uri,
      at,
    );
    const target = def.targetUri ?? def.uri;
    const line = (def.targetRange ?? def.range).start.line;
    assert.deepStrictEqual(
      [path.relative(app, target.fsPath), line],
      [path.join("config", "routes.rb"), 1],
    );
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
    await settings().update("commands.htmlbeautifier", undefined, global);
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
    await settings().update("commands.erbLint", undefined, global);
  },

  async "herb-lint diagnostics"() {
    const bin = process.env.RT_HERB_LINT;
    if (!bin || !fs.existsSync(bin)) return "skipped: RT_HERB_LINT not set";
    const global = vscode.ConfigurationTarget.Global;
    await settings().update("commands.herbLint", bin, global);
    await settings().update("herbLint.enabled", true, global);
    const editor = await open("app/views/users/lint.html.erb");
    const codes = await until(
      "herb-lint offenses",
      () => {
        const found = vscode.languages
          .getDiagnostics(editor.document.uri)
          .filter((d) => d.source === "herb-lint")
          .map((d) => d.code);
        return found.length && found;
      },
      60000,
    );
    assert.ok(codes.includes("erb-require-whitespace-inside-tags"), codes);
    await settings().update("herbLint.enabled", undefined, global);
    await settings().update("commands.herbLint", undefined, global);
  },

  async "wrap in ERB output: selection and empty cursor"() {
    const editor = await open("app/views/users/wrap.html.erb");
    try {
      select(editor, 0, 0, 0, 5);
      await vscode.commands.executeCommand("rubyTemplates.wrapInErb");
      assert.strictEqual(editor.document.getText(), "<%= hello %> world\n");
    } finally {
      await revert(editor);
    }
    const empty = await open("app/views/users/wrap_empty.html.erb");
    try {
      select(empty, 0, 0, 0, 0);
      await vscode.commands.executeCommand("rubyTemplates.wrapInErb");
      assert.strictEqual(empty.document.lineAt(0).text, "<%=  %>");
      // Cursor lands between the spaces, ready to type.
      assert.strictEqual(empty.selection.active.character, 4);
    } finally {
      await revert(empty);
    }
  },

  async "toggle Slim marker cycles lines but not engine bodies"() {
    const text = await runOnSelection(
      "app/views/users/toggle.html.slim",
      [0, 0, 5, 0],
      "rubyTemplates.toggleSlimTag",
    );
    assert.strictEqual(
      text,
      "= foo\n== bar\n/ baz\n- qux\njavascript:\n  - not a marker\n",
    );
  },

  async "wrap in Slim output: whole line, inline and empty cursor"() {
    const file = "app/views/users/wrap.html.slim";
    assert.strictEqual(
      await runOnSelection(file, [1, 2, 1, 13], "rubyTemplates.wrapInSlim"),
      "p\n  = hello world\n",
    );
    assert.strictEqual(
      await runOnSelection(file, [1, 8, 1, 13], "rubyTemplates.wrapInSlim"),
      "p\n  hello #{world}\n",
    );
    const editor = await open(file);
    try {
      select(editor, 1, 2, 1, 2);
      await vscode.commands.executeCommand("rubyTemplates.wrapInSlim");
      assert.strictEqual(editor.document.lineAt(1).text, "  #{}hello world");
      assert.strictEqual(editor.selection.active.character, 4);
    } finally {
      await revert(editor);
    }
  },

  async "toggle Haml prefix cycles - = -#"() {
    const text = await runOnSelection(
      "app/views/users/toggle.html.haml",
      [0, 0, 4, 0],
      "rubyTemplates.toggleHamlPrefix",
    );
    assert.strictEqual(text, "= foo\n-# bar\n- baz\n%p\n");
  },

  async "reindent Haml command"() {
    const text = await runOnSelection(
      "app/views/users/reindent.html.haml",
      [0, 0, 0, 0],
      "rubyTemplates.reindentHaml",
    );
    assert.strictEqual(text, "%div\n    %p\n        %b x\n");
  },

  async "Haml Format Document reindents"() {
    const editor = await open("app/views/users/format.html.haml");
    try {
      await applyEdits(editor, await formatEdits(editor));
      assert.strictEqual(editor.document.getText(), "%ul\n  %li\n    %b x\n");
    } finally {
      await revert(editor);
    }
  },

  async "Slim Format Document via the server"() {
    const editor = await open("app/views/users/format.html.slim");
    try {
      await applyEdits(editor, await formatEdits(editor));
      assert.strictEqual(editor.document.getText(), "div\n  p= x\n  p y\n");
    } finally {
      await revert(editor);
    }
  },

  async "extract ERB partial"() {
    const rel = "app/views/users/extract.html.erb";
    return answeringInputBox("erb_part", async () => {
      const editor = await open(rel);
      try {
        select(editor, 1, 2, 2, 10);
        await vscode.commands.executeCommand("rubyTemplates.extractPartial");
        assert.strictEqual(
          editor.document.getText(),
          '<div>\n  <%= render "users/erb_part" %>\n</div>\n',
        );
        assert.strictEqual(
          fs.readFileSync(
            path.join(app, "app/views/users/_erb_part.html.erb"),
            "utf8",
          ),
          "<p>a</p>\n<p>b</p>\n",
        );
      } finally {
        await revert(editor);
      }
    });
  },

  async "extract Haml partial"() {
    const rel = "app/views/users/extract.html.haml";
    return answeringInputBox("haml_part", async () => {
      const editor = await open(rel);
      try {
        select(editor, 1, 4, 2, 4);
        await vscode.commands.executeCommand("rubyTemplates.extractPartial");
        assert.strictEqual(
          editor.document.getText(),
          '%div\n  = render "users/haml_part"\n',
        );
        assert.strictEqual(
          fs.readFileSync(
            path.join(app, "app/views/users/_haml_part.html.haml"),
            "utf8",
          ),
          "%p a\n%p b\n",
        );
      } finally {
        await revert(editor);
      }
    });
  },

  async "extract Slim partial"() {
    const rel = "app/views/users/extract.html.slim";
    return answeringInputBox("slim_part", async () => {
      const editor = await open(rel);
      try {
        select(editor, 1, 0, 2, 5);
        await vscode.commands.executeCommand(
          "rubyTemplates.extractSlimPartial",
        );
        assert.strictEqual(
          editor.document.getText(),
          'div\n  = render "slim_part"\n',
        );
        assert.strictEqual(
          fs.readFileSync(
            path.join(app, "app/views/users/_slim_part.html.slim"),
            "utf8",
          ),
          "p a\np b\n",
        );
      } finally {
        await revert(editor);
      }
    });
  },

  async "haml-lint diagnostics"() {
    const bin = gemTool("haml-lint");
    if (!bin) return "skipped: haml-lint not installed";
    const global = vscode.ConfigurationTarget.Global;
    await settings().update("commands.hamlLint", bin, global);
    try {
      const editor = await open("app/views/users/lint.html.haml");
      const found = await until(
        "haml-lint offenses",
        () => {
          const found = vscode.languages
            .getDiagnostics(editor.document.uri)
            .filter((d) => d.source === "haml-lint");
          return found.length && found;
        },
        60000,
      );
      const trailing = found.find((d) => d.code === "TrailingWhitespace");
      assert.ok(trailing, found.map((d) => d.code).join());
      assert.strictEqual(trailing.range.start.line, 1);
    } finally {
      await settings().update("commands.hamlLint", undefined, global);
    }
  },

  async "slim-lint diagnostics on save"() {
    const bin = gemTool("slim-lint");
    if (!bin) return "skipped: slim-lint not installed";
    const global = vscode.ConfigurationTarget.Global;
    await settings().update("commands.slimLint", bin, global);
    await settings().update("slimLint.enabled", true, global);
    const editor = await open("app/views/users/lint.html.slim");
    try {
      // slim-lint runs on save; a clean document wouldn't fire it.
      await editor.edit((eb) => eb.insert(new vscode.Position(2, 0), "\n"));
      await editor.document.save();
      const found = await until(
        "slim-lint offenses",
        () => {
          const found = vscode.languages
            .getDiagnostics(editor.document.uri)
            .filter((d) => d.source === "slim-lint");
          return found.length && found;
        },
        60000,
      );
      assert.strictEqual(found[0].code, "TrailingWhitespace");
      assert.strictEqual(found[0].range.start.line, 0);
    } finally {
      fs.writeFileSync(editor.document.uri.fsPath, "p \n  = foo\n");
      await settings().update("slimLint.enabled", undefined, global);
      await settings().update("commands.slimLint", undefined, global);
      await revert(editor);
    }
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
