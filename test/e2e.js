// Minimal LSP smoke test against a fake Rails tree: definition, hover, completion, diagnostics, symbols, rename.
import cp from "child_process";
import fs from "fs";
import path from "path";
import os from "os";
import { fileURLToPath, pathToFileURL } from "url";
import { isDeepStrictEqual } from "util";
// CI builds per target (`server/target/<triple>/release`) and passes the binary in.
const bin =
  process.env.RUBY_TEMPLATES_LS ??
  path.join(
    import.meta.dirname,
    "..",
    "server",
    "target",
    "release",
    "ruby-templates-ls",
  );
// Node escapes `~` (8.3 names such as RUNNER~1 on CI); the server, like VS Code, leaves it as is.
const fileUri = (p) => pathToFileURL(p).href.replaceAll("%7E", "~");
// macOS puts tmpdir under the /var symlink; one spelling of each path keeps built and returned URIs equal.
const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "rt-e2e-")));
const views = path.join(root, "app", "views");
fs.writeFileSync(
  path.join(root, "Gemfile.lock"),
  "GEM\n  specs:\n    actionview (7.1.3)\n",
);
fs.mkdirSync(path.join(views, "users"), { recursive: true });
fs.mkdirSync(path.join(views, "shared"), { recursive: true });
fs.mkdirSync(path.join(views, "strict"), { recursive: true });
fs.writeFileSync(
  path.join(views, "strict", "_row.html.erb"),
  "<%# locals: (a:, b: 1) %>\n",
);
fs.writeFileSync(
  path.join(views, "strict", "_pick.html.erb"),
  "<%# locals: (html_key:) %>\n",
);
fs.writeFileSync(
  path.join(views, "strict", "_pick.json.erb"),
  "<%# locals: (json_key:) %>\n",
);
for (const name of ["_dual.html.erb", "_dual.html+phone.erb"]) {
  fs.writeFileSync(path.join(views, "strict", name), "<%# locals: (u:) %>\n");
}
fs.writeFileSync(
  path.join(views, "strict", "list.html.erb"),
  '<%= render "strict/row", a: 1 %>\n',
);
fs.mkdirSync(path.join(root, "app", "components"), { recursive: true });
fs.writeFileSync(
  path.join(views, "users", "_card.html.slim"),
  ".card = user.name\n",
);
fs.writeFileSync(
  path.join(views, "shared", "_footer.html.erb"),
  "<footer></footer>\n",
);
fs.mkdirSync(path.join(root, "app", "controllers"), { recursive: true });
fs.writeFileSync(
  path.join(root, "app", "controllers", "application_controller.rb"),
  "class ApplicationController < ActionController::Base\n  before_action :set_current_user\n  before_action :set_admin, only: :show\n\n  def set_current_user\n    @current_user = User.first\n  end\n\n  def set_admin\n    @admin = true\n  end\nend\n",
);
fs.writeFileSync(
  path.join(root, "app", "components", "badge_component.rb"),
  "class BadgeComponent\n  renders_one :icon\n  renders_many :links\nend\n",
);
fs.writeFileSync(path.join(views, "users", "_user.html.erb"), "<p></p>\n");
fs.writeFileSync(
  path.join(views, "users", "_user.json.jbuilder"),
  "json.name user.name\n",
);
fs.mkdirSync(path.join(root, "app", "javascript", "controllers"), {
  recursive: true,
});
fs.writeFileSync(
  path.join(root, "app", "javascript", "controllers", "hello_controller.js"),
  'export default class extends Controller {\n  static targets = ["name"]\n  static values = { greetingText: String }\n  greet() {}\n}\n',
);
fs.mkdirSync(path.join(root, "app", "controllers"), { recursive: true });
fs.mkdirSync(path.join(root, "app", "helpers"), { recursive: true });
fs.mkdirSync(path.join(views, "layouts"), { recursive: true });
fs.writeFileSync(path.join(views, "layouts", "admin.html.erb"), "");
fs.writeFileSync(
  path.join(root, "app", "helpers", "users_helper.rb"),
  "module UsersHelper\n  def format_price(x)\n  end\nend\n",
);
const controllerText =
  'class UsersController < ApplicationController\n  layout "admin"\n\n  def index\n    render :index\n  end\n  before_action :set_menu\n\n  def set_menu\n    @menu = Menu.first\n  end\nend\n';
const controllerFile = path.join(
  root,
  "app",
  "controllers",
  "users_controller.rb",
);
fs.writeFileSync(controllerFile, controllerText);
fs.mkdirSync(path.join(root, "app", "mailers"), { recursive: true });
fs.writeFileSync(
  path.join(root, "app", "mailers", "user_mailer.rb"),
  'class UserMailer\n  def subject = t("users.index.title")\nend\n',
);
fs.mkdirSync(path.join(root, "app", "assets", "images"), { recursive: true });
fs.writeFileSync(path.join(root, "app", "assets", "images", "logo.png"), "");
fs.mkdirSync(path.join(root, "app", "assets", "stylesheets"), {
  recursive: true,
});
fs.writeFileSync(
  path.join(root, "app", "assets", "stylesheets", "admin.css"),
  "",
);
fs.mkdirSync(path.join(root, "db"), { recursive: true });
fs.writeFileSync(
  path.join(root, "db", "schema.rb"),
  'create_table "users", force: :cascade do |t|\n  t.string "name"\n  t.string "nickname"\n  t.string "email"\nend\ncreate_table "reports", force: :cascade do |t|\n  t.string "title", null: false\nend\n',
);
fs.mkdirSync(path.join(root, "app", "models"), { recursive: true });
fs.writeFileSync(
  path.join(root, "app", "models", "report.rb"),
  "class Report < ApplicationRecord\nend\n",
);
fs.writeFileSync(
  path.join(root, "app", "controllers", "reports_controller.rb"),
  "class ReportsController < ApplicationController\n  def index\n    @reports = Report.order(:title)\n  end\nend\n",
);
fs.mkdirSync(path.join(root, "config", "locales"), { recursive: true });
fs.writeFileSync(
  path.join(root, "config", "locales", "en.yml"),
  "en:\n  users:\n    index:\n      title: Users\n",
);
fs.writeFileSync(
  path.join(root, "config", "locales", "uk.yml"),
  "uk:\n  users:\n    index:\n      title: Користувачі\n",
);
fs.writeFileSync(
  path.join(root, "app", "components", "badge_component.html.haml"),
  "%span.badge= content\n",
);
const file = path.join(views, "users", "index.html.erb");
const text =
  '<%= render "card", user: u %>\n<%= render "shared/footer" %>\n<%= render BadgeComponent.new %>\n<%= render "sh\n<%= f.text_\n<%= link_to\n<%= render "shared/missing" %>\n<% content_for :aside do %>\n<nav>\n<%= render "card" %>\n</nav>\n<% end %>\n<%= render template: "us\n<%= link_to "x",\n  link_ %>\n<%= render @users %>\n<div data-controller="hello"></div>\n<%= image_tag "logo.png", alt: "Logo" %>\n<%= t(".title") %>\n';
fs.writeFileSync(file, text);
const uri = fileUri(file);
const hamlFile = path.join(views, "users", "show.html.haml");
const hamlText = [
  "- content_for :aside do",
  '  %nav= render "card"',
  "%section#main",
  '  = render "shared/footer"',
  "  = render BadgeComponent.new",
  "  = link_",
  "  %p see (link_",
  "  %sec",
  "  %a{hr}",
  "  :jav",
  "  &= x",
  '  = render "shared/missing"',
  "  %p text",
  "    %b nested",
  "  :css",
  "    .btn{",
  "  :ruby",
  "    x = link_",
  "",
].join("\n");
fs.writeFileSync(hamlFile, hamlText);
const hamlUri = fileUri(hamlFile);
const slimFile = path.join(views, "users", "show.html.slim");
const slimText =
  'mjml:\n  x\n- else\nbr\n  p\n- form_with model: @u do |f|\n  = f.text_field :x\n= link_to("x"\n';
fs.writeFileSync(slimFile, slimText);
const slimUri = fileUri(slimFile);
const outlineFile = path.join(views, "users", "outline.html.slim");
const outlineText =
  'main#page\n  = content_for :aside do\n    nav = render "card"\n  a(href="/"\n    class="x")\n    | Home\n\njavascript:\n  go();\n';
fs.writeFileSync(outlineFile, outlineText);
const outlineUri = fileUri(outlineFile);
// Unsaved text differs from disk; VS Code sends `+` percent-encoded.
const phoneFile = path.join(views, "users", "edit.html+phone.slim");
fs.writeFileSync(phoneFile, '\n\n= render "card"\n');
const phoneText = '= render "card"\n';
const phoneUri = fileUri(phoneFile).replace("+", "%2B");

const enginePartial = path.join(
  root,
  "engines",
  "blog",
  "app",
  "views",
  "blog",
  "_post.html.erb",
);
fs.mkdirSync(path.dirname(enginePartial), { recursive: true });
fs.writeFileSync(enginePartial, "<article></article>\n");
const engineUri = fileUri(path.join(views, "users", "engine.html.erb"));

const startServer = () => {
  const proc = cp.spawn(bin);
  let id = 0,
    buf = Buffer.alloc(0);
  const pending = new Map();
  const notes = [];
  proc.stdout.on("data", (d) => {
    buf = Buffer.concat([buf, d]);
    for (;;) {
      const m = buf.toString("utf8").match(/^Content-Length: (\d+)\r\n\r\n/);
      if (!m) return;
      const len = +m[1],
        start = m[0].length;
      if (buf.length < start + len) return;
      const msg = JSON.parse(buf.slice(start, start + len).toString("utf8"));
      buf = buf.slice(start + len);
      if (msg.id !== undefined && pending.has(msg.id)) {
        pending.get(msg.id)(msg);
        pending.delete(msg.id);
      } else notes.push(msg);
    }
  });
  const send = (method, params, isNote) =>
    new Promise((res) => {
      const msg = { jsonrpc: "2.0", method, params };
      if (!isNote) {
        msg.id = ++id;
        pending.set(msg.id, res);
      }
      const s = JSON.stringify(msg);
      proc.stdin.write(`Content-Length: ${Buffer.byteLength(s)}\r\n\r\n${s}`);
      if (isNote) res();
    });
  return { send, notes };
};
const { send, notes } = startServer();
const rel = (p) => p.replace(fileUri(root), "");
let failures = 0;
const check = (label, actual, expected) => {
  const ok = isDeepStrictEqual(actual, expected);
  if (!ok) failures++;
  console.log(`${ok ? "ok  " : "FAIL"} ${label.padEnd(24)} →`, actual);
  if (!ok) console.log("     expected", expected);
};
(async () => {
  const init = await send("initialize", {
    processId: null,
    rootUri: fileUri(root),
    capabilities: {},
    initializationOptions: {
      slim: { extraEngines: ["mjml"], disabledDiagnostics: ["explicit-end"] },
    },
  });
  await send("initialized", {}, true);
  await send(
    "textDocument/didOpen",
    { textDocument: { uri, languageId: "erb", version: 1, text } },
    true,
  );
  const req = (method, u, line, character) =>
    send(method, { textDocument: { uri: u }, position: { line, character } });
  const targets = (r) => (r.result || []).map((l) => rel(l.uri));
  const labels = (r) => (r.result || []).map((i) => i.label);
  const firstLine = (r) => r.result?.contents.value.split("\n")[0] ?? null;
  const tree = (s) =>
    s.children ? `${s.name} [${s.children.map(tree).join(", ")}]` : s.name;
  const component = [
    "/app/components/badge_component.rb",
    "/app/components/badge_component.html.haml",
  ];

  check(
    'definition "card"',
    targets(await req("textDocument/definition", uri, 0, 14)),
    ["/app/views/users/_card.html.slim"],
  );
  check(
    "definition shared/footer",
    targets(await req("textDocument/definition", uri, 1, 16)),
    ["/app/views/shared/_footer.html.erb"],
  );
  check(
    "definition component",
    targets(await req("textDocument/definition", uri, 2, 14)),
    component,
  );
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: engineUri,
        languageId: "erb",
        version: 1,
        text: '<%= render "blog/post" %>\n',
      },
    },
    true,
  );
  check(
    "definition: render in an in-repo engine",
    targets(await req("textDocument/definition", engineUri, 0, 14)),
    ["/engines/blog/app/views/blog/_post.html.erb"],
  );
  const liquidUri = fileUri(path.join(views, "users", "card.liquid"));
  const liquidHelperUri = fileUri(path.join(views, "users", "helper.liquid"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: liquidUri,
        languageId: "liquid",
        version: 1,
        text: '{% render "card" %}\n',
      },
    },
    true,
  );
  check(
    "definition: liquid render is not a partial",
    targets(await req("textDocument/definition", liquidUri, 0, 14)),
    [],
  );
  check(
    "document links: none in liquid",
    (
      await send("textDocument/documentLink", {
        textDocument: { uri: liquidUri },
      })
    ).result ?? null,
    null,
  );
  check(
    "hover: none on a liquid render",
    (await req("textDocument/hover", liquidUri, 0, 14)).result ?? null,
    null,
  );
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: liquidHelperUri,
        languageId: "liquid",
        version: 1,
        text: "{{ link_to }}\n",
      },
    },
    true,
  );
  check(
    "hover: none on a helper in liquid",
    (await req("textDocument/hover", liquidHelperUri, 0, 5)).result ?? null,
    null,
  );
  check(
    "hover",
    firstLine(await req("textDocument/hover", uri, 0, 14)),
    "**Partial** `card`",
  );
  check(
    'completion render "sh',
    labels(await req("textDocument/completion", uri, 3, 14)),
    ["shared/footer"],
  );
  check(
    "completion f.text_",
    labels(await req("textDocument/completion", uri, 4, 11)),
    ["text_field", "text_area"],
  );
  check(
    "completion link_to",
    labels(await req("textDocument/completion", uri, 5, 11)),
    ["link_to"],
  );
  const calledHelper = {};
  for (const [lang, text] of Object.entries({
    erb: "<%= link_to user.name, edit_user_path(user) %>\n<%= link_to %>\n",
    haml: '= link_to "Users", users_path\n= link_to\n',
    slim: '= link_to("Users", users_path)\n= link_to\n',
  })) {
    const calledUri = fileUri(path.join(views, "users", `called.html.${lang}`));
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: calledUri, languageId: lang, version: 1, text } },
      true,
    );
    const insert = async (line, character) =>
      (
        await req("textDocument/completion", calledUri, line, character)
      ).result?.find((i) => i.label === "link_to")?.insertText ?? "name";
    const at = text.indexOf("link_to") + "link_to".length;
    calledHelper[lang] = [await insert(0, at - 3), await insert(1, at)];
  }
  const snippet = 'link_to "${1:text}", ${2:path}';
  check("helper completion on a call with arguments", calledHelper, {
    erb: ["name", snippet],
    haml: ["name", snippet],
    slim: ["name", snippet],
  });
  const blockDefinitions = {};
  for (const [lang, text] of Object.entries({
    erb: "<% @users.each do |user| %>\n  <p><%= user.name %></p>\n<% end %>\n",
    haml: "- @users.each do |user|\n  %p= user.name\n",
    slim: "- @users.each do |user|\n  p = user.name\n",
  })) {
    const blockUri = fileUri(path.join(views, "users", `block.html.${lang}`));
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: blockUri, languageId: lang, version: 1, text } },
      true,
    );
    const read = text.split("\n")[1].indexOf("user.name") + 1;
    const r = (await req("textDocument/definition", blockUri, 1, read)).result;
    blockDefinitions[lang] = r
      ? `${r.range.start.line}:${r.range.start.character}-${r.range.end.character}`
      : null;
  }
  const assignedUri = fileUri(path.join(views, "users", "_assigned.html.erb"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: assignedUri,
        languageId: "erb",
        version: 1,
        text: "<%# locals: (user: nil) %>\n<% user ||= 1 %>\n<% [1].each do |user| %><%= user %><% end %><%= user %>\n",
      },
    },
    true,
  );
  const assignedAt = async (character) => {
    const r = (await req("textDocument/definition", assignedUri, 2, character))
      .result;
    const l = Array.isArray(r) ? r[0] : r;
    return l ? `${l.range.start.line}:${l.range.start.character}` : null;
  };
  check(
    "definition on a strict local the partial assigns",
    [await assignedAt(48), await assignedAt(28)],
    ["0:0", "2:16"],
  );
  const lookupUri = fileUri(path.join(views, "users", "lookup.html.erb"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: lookupUri,
        languageId: "erb",
        version: 1,
        text: '<%= render partial: "user", formats: [:json] %>\n<%= render "user", formats: [:json] %>\n<%= render partial: "user",\n  formats: [:json] %>\n<%= render formats: [:json], partial: "user" %>\n<%= render partial: "users/ghost",\n  formats: [:json] %>\n<%= render formats: [:json],\n  partial: "user" %>\n',
      },
    },
    true,
  );
  check(
    "render formats: picks the file",
    [
      targets(await req("textDocument/definition", lookupUri, 0, 22)),
      targets(await req("textDocument/definition", lookupUri, 1, 13)),
    ],
    [
      ["/app/views/users/_user.json.jbuilder"],
      ["/app/views/users/_user.html.erb"],
    ],
  );
  check(
    "render formats: on a later line picks the file",
    targets(await req("textDocument/definition", lookupUri, 2, 22)),
    ["/app/views/users/_user.json.jbuilder"],
  );
  check(
    "render formats: before the name picks the file",
    targets(await req("textDocument/definition", lookupUri, 4, 40)),
    ["/app/views/users/_user.json.jbuilder"],
  );
  check(
    "render formats: on a line before the name picks the file",
    targets(await req("textDocument/definition", lookupUri, 8, 14)),
    ["/app/views/users/_user.json.jbuilder"],
  );
  const ghost = {
    start: { line: 5, character: 21 },
    end: { line: 5, character: 32 },
  };
  const ghostActions = await send("textDocument/codeAction", {
    textDocument: { uri: lookupUri },
    range: ghost,
    context: {
      diagnostics: [
        { range: ghost, code: "Rails/MissingPartial", message: "not found" },
      ],
    },
  });
  check(
    "codeAction create partial in the lookup's format",
    ghostActions.result?.map((a) => a.title),
    ["Create partial `users/_ghost.json.erb`"],
  );
  const untitled = {};
  for (const [lang, text] of Object.entries({
    erb: '<div>\n<%= render "card" %>\n',
    haml: '%div\n  = render "card"\n- end\n',
    slim: 'div\n  = render "card"\n  p(class="x"\n',
  })) {
    const u = `untitled:Untitled-${lang}`;
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: u, languageId: lang, version: 1, text } },
      true,
    );
    const at = {
      textDocument: { uri: u },
      position: { line: 1, character: 14 },
    };
    const replies = [
      await send("textDocument/documentSymbol", { textDocument: { uri: u } }),
      await send("textDocument/foldingRange", { textDocument: { uri: u } }),
      await send("textDocument/definition", at),
      await send("textDocument/hover", at),
      await send("textDocument/references", {
        ...at,
        context: { includeDeclaration: true },
      }),
      await send("textDocument/prepareRename", at),
      await send("textDocument/documentLink", { textDocument: { uri: u } }),
    ];
    const published = () =>
      notes
        .filter(
          (n) =>
            n.method === "textDocument/publishDiagnostics" &&
            n.params.uri === u,
        )
        .flatMap((n) => n.params.diagnostics.map((d) => d.code));
    for (let waited = 0; waited < 2000 && !published().length; waited += 50) {
      await new Promise((r) => setTimeout(r, 50));
    }
    untitled[lang] = {
      errors: replies.filter((r) => r.error).length,
      symbols: replies[0].result?.length > 0,
      definition: replies[2].result ?? null,
      diagnostics: published().length > 0,
      unclosed: published().includes("Html/UnclosedElement"),
    };
  }
  const untitledOk = {
    errors: 0,
    symbols: true,
    definition: null,
    diagnostics: true,
    unclosed: false,
  };
  check("untitled documents", untitled, {
    erb: { ...untitledOk, unclosed: true },
    haml: untitledOk,
    slim: untitledOk,
  });
  const erbUntitled = "untitled:Untitled-fmt";
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: erbUntitled,
        languageId: "erb",
        version: 1,
        text: "<div>\n<p>hi</p>\n</div>\n",
      },
    },
    true,
  );
  const untitledEdits =
    (
      await send("textDocument/formatting", {
        textDocument: { uri: erbUntitled },
        options: { tabSize: 2, insertSpaces: true },
      })
    ).result || [];
  check(
    "untitled erb formatting",
    untitledEdits[0]?.newText,
    "<div>\n  <p>hi</p>\n</div>\n",
  );
  const jsUri = "untitled:Untitled-erb-js";
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: jsUri,
        languageId: "erb-js",
        version: 1,
        text: "<div>\n<% end %>\n",
      },
    },
    true,
  );
  const jsCodes = () =>
    notes
      .filter(
        (n) =>
          n.method === "textDocument/publishDiagnostics" &&
          n.params.uri === jsUri,
      )
      .flatMap((n) => n.params.diagnostics.map((d) => d.code));
  // The stray `end` is a non-HTML diagnostic, so its arrival proves the server has published.
  for (let waited = 0; waited < 2000 && !jsCodes().length; waited += 50) {
    await new Promise((r) => setTimeout(r, 50));
  }
  check("untitled erb-js gets no HTML diagnostics", jsCodes(), ["Syntax/Ruby"]);
  // The latest diagnostics as `line:code` for a document opened with `languageId`; empty when none are published.
  const diagnosticsFor = async (uri, languageId, text) => {
    await send(
      "textDocument/didOpen",
      { textDocument: { uri, languageId, version: 1, text } },
      true,
    );
    const published = () =>
      notes.filter(
        (n) =>
          n.method === "textDocument/publishDiagnostics" &&
          n.params.uri === uri,
      );
    for (let waited = 0; waited < 2000 && !published().length; waited += 50) {
      await new Promise((r) => setTimeout(r, 50));
    }
    return (published().at(-1)?.params.diagnostics ?? []).map(
      (d) => `${d.range.start.line}:${d.code}`,
    );
  };
  const mixed = "% if x\n  a\n<% else %>\n  b\n% end\n";
  check(
    "erb-percent: % lines and <% else %> form one block",
    await diagnosticsFor("file:///v/mixed.html.erb", "erb-percent", mixed),
    [],
  );
  check(
    "erb: the same text has a stray else",
    (await diagnosticsFor("file:///v/plain.html.erb", "erb", mixed)).length > 0,
    true,
  );
  check(
    "erb-percent: a syntax error in a % line is reported on it",
    [
      ...new Set(
        await diagnosticsFor(
          "file:///v/bad.html.erb",
          "erb-percent",
          "a\n% x = = 1\nb\n",
        ),
      ),
    ],
    ["1:Syntax/Ruby"],
  );
  const stimulusAttributes = {};
  for (const [lang, text] of Object.entries({
    erb: '<div class="x"\n     data-controller="hello" data-hel',
    haml: '%div(data-controller="hello" data-hel',
    haml_hash: '%div{ data: { controller: "hello", hel',
    slim: 'div data-controller="hello" data-hel',
    erb_tag: '<div data-controller="hello" <%= attrs %> data-hel',
    haml_lines: '%div(data-controller="hello"\n     data-hel',
    haml_rocket: '%div{ "data-controller" => "hello", data',
    slim_lines: 'div(\n  data-controller="hello"\n  data-hel',
  })) {
    const ext = lang.split("_")[0];
    const attrUri = fileUri(
      path.join(views, "users", `stimulus_${lang}.html.${ext}`),
    );
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: attrUri, languageId: ext, version: 1, text } },
      true,
    );
    const lines = text.split("\n");
    const last = lines.length - 1;
    const r = (
      await req("textDocument/completion", attrUri, last, lines[last].length)
    ).result;
    stimulusAttributes[lang] = (r ?? [])
      .map((i) => i.label)
      .filter((l) => l.includes("hello"));
  }
  check(
    "stimulus attributes of the element's controllers",
    stimulusAttributes,
    {
      erb: ["data-hello-target", "data-hello-greeting-text-value"],
      haml: ["data-hello-target", "data-hello-greeting-text-value"],
      haml_hash: ["hello_target", "hello_greeting_text_value"],
      slim: ["data-hello-target", "data-hello-greeting-text-value"],
      erb_tag: ["data-hello-target", "data-hello-greeting-text-value"],
      haml_lines: ["data-hello-target", "data-hello-greeting-text-value"],
      haml_rocket: ["data-hello-target", "data-hello-greeting-text-value"],
      slim_lines: ["data-hello-target", "data-hello-greeting-text-value"],
    },
  );
  const columnChecks = { hover: {}, definition: {}, tokens: {} };
  for (const [lang, text] of Object.entries({
    erb: "<% @reports.each do |report| %>\n  <p><%= report.title %></p>\n<% end %>\n",
    haml: "- @reports.each do |report|\n  %p= report.title\n",
    slim: "- @reports.each do |report|\n  p = report.title\n",
  })) {
    const columnUri = fileUri(
      path.join(views, "reports", `index.html.${lang}`),
    );
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: columnUri, languageId: lang, version: 1, text } },
      true,
    );
    const title = text.split("\n")[1].indexOf("report.title") + 8;
    columnChecks.hover[lang] = (
      await req("textDocument/hover", columnUri, 1, title)
    ).result?.contents.value;
    columnChecks.tokens[lang] = (
      await send("textDocument/semanticTokens/full", {
        textDocument: { uri: columnUri },
      })
    ).result?.data;
    const found = (await req("textDocument/definition", columnUri, 1, title))
      .result;
    columnChecks.definition[lang] = found
      ? `${rel(found.uri)}:${found.range.start.line + 1}`
      : null;
  }
  const reportsHover = "`reports.title` — string, null: false";
  check("hover on a model column", columnChecks.hover, {
    erb: reportsHover,
    haml: reportsHover,
    slim: reportsHover,
  });
  // [line, column, length, type, modifiers] deltas: `report` twice, as a variable (type 1).
  check("semantic tokens for block variables", columnChecks.tokens, {
    erb: [0, 21, 6, 1, 0, 1, 9, 6, 1, 0],
    haml: [0, 20, 6, 1, 0, 1, 6, 6, 1, 0],
    slim: [0, 20, 6, 1, 0, 1, 6, 6, 1, 0],
  });
  const schemaTarget = "/db/schema.rb:7";
  check("definition on a model column", columnChecks.definition, {
    erb: schemaTarget,
    haml: schemaTarget,
    slim: schemaTarget,
  });
  check("definition on a block variable", blockDefinitions, {
    erb: "0:19-23",
    haml: "0:18-22",
    slim: "0:18-22",
  });
  check(
    "definition render @users",
    targets(await req("textDocument/definition", uri, 15, 12)),
    ["/app/views/users/_user.html.erb"],
  );
  check(
    "definition data-controller",
    targets(await req("textDocument/definition", uri, 16, 24)),
    ["/app/javascript/controllers/hello_controller.js"],
  );
  check(
    "definition image_tag",
    targets(await req("textDocument/definition", uri, 17, 18)),
    ["/app/assets/images/logo.png"],
  );
  check(
    'definition t(".title")',
    (await req("textDocument/definition", uri, 18, 9)).result?.map(
      (l) => `${rel(l.uri)}:${l.range.start.line + 1}`,
    ),
    ["/config/locales/en.yml:4", "/config/locales/uk.yml:4"],
  );
  check(
    'hover t(".title")',
    (await req("textDocument/hover", uri, 18, 9)).result?.contents.value
      .split("\n")
      .slice(2)
      .join(" "),
    "- **en**: `Users` - **uk**: `Користувачі`",
  );
  check(
    'completion t(".',
    labels(await req("textDocument/completion", uri, 18, 8)),
    [".title"],
  );
  check(
    'completion template: "us',
    labels(await req("textDocument/completion", uri, 12, 24)),
    ["users/edit", "users/index", "users/outline", "users/show"],
  );
  check(
    "completion 2nd tag line",
    labels(await req("textDocument/completion", uri, 14, 7)),
    ["link_to"],
  );
  check(
    "hover <%=",
    (await req("textDocument/hover", uri, 0, 2)).result?.contents.value,
    "**`<%=`** outputs the value, HTML-escaped unless it is `html_safe`.",
  );
  check(
    "hover link_to",
    (await req("textDocument/hover", uri, 5, 6)).result?.contents.value
      .split("\n")
      .pop(),
    "[API docs](https://api.rubyonrails.org/classes/ActionView/Helpers/UrlHelper.html#method-i-link_to)",
  );
  // Hover right after a multibyte char once killed the server (byte slice inside `’`).
  const quoteUri = fileUri(path.join(views, "users", "quote.html.erb"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: quoteUri,
        languageId: "erb",
        version: 1,
        text: '<p>Don’t</p>\n<%= link_to "Don’t", x %>\n',
      },
    },
    true,
  );
  check(
    "hover after ’ in text",
    (await req("textDocument/hover", quoteUri, 0, 7)).error ?? null,
    null,
  );
  check(
    "hover after ’ in code",
    (await req("textDocument/hover", quoteUri, 1, 18)).error ?? null,
    null,
  );
  check(
    "hover link_to after ’",
    (await req("textDocument/hover", quoteUri, 1, 6)).result?.contents.value
      .split("\n")
      .pop(),
    "[API docs](https://api.rubyonrails.org/classes/ActionView/Helpers/UrlHelper.html#method-i-link_to)",
  );
  const links = await send("textDocument/documentLink", {
    textDocument: { uri },
  });
  check(
    "documentLinks",
    (links.result || []).map((l) => rel(l.target)),
    [
      "/app/views/users/_card.html.slim",
      "/app/views/shared/_footer.html.erb",
      "/app/views/users/_card.html.slim",
      "/app/views/users/_user.html.erb",
      "/app/javascript/controllers/hello_controller.js",
      "/app/assets/images/logo.png",
    ],
  );
  check(
    "documentLink tooltips from the Rails root",
    (links.result || []).map((l) => l.tooltip).slice(-2),
    [
      "app/javascript/controllers/hello_controller.js",
      "app/assets/images/logo.png",
    ],
  );
  const folds = await send("textDocument/foldingRange", {
    textDocument: { uri },
  });
  check(
    "foldingRanges",
    (folds.result || []).map((f) => `${f.startLine}-${f.endLine}`),
    ["3-5", "7-10", "12-13", "8-9", "13-14"],
  );
  const syms = await send("textDocument/documentSymbol", {
    textDocument: { uri },
  });
  check("documentSymbols", (syms.result || []).map(tree), [
    'render "card"',
    'render "shared/footer"',
    'render "shared/missing"',
    'content_for :aside [nav [render "card"]]',
  ]);
  await new Promise((r) => setTimeout(r, 200));
  const diagnostics = (u) => {
    const diag = notes.find(
      (n) =>
        n.method === "textDocument/publishDiagnostics" && n.params.uri === u,
    );
    return diag
      ? diag.params.diagnostics.map(
          (d) => `${d.range.start.line + 1}: ${d.message}`,
        )
      : "none";
  };
  // Lines 4, 5, 6 and 13 are tags left open for the completion checks, each running on to the next `%>`: Prism's errors
  // for the method Rails builds from the template (`test/oracle/erb.rb` prints the same ones).
  check("diagnostics", diagnostics(uri), [
    "7: unexpected local variable or method, expecting end-of-input",
    "7: unexpected string literal, expecting end-of-input",
    "10: unexpected local variable or method, expecting end-of-input",
    "13: unexpected local variable or method, expecting end-of-input",
    "14: unexpected '<', ignoring it",
    "14: expected a matching `)`",
    "7: Missing partial shared/_missing",
  ]);

  const sources = (u) => [
    ...new Set(
      notes
        .find(
          (n) =>
            n.method === "textDocument/publishDiagnostics" &&
            n.params.uri === u,
        )
        ?.params.diagnostics.map((d) => d.source),
    ),
  ];
  check("erb diagnostics source", sources(uri), ["erb"]);

  const strictUri = fileUri(path.join(views, "strict", "show.html.erb"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: strictUri,
        languageId: "erb",
        version: 1,
        text: '<%= render "strict/row" %>\n',
      },
    },
    true,
  );
  await new Promise((r) => setTimeout(r, 400));
  check("strict locals under actionview 7.1", diagnostics(strictUri), [
    "1: missing local: :a",
  ]);
  const strictFixes = async (
    u,
    text,
    at = {
      start: { line: 0, character: 12 },
      end: { line: 0, character: 22 },
    },
    languageId = "erb",
  ) => {
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: u, languageId, version: 1, text } },
      true,
    );
    const { result } = await send("textDocument/codeAction", {
      textDocument: { uri: u },
      range: at,
      context: {
        diagnostics: [{ range: at, code: "Rails/StrictLocals", message: "" }],
      },
    });
    return result
      .filter((a) => a.kind === "quickfix")
      .map((a) => {
        const edits = Object.entries(a.edit.changes).map(([target, [e]]) => {
          return `${rel(target)} ${e.range.start.line}:${e.range.start.character}-${e.range.end.line === e.range.start.line ? "" : `${e.range.end.line}:`}${e.range.end.character} "${e.newText}"`;
        });
        return `${a.title} | ${edits.sort().join(" + ")}`;
      });
  };
  const extraUri = fileUri(path.join(views, "strict", "extra.html.erb"));
  const multiUri = fileUri(path.join(views, "strict", "multi.html.erb"));
  const pipesUri = fileUri(path.join(views, "strict", "pipes.html.haml"));
  check(
    "strict locals quick fixes on a haml pipe multiline",
    await strictFixes(
      pipesUri,
      '= render "strict/row", |\n    c: 2 |\n',
      { start: { line: 0, character: 10 }, end: { line: 0, character: 20 } },
      "haml",
    ),
    [
      'Pass `a` to the render | /app/views/strict/pipes.html.haml 1:8-8 ", a: nil"',
      'Remove `c` from the render | /app/views/strict/pipes.html.haml 0:21-1:8 ""',
      'Declare `c` in the locals of `_row.html.erb` | /app/views/strict/_row.html.erb 0:21-21 ", c: nil"',
    ],
  );
  check(
    "strict locals quick fixes offer an unknown key passed twice once",
    await strictFixes(
      fileUri(path.join(views, "strict", "twice.html.erb")),
      '<%= render "strict/row", c: 2, c: 3 %>\n',
    ),
    [
      'Pass `a` to the render | /app/views/strict/twice.html.erb 0:35-35 ", a: nil"',
      'Remove `c` from the render | /app/views/strict/twice.html.erb 0:23-29 ""',
      'Declare `c` in the locals of `_row.html.erb` | /app/views/strict/_row.html.erb 0:21-21 ", c: nil"',
    ],
  );
  check(
    "strict locals quick fixes on a call continued before its partial",
    await strictFixes(
      fileUri(path.join(views, "strict", "continued.html.erb")),
      '<%= render formats: [:html], locals: { c: 2 },\n  partial: "strict/row" %>\n',
      { start: { line: 1, character: 12 }, end: { line: 1, character: 22 } },
    ),
    [
      'Pass `a` to the render | /app/views/strict/continued.html.erb 0:43-43 ", a: nil"',
      'Remove `c` from the render | /app/views/strict/continued.html.erb 0:39-43 ""',
      'Declare `c` in the locals of `_row.html.erb` | /app/views/strict/_row.html.erb 0:21-21 ", c: nil"',
    ],
  );
  const dualUri = fileUri(path.join(views, "strict", "dual.html.erb"));
  const dualFixes = await strictFixes(
    dualUri,
    '<%= render "strict/dual", z: 1 %>\n',
  );
  await new Promise((r) => setTimeout(r, 400));
  check(
    "strict locals of two variants",
    [diagnostics(dualUri), dualFixes],
    [
      ["1: missing local: :u"],
      [
        'Pass `u` to the render | /app/views/strict/dual.html.erb 0:30-30 ", u: nil"',
        'Remove `z` from the render | /app/views/strict/dual.html.erb 0:24-30 ""',
        'Declare `z` in the locals of `_dual.html.erb` and 1 other | /app/views/strict/_dual.html%2Bphone.erb 0:15-15 ", z: nil" + /app/views/strict/_dual.html.erb 0:15-15 ", z: nil"',
      ],
    ],
  );
  check(
    "strict locals quick fixes",
    [
      await strictFixes(strictUri, '<%= render "strict/row" %>\n'),
      await strictFixes(extraUri, '<%= render "strict/row", a: 1, c: 2 %>\n'),
      await strictFixes(
        multiUri,
        '<p>h\u00e9llo \u65e5\u672c</p>\n<%\n  render "strict/row",\n    c: items\n      .first\n%>\n',
        {
          start: { line: 2, character: 10 },
          end: { line: 2, character: 20 },
        },
      ),
    ],
    [
      [
        'Pass `a` to the render | /app/views/strict/show.html.erb 0:23-23 ", a: nil"',
      ],
      [
        'Remove `c` from the render | /app/views/strict/extra.html.erb 0:29-35 ""',
        'Declare `c` in the locals of `_row.html.erb` | /app/views/strict/_row.html.erb 0:21-21 ", c: nil"',
      ],
      [
        'Pass `a` to the render | /app/views/strict/multi.html.erb 4:12-12 ", a: nil"',
        'Remove `c` from the render | /app/views/strict/multi.html.erb 2:21-4:12 ""',
        'Declare `c` in the locals of `_row.html.erb` | /app/views/strict/_row.html.erb 0:21-21 ", c: nil"',
      ],
    ],
  ); // An unsaved document counts as a caller in later checks.
  await send(
    "textDocument/didClose",
    { textDocument: { uri: extraUri } },
    true,
  );

  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: hamlUri,
        languageId: "haml",
        version: 1,
        text: hamlText,
      },
    },
    true,
  );
  const haml = (method, line, character) =>
    req(method, hamlUri, line, character);
  check(
    'haml definition "card"',
    targets(await haml("textDocument/definition", 1, 17)),
    ["/app/views/users/_card.html.slim"],
  );
  check(
    "haml definition comp.",
    targets(await haml("textDocument/definition", 4, 15)),
    component,
  );
  check(
    "haml hover render",
    firstLine(await haml("textDocument/hover", 3, 16)),
    "**Partial** `shared/footer`",
  );
  check(
    "haml hover &=",
    firstLine(await haml("textDocument/hover", 10, 2)),
    "**Haml** `&=`",
  );
  check(
    "haml completion link_",
    labels(await haml("textDocument/completion", 5, 9)),
    ["link_to"],
  );
  check(
    "haml completion text (",
    labels(await haml("textDocument/completion", 6, 15)),
    [],
  );
  check(
    "haml completion tag",
    labels(await haml("textDocument/completion", 7, 6)),
    ["section"],
  );
  check(
    "haml completion {hr",
    labels(await haml("textDocument/completion", 8, 7)),
    ["href", "hreflang"],
  );
  check(
    "haml completion :jav",
    labels(await haml("textDocument/completion", 9, 6)),
    ["javascript"],
  );
  check(
    "haml completion :css {",
    labels(await haml("textDocument/completion", 15, 9)),
    [],
  );
  check(
    "haml completion :ruby",
    labels(await haml("textDocument/completion", 17, 14)),
    ["link_to"],
  );
  const hamlSyms = await send("textDocument/documentSymbol", {
    textDocument: { uri: hamlUri },
  });
  check("haml documentSymbols", (hamlSyms.result || []).map(tree), [
    'content_for :aside [nav [render "card"]]',
    'section#main [render "shared/footer", render "shared/missing"]',
  ]);
  const hamlFolds = await send("textDocument/foldingRange", {
    textDocument: { uri: hamlUri },
  });
  check(
    "haml foldingRanges",
    (hamlFolds.result || []).map((f) => `${f.startLine}-${f.endLine}`).sort(),
    ["0-1", "12-13", "14-15", "16-17", "2-17"],
  );
  await new Promise((r) => setTimeout(r, 200));
  check("haml diagnostics", diagnostics(hamlUri), [
    "14: Illegal nesting: content can't be both given on the same line as %p and nested within it.",
    "12: Missing partial shared/_missing",
  ]);

  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: outlineUri,
        languageId: "slim",
        version: 1,
        text: outlineText,
      },
    },
    true,
  );
  const slimSyms = await send("textDocument/documentSymbol", {
    textDocument: { uri: outlineUri },
  });
  check("slim documentSymbols", (slimSyms.result || []).map(tree), [
    'main#page [content_for :aside [nav [render "card"]]]',
    "javascript:",
  ]);
  const slimFolds = await send("textDocument/foldingRange", {
    textDocument: { uri: outlineUri },
  });
  check(
    "slim foldingRanges",
    (slimFolds.result || []).map((f) => `${f.startLine}-${f.endLine}`),
    ["0-5", "1-2", "3-5", "7-8"],
  );
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: slimUri,
        languageId: "slim",
        version: 1,
        text: slimText,
      },
    },
    true,
  );
  check("haml diagnostics source", sources(hamlUri), ["haml"]);

  await new Promise((r) => setTimeout(r, 200));
  const slimDiag = notes.find(
    (n) =>
      n.method === "textDocument/publishDiagnostics" &&
      n.params.uri === slimUri,
  );
  check(
    "slim diagnostics",
    slimDiag
      ? slimDiag.params.diagnostics.map(
          (d) =>
            `${d.range.start.line + 1}:${d.range.start.character + 1} ${d.code}`,
        )
      : [],
    [
      "3:3 Syntax/Ruby",
      "4:1 Syntax/Ruby",
      "5:3 Html/VoidChildren",
      "8:1 Syntax/Ruby",
      "8:1 Syntax/Ruby",
    ],
  );
  check("slim diagnostics source", sources(slimUri), ["slim"]);

  // Haml's Ruby is checked once Haml accepts the template: an error in code, and a `rescue` inside an `if`.
  const hamlRubyUri = fileUri(path.join(views, "users", "ruby.html.haml"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: hamlRubyUri,
        languageId: "haml",
        version: 1,
        text: "%p\n  - x = [1 2]\n%a{href: x} t\n- if x\n  - rescue\n",
      },
    },
    true,
  );
  await new Promise((r) => setTimeout(r, 200));
  const hamlRuby = notes.find(
    (n) =>
      n.method === "textDocument/publishDiagnostics" &&
      n.params.uri === hamlRubyUri,
  );
  check(
    "haml ruby diagnostics",
    hamlRuby.params.diagnostics.map(
      (d) =>
        `${d.range.start.line + 1}:${d.range.start.character + 1} ${d.code}`,
    ),
    [
      "2:11 Syntax/Ruby",
      "5:5 Syntax/Ruby",
      "4:3 Syntax/Ruby",
      "5:3 Syntax/Ruby",
    ],
  );

  const editUri = fileUri(path.join(views, "users", "edit.html.slim"));
  const editText =
    'div\n    p= link_\n    a hr\n    jav\n    == raw x  \n    | text lin\n    p Hi #{link_\n    = render "card"\n';
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: editUri,
        languageId: "slim",
        version: 1,
        text: editText,
      },
    },
    true,
  );
  const expect = (label, ok, got) => {
    if (!ok) failures++;
    console.log(`${ok ? "ok  " : "FAIL"} ${label.padEnd(24)} →`, got);
  };
  const complete = async (line, character) =>
    (
      (
        await send("textDocument/completion", {
          textDocument: { uri: editUri },
          position: { line, character },
        })
      ).result || []
    ).map((i) => i.label);
  const helpers = await complete(1, 11);
  expect(
    "slim completion p= link_",
    helpers.includes("link_to"),
    helpers.slice(0, 3),
  );
  const attrs = await complete(2, 8);
  expect(
    "slim completion a hr",
    attrs.includes("href") && !attrs.includes("link_to"),
    attrs.filter((l) => l.startsWith("hr")),
  );
  const engines = await complete(3, 7);
  expect(
    "slim completion jav",
    engines.includes("javascript:"),
    engines.filter((l) => l.startsWith("jav")),
  );
  const inText = await complete(5, 14);
  expect("slim completion in text", inText.length === 0, inText);
  const openInterpolation = await complete(6, 16);
  expect(
    "slim completion #{link_",
    openInterpolation.includes("link_to"),
    openInterpolation.slice(0, 3),
  );
  const slimDef = (
    (
      await send("textDocument/definition", {
        textDocument: { uri: editUri },
        position: { line: 7, character: 15 },
      })
    ).result || []
  ).map((l) => rel(l.uri));
  expect(
    'slim definition "card"',
    slimDef.length === 1 && slimDef[0].endsWith("_card.html.slim"),
    slimDef,
  );
  const markerHover = await send("textDocument/hover", {
    textDocument: { uri: editUri },
    position: { line: 4, character: 5 },
  });
  const markerDoc = markerHover.result?.contents.value ?? "";
  expect(
    "slim hover ==",
    markerDoc.includes("unescaped output"),
    markerDoc.split("\n")[0],
  );
  const edits =
    (
      await send("textDocument/formatting", {
        textDocument: { uri: editUri },
        options: { tabSize: 2, insertSpaces: true },
      })
    ).result || [];
  const formatted =
    'div\n  p= link_\n  a hr\n  jav\n  == raw x\n  | text lin\n  p Hi #{link_\n  = render "card"\n';
  expect(
    "slim formatting",
    edits.length === 1 && edits[0].newText === formatted,
    JSON.stringify(edits[0]?.newText),
  );
  const tabs = await send("textDocument/formatting", {
    textDocument: { uri: editUri },
    options: { tabSize: 2, insertSpaces: false },
  });
  expect(
    "slim formatting, tabs",
    tabs.result?.length === 1 && tabs.result[0].newText === formatted,
    JSON.stringify(tabs.result?.[0]?.newText),
  );

  // Haml through the same formatter: the editor's width, the whole document or the lines of a range.
  const reindentUri = fileUri(path.join(views, "users", "reindent.html.haml"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: reindentUri,
        languageId: "haml",
        version: 1,
        // The formatter drops the empty lines at the end; range formatting still edits the lines above.
        text: "%div\n    %p\n        %b x\n    %i\n\n\n",
      },
    },
    true,
  );
  const hamlOptions = { tabSize: 2, insertSpaces: true };
  const hamlEdits =
    (
      await send("textDocument/formatting", {
        textDocument: { uri: reindentUri },
        options: hamlOptions,
      })
    ).result || [];
  expect(
    "haml formatting",
    hamlEdits[0]?.newText === "%div\n  %p\n    %b x\n  %i\n",
    JSON.stringify(hamlEdits[0]?.newText),
  );
  const rangeEdits =
    (
      await send("textDocument/rangeFormatting", {
        textDocument: { uri: reindentUri },
        options: hamlOptions,
        range: {
          start: { line: 3, character: 0 },
          end: { line: 3, character: 0 },
        },
      })
    ).result || [];
  expect(
    "haml range formatting",
    rangeEdits.length === 1 &&
      rangeEdits[0].range.start.line === 3 &&
      rangeEdits[0].newText === "  %i",
    JSON.stringify(rangeEdits),
  );
  // Without dynamic registration nothing is advertised: the client's own ERB and Haml providers decide when to ask.
  const staticFormatting = init.result.capabilities.documentFormattingProvider;
  expect(
    "no static formatter",
    staticFormatting === undefined,
    staticFormatting,
  );

  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: phoneUri,
        languageId: "slim",
        version: 1,
        text: phoneText,
      },
    },
    true,
  );
  const missing = {
    start: { line: 6, character: 12 },
    end: { line: 6, character: 26 },
  };
  const actions = await send("textDocument/codeAction", {
    textDocument: { uri },
    range: missing,
    context: {
      diagnostics: [
        { range: missing, code: "Rails/MissingPartial", message: "not found" },
      ],
    },
  });
  check(
    "codeAction create partial",
    actions.result?.map((a) => [
      a.title,
      rel(a.edit.documentChanges[0].uri),
      a.edit.documentChanges[0].kind,
    ]),
    [
      [
        "Create partial `shared/_missing.html.erb`",
        "/app/views/shared/_missing.html.erb",
        "create",
      ],
    ],
  );
  const symbols = await send("workspace/symbol", { query: "shfoot" });
  check(
    "workspace/symbol shfoot",
    symbols.result?.map((s) => `${s.name} ${s.containerName}`),
    ["shared/footer partial"],
  );
  const linked = await req("textDocument/linkedEditingRange", uri, 8, 2);
  check(
    "linkedEditingRange nav",
    linked.result?.ranges.map((r) => `${r.start.line}:${r.start.character}`),
    ["8:1", "10:2"],
  );
  const refs = await send("textDocument/references", {
    textDocument: { uri },
    position: { line: 1, character: 16 },
    context: { includeDeclaration: false },
  });
  check(
    "references shared/footer",
    refs.result?.map((l) => `${rel(l.uri)} ${l.range.start.line}`).sort(),
    ["/app/views/users/index.html.erb 1", "/app/views/users/show.html.haml 3"],
  );
  const keyRefs = await send("textDocument/references", {
    textDocument: { uri },
    position: { line: 18, character: 8 },
    context: { includeDeclaration: true },
  });
  check(
    "references i18n key",
    keyRefs.result?.map((l) => `${rel(l.uri)} ${l.range.start.line}`).sort(),
    [
      "/app/mailers/user_mailer.rb 1",
      "/app/views/users/index.html.erb 18",
      "/config/locales/en.yml 3",
      "/config/locales/uk.yml 3",
    ],
  );
  const controllerUri = fileUri(controllerFile);
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: controllerUri,
        languageId: "ruby",
        version: 1,
        text: controllerText,
      },
    },
    true,
  );
  check(
    "controller action → template",
    targets(await req("textDocument/definition", controllerUri, 3, 7)),
    ["/app/views/users/index.html.erb"],
  );
  const controllerLinks = await send("textDocument/documentLink", {
    textDocument: { uri: controllerUri },
  });
  check(
    "controller links: layout and render",
    controllerLinks.result?.map(
      (l) => `${l.range.start.line} ${rel(l.target)}`,
    ),
    [
      "1 /app/views/layouts/admin.html.erb",
      "4 /app/views/users/index.html.erb",
    ],
  );
  check(
    "controller gets no completion",
    (await req("textDocument/completion", controllerUri, 4, 10)).result,
    null,
  );
  const helperUri = fileUri(path.join(views, "users", "helper.html.erb"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: helperUri,
        languageId: "erb",
        version: 1,
        text: "<%= format_price(1) %> format_price\n",
      },
    },
    true,
  );
  check(
    "helper definition",
    [
      (await req("textDocument/definition", helperUri, 0, 6)).result?.map(
        (l) => `${rel(l.uri)}:${l.range.start.line}`,
      ),
      (await req("textDocument/definition", helperUri, 0, 26)).result,
    ],
    [["/app/helpers/users_helper.rb:1"], null],
  );
  check(
    "helper hover",
    [
      rel(
        (await req("textDocument/hover", helperUri, 0, 6)).result?.contents
          .value ?? "",
      ),
      (await req("textDocument/hover", helperUri, 0, 26)).result,
    ],
    [
      "```ruby\ndef format_price(x)\n```\n\n[app/helpers/users_helper.rb:2](/app/helpers/users_helper.rb#L2)",
      null,
    ],
  );
  const shadowUri = fileUri(path.join(views, "users", "shadow.html.erb"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: shadowUri,
        languageId: "erb",
        version: 1,
        text: '<% [1].each do |format_price| %>\n<%= format_price %>\n<% end %>\n<%= "format_price" %>\n',
      },
    },
    true,
  );
  check(
    "no helper hover on a block parameter, its use or in a string",
    [
      (await req("textDocument/hover", shadowUri, 0, 22)).result,
      (await req("textDocument/hover", shadowUri, 1, 6)).result,
      (await req("textDocument/hover", shadowUri, 3, 8)).result,
    ],
    [null, null, null],
  );
  const tagsUri = fileUri(path.join(views, "users", "tags.html.erb"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: tagsUri,
        languageId: "erb",
        version: 1,
        text: '<%= stylesheet_link_tag "application",\n                        "admin",\n                        "data-turbo-track": "reload" %>\n',
      },
    },
    true,
  );
  check(
    "definition on a stylesheet_link_tag continuation line",
    targets(await req("textDocument/definition", tagsUri, 1, 26)),
    ["/app/assets/stylesheets/admin.css"],
  );
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: helperUri, version: 2 },
      contentChanges: [{ text: "<%= @m %>\n<%= @menu.name %>\n<%= @tag %>\n" }],
    },
    true,
  );
  const cardUri = fileUri(path.join(views, "users", "_card.html.slim"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: cardUri,
        languageId: "slim",
        version: 1,
        text: "p = user.name\n",
      },
    },
    true,
  );
  const at = (r) =>
    r.result?.map(
      (l) => `${rel(l.uri)}:${l.range.start.line}:${l.range.start.character}`,
    );
  check(
    "ivars and locals",
    [
      (await req("textDocument/completion", helperUri, 0, 6)).result?.map(
        (i) => [i.label, i.textEdit.range.start.character],
      ),
      at(await req("textDocument/definition", helperUri, 1, 6)),
      at(await req("textDocument/definition", cardUri, 0, 5)),
      rel(
        (await req("textDocument/hover", helperUri, 1, 6)).result?.contents
          .value ?? "",
      ),
      (await req("textDocument/hover", helperUri, 2, 6)).result,
    ],
    [
      [["@menu", 4]],
      ["/app/controllers/users_controller.rb:9:0"],
      ["/app/views/users/index.html.erb:0:19"],
      "`@menu`\n\n- [app/controllers/users_controller.rb:10](/app/controllers/users_controller.rb#L10): `@menu = Menu.first`",
      null,
    ],
  );
  const navUri = fileUri(path.join(views, "shared", "_nav.html.erb"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: navUri,
        languageId: "erb",
        version: 1,
        text: "<%= @current_user %>\n<%= @admin %>\n",
      },
    },
    true,
  );
  check(
    "ivars of ApplicationController in a folder without a controller",
    [
      at(await req("textDocument/definition", navUri, 0, 6)),
      (await req("textDocument/hover", navUri, 1, 6)).result,
    ],
    [["/app/controllers/application_controller.rb:5:0"], null],
  );
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: helperUri, version: 3 },
      contentChanges: [
        { text: "<%= render BadgeComponent.new do |b| %>\n  <% b.with_l %>\n" },
      ],
    },
    true,
  );
  const badgeUri = fileUri(
    path.join(root, "app", "components", "badge_component.html.haml"),
  );
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: badgeUri,
        languageId: "haml",
        version: 1,
        text: "- if ic\n",
      },
    },
    true,
  );
  check(
    "ViewComponent slots",
    [
      labels(await req("textDocument/completion", helperUri, 1, 13)),
      labels(await req("textDocument/completion", badgeUri, 0, 7)),
    ],
    [
      ["with_link", "with_links"],
      ["icon", "icon?"],
    ],
  );
  await send(
    "rubyTemplates/routes",
    {
      root,
      routes: [
        {
          name: "users",
          verb: "GET",
          path: "/users(.:format)",
          action: "users#index",
          source: "config/routes.rb:2",
        },
        {
          name: "",
          verb: "POST",
          path: "/users(.:format)",
          action: "users#create",
        },
      ],
    },
    true,
  );
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: helperUri, version: 4 },
      contentChanges: [
        { text: '<%= link_to "x", users_p %>\n<%= users_path %>\n' },
      ],
    },
    true,
  );
  check(
    "route helpers: completion, hover, definition",
    [
      labels(await req("textDocument/completion", helperUri, 0, 24)),
      firstLine(await req("textDocument/hover", helperUri, 1, 6)),
      (await req("textDocument/definition", helperUri, 1, 6)).result?.map(
        (l) => `${rel(l.uri)}:${l.range.start.line}`,
      ),
    ],
    [["users_path"], "`users_path` → `/users`", ["/config/routes.rb:1"]],
  );
  const slimRubyUri = fileUri(path.join(views, "users", "_ruby.html.slim"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: slimRubyUri,
        languageId: "slim",
        version: 1,
        text: '/# locals: (user:)\n= @m\np #{users_p}\n= t("users.index.ti")\na href=us\n= t("users.index.title")\n',
      },
    },
    true,
  );
  check(
    "Slim: ivars, routes, i18n, locals",
    [
      labels(await req("textDocument/completion", slimRubyUri, 1, 4)),
      labels(await req("textDocument/completion", slimRubyUri, 2, 11)),
      labels(await req("textDocument/completion", slimRubyUri, 3, 19)),
      firstLine(await req("textDocument/hover", slimRubyUri, 5, 8)),
      labels(await req("textDocument/completion", slimRubyUri, 4, 9)),
    ],
    [
      ["@menu"],
      ["users_path"],
      ["users.index.title"],
      "`users.index.title`",
      ["user", "users_path", "users_url"],
    ],
  );
  const blockDocs = {
    erb: "<%# locals: (user:) %>\n<% @users.each do |user| %>\n<%= us %>\n<% end %>\n<%= us %>\n",
    slim: "- @users.each do |user|\n  = us\n= us\n",
    haml: "- @users.each do |user|\n  %p= us\n= us\n",
  };
  const blockVariables = {};
  for (const [lang, text] of Object.entries(blockDocs)) {
    const blockUri = fileUri(path.join(views, "users", `_each.html.${lang}`));
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: blockUri, languageId: lang, version: 1, text } },
      true,
    );
    const lines = text.split("\n");
    const at = async (line) =>
      (
        await req(
          "textDocument/completion",
          blockUri,
          line,
          lines[line].indexOf("us") + 2,
        )
      ).result
        ?.filter((i) => i.label === "user")
        .map((i) => i.detail);
    const typed = lines.flatMap((l, i) => (/\bus\b/.test(l) ? [i] : []));
    blockVariables[lang] = [await at(typed[0]), await at(typed[1])];
  }
  check("block variables in scope", blockVariables, {
    erb: [["block variable"], ["local (required)"]],
    slim: [["block variable"], []],
    haml: [["block variable"], []],
  });
  const keyDocs = {
    erb: '<%= render "strict/row", a: 1, ^ %>\n',
    slim: '= render "strict/row",\n    b: 2,\n    ^\n',
    haml: '= render("strict/row", ^, b: 2)\n',
    haml_pipes: '= render "strict/row", b: 2, |\n  ^ |\np x\n',
  };
  const renderKeys = {};
  for (const [lang, marked] of Object.entries(keyDocs)) {
    const ext = lang.split("_")[0];
    const keysUri = fileUri(
      path.join(views, "strict", `keys_${lang}.html.${ext}`),
    );
    const text = marked.replace("^", "");
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: keysUri, languageId: ext, version: 1, text } },
      true,
    );
    const before = marked.slice(0, marked.indexOf("^")).split("\n");
    const r = await req(
      "textDocument/completion",
      keysUri,
      before.length - 1,
      before.at(-1).length,
    );
    renderKeys[lang] = r.result?.map((i) => `${i.insertText} (${i.detail})`);
  }
  check("render locals keys", renderKeys, {
    erb: ["b:  (local = 1)"],
    slim: ["a:  (local (required))"],
    haml: ["a:  (local (required))"],
    haml_pipes: ["a:  (local (required))"],
  });
  const pickUri = fileUri(path.join(views, "strict", "keys_pick.html.erb"));
  const pickText =
    '<%= render partial: "strict/pick", formats: [:json],\n  locals: { } %>\n';
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: pickUri,
        languageId: "erb",
        version: 1,
        text: pickText,
      },
    },
    true,
  );
  check(
    "render locals keys follow formats:",
    (await req("textDocument/completion", pickUri, 1, 12)).result?.map(
      (i) => i.label,
    ),
    ["json_key"],
  );
  const rowUri = fileUri(path.join(views, "strict", "_row.html.erb"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: rowUri,
        languageId: "erb",
        version: 1,
        text: "<%# locals: (a:, b: 1) %>\n<%= a %> <%= b %> <%= link_to %>\n<% [1].each do |a| %><%= a %><% end %><%= f(b: 1) %>\n",
      },
    },
    true,
  );
  const hoverText = async (character) =>
    (await req("textDocument/hover", rowUri, 1, character)).result?.contents
      .value ?? null;
  check(
    "partial local hover",
    [
      await hoverText(4),
      await hoverText(13),
      (await hoverText(23))?.split("\n")[0],
      (await req("textDocument/hover", rowUri, 2, 25)).result,
      (await req("textDocument/hover", rowUri, 2, 44)).result,
    ],
    [
      "`a`: local, required\n\nPassed by 1 `render` call",
      "`b`: local, default `1`\n\nNo `render` passes it",
      "```ruby",
      null,
      null,
    ],
  );
  const prepare = async (line, character) =>
    (await req("textDocument/prepareRename", rowUri, line, character)).result;
  const localRename = await send("textDocument/rename", {
    textDocument: { uri: rowUri },
    position: { line: 1, character: 4 },
    newName: "item",
  });
  const refusedName = async (newName) =>
    (
      await send("textDocument/rename", {
        textDocument: { uri: rowUri },
        position: { line: 1, character: 4 },
        newName,
      })
    ).error?.code ?? null;
  const flat = (changes) =>
    Object.entries(changes ?? {})
      .flatMap(([u, es]) =>
        es.map(
          (e) =>
            `${rel(u)} ${e.range.start.line}:${e.range.start.character}-${e.range.end.character} ${e.newText}`,
        ),
      )
      .sort();
  const localRefs = (
    await send("textDocument/references", {
      textDocument: { uri: rowUri },
      position: { line: 1, character: 4 },
      context: { includeDeclaration: true },
    })
  ).result
    .map(
      (l) => `${rel(l.uri)} ${l.range.start.line}:${l.range.start.character}`,
    )
    .sort();
  check(
    "rename and references of a partial local",
    [
      await prepare(1, 4),
      await prepare(0, 13),
      await prepare(1, 23),
      await prepare(2, 25),
      flat(localRename.result?.changes),
      localRefs,
      await refusedName("class"),
      await refusedName("link_to"),
      await refusedName("b"),
    ],
    [
      { start: { line: 1, character: 4 }, end: { line: 1, character: 5 } },
      { start: { line: 0, character: 13 }, end: { line: 0, character: 14 } },
      null,
      null,
      [
        "/app/views/strict/_row.html.erb 0:13-14 item",
        "/app/views/strict/_row.html.erb 1:4-5 item",
        "/app/views/strict/list.html.erb 0:25-26 item",
      ],
      [
        "/app/views/strict/_row.html.erb 0:13",
        "/app/views/strict/_row.html.erb 1:4",
        "/app/views/strict/list.html.erb 0:25",
      ],
      -32602,
      -32602,
      -32602,
    ],
  );
  const i18nDocs = {
    slim: '= t(".title")\n= t(".ti")\n',
    haml: '= t(".title")\n%p= t(".ti")\n',
  };
  const i18nResults = {};
  for (const [lang, text] of Object.entries(i18nDocs)) {
    const i18nUri = fileUri(path.join(views, "users", `index.html.${lang}`));
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: i18nUri, languageId: lang, version: 1, text } },
      true,
    );
    const typed = text.split("\n")[1].length;
    i18nResults[lang] = [
      firstLine(await req("textDocument/hover", i18nUri, 0, 8)),
      (await req("textDocument/definition", i18nUri, 0, 8)).result?.map(
        (l) => `${rel(l.uri)}:${l.range.start.line}`,
      ),
      labels(await req("textDocument/completion", i18nUri, 1, typed - 2)),
    ];
  }
  const i18nExpected = [
    "`users.index.title`",
    ["/config/locales/en.yml:3", "/config/locales/uk.yml:3"],
    [".title"],
  ];
  check("lazy i18n keys in Slim and Haml", i18nResults, {
    slim: i18nExpected,
    haml: i18nExpected,
  });
  const streamUri = fileUri(
    path.join(views, "users", "create.turbo_stream.erb"),
  );
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: streamUri,
        languageId: "erb",
        version: 1,
        text: "<%= turbo_stream.rep %>\n",
      },
    },
    true,
  );
  check(
    "turbo_stream. completion",
    labels(await req("textDocument/completion", streamUri, 0, 20)),
    ["replace", "replace_all"],
  );
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: streamUri, version: 2 },
      contentChanges: [{ text: '<div data-action="click->he">\n' }],
    },
    true,
  );
  check(
    "stimulus completion",
    labels(await req("textDocument/completion", streamUri, 0, 27)),
    ["hello"],
  );
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: streamUri, version: 3 },
      contentChanges: [
        { text: '<p>render "users/card"</p> <%= render "users/card" %>\n' },
      ],
    },
    true,
  );
  const streamLinks = await send("textDocument/documentLink", {
    textDocument: { uri: streamUri },
  });
  check(
    "documentLink skips prose",
    streamLinks.result?.map((l) => l.range.start.character),
    [39],
  );
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: streamUri, version: 4 },
      contentChanges: [
        {
          text: "<%= form_with model: @user do |f| %>\n  <%= f.text_field :n\n",
        },
      ],
    },
    true,
  );
  check(
    "schema attribute completion",
    labels(await req("textDocument/completion", streamUri, 1, 21)),
    ["name", "nickname"],
  );
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: streamUri, version: 5 },
      contentChanges: [
        { text: "<p>Please fill in the form.</p>\n<p>Clear the cache</p>\n" },
      ],
    },
    true,
  );
  check(
    "no form builder in prose",
    labels(await req("textDocument/completion", streamUri, 0, 26)),
    [],
  );
  check(
    "no helper hover in prose",
    (await req("textDocument/hover", streamUri, 1, 16)).result ?? null,
    null,
  );
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: streamUri, version: 6 },
      contentChanges: [{ text: "<section>\n  <butt\n  <a hre\n  </\n" }],
    },
    true,
  );
  const html = async (line, character) =>
    (
      await req("textDocument/completion", streamUri, line, character)
    ).result?.map((i) => i.insertText ?? i.label);
  check("html element completion", await html(1, 7), ["button"]);
  check("html attribute completion", await html(2, 8), [
    'href="$1"',
    'hreflang="$1"',
  ]);
  check("html close completion", await html(3, 4), ["section>"]);
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: streamUri, version: 7 },
      contentChanges: [{ text: "<p><%=foo%></p>\n<%= ok %>\n" }],
    },
    true,
  );
  const spacing = (only) =>
    send("textDocument/codeAction", {
      textDocument: { uri: streamUri },
      range: {
        start: { line: 0, character: 5 },
        end: { line: 0, character: 5 },
      },
      context: { diagnostics: [], only },
    });
  const spacingActions = (r) =>
    r.result?.map((a) => [
      a.title,
      a.kind,
      a.edit.changes[streamUri].map((e) => e.newText),
    ]);
  check(
    "codeAction ERB tag spacing",
    spacingActions(await spacing(undefined)),
    [["Add spaces inside this ERB tag", "refactor.rewrite", ["<%= foo %>"]]],
  );
  check(
    "source action ERB spacing",
    spacingActions(await spacing(["source"]))?.filter(([, kind]) =>
      kind.startsWith("source"),
    ),
    [["Add spaces inside every ERB tag", "source.erbSpacing", ["<%= foo %>"]]],
  );
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: streamUri, version: 8 },
      contentChanges: [{ text: "<%= render Bad\n" }],
    },
    true,
  );
  const components = await req("textDocument/completion", streamUri, 0, 14);
  check(
    "component completion",
    components.result?.map((i) => [i.label, i.textEdit.newText]),
    [["BadgeComponent", "BadgeComponent.new($1)"]],
  );
  await send(
    "textDocument/didChange",
    {
      textDocument: { uri: streamUri, version: 9 },
      contentChanges: [
        { text: '<b data-action="hello#">\n<i data-hello-target="">\n' },
      ],
    },
    true,
  );
  check(
    "stimulus action and target completion",
    [
      labels(await req("textDocument/completion", streamUri, 0, 22)),
      labels(await req("textDocument/completion", streamUri, 1, 22)),
    ],
    [["greet"], ["name"]],
  );
  const jbuilderUri = fileUri(path.join(views, "users", "index.json.jbuilder"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: jbuilderUri,
        languageId: "ruby",
        version: 1,
        text: 'json.array! @users, partial: "users/user", as: :user\njson.partial! "users/user", user: @user\njson.tags @tags, label: "#tag", partial: "users/user"\n',
      },
    },
    true,
  );
  check(
    "jbuilder array! partial:",
    targets(await req("textDocument/definition", jbuilderUri, 0, 33)),
    ["/app/views/users/_user.json.jbuilder"],
  );
  check(
    "jbuilder partial!",
    targets(await req("textDocument/definition", jbuilderUri, 1, 18)),
    ["/app/views/users/_user.json.jbuilder"],
  );
  check(
    "jbuilder partial: after a # in a string",
    targets(await req("textDocument/definition", jbuilderUri, 2, 45)),
    ["/app/views/users/_user.json.jbuilder"],
  );
  check(
    "jbuilder completion",
    labels(
      await send("textDocument/completion", {
        textDocument: { uri: jbuilderUri },
        position: { line: 1, character: 21 },
      }),
    ),
    ["users/user"],
  );
  const card = path.join(views, "users", "_card.html.slim");
  const renamed = await send("workspace/willRenameFiles", {
    files: [
      {
        oldUri: fileUri(card),
        newUri: fileUri(card.replace("_card", "_tile")),
      },
    ],
  });
  const renameEdits = Object.entries(renamed.result?.changes ?? {})
    .flatMap(([u, es]) =>
      es.map(
        (e) =>
          `${rel(decodeURIComponent(u))} ${e.range.start.line}:${e.range.start.character} ${e.newText}`,
      ),
    )
    .sort();
  check("willRenameFiles _card", renameEdits, [
    "/app/views/users/edit.html+phone.slim 0:10 tile",
    // Open and never saved: renamed in the buffer too.
    "/app/views/users/edit.html.slim 7:14 tile",
    "/app/views/users/index.html.erb 0:12 tile",
    "/app/views/users/index.html.erb 9:12 tile",
    "/app/views/users/outline.html.slim 2:18 tile",
    "/app/views/users/show.html.haml 1:16 tile",
  ]);
  // A changed .ruby-version changes the grammar Ruby is parsed with, so open documents are checked again.
  const versionedUri = fileUri(path.join(views, "users", "versioned.html.erb"));
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: versionedUri,
        languageId: "erb",
        version: 1,
        text: "<% x = a\n  && b %>\n",
      },
    },
    true,
  );
  const publishedFor = (u) =>
    notes.filter(
      (n) =>
        n.method === "textDocument/publishDiagnostics" && n.params.uri === u,
    );
  await new Promise((r) => setTimeout(r, 200));
  check(
    "no .ruby-version",
    publishedFor(versionedUri).at(-1)?.params.diagnostics.length,
    0,
  );
  const rubyVersionFile = path.join(root, ".ruby-version");
  fs.writeFileSync(rubyVersionFile, "3.3.0\n");
  await send(
    "workspace/didChangeWatchedFiles",
    { changes: [{ uri: fileUri(rubyVersionFile), type: 1 }] },
    true,
  );
  await new Promise((r) => setTimeout(r, 200));
  check(
    ".ruby-version 3.3.0",
    publishedFor(versionedUri)
      .at(-1)
      ?.params.diagnostics.map((d) => d.message),
    ["unexpected '&&', ignoring it"],
  );
  // Diagnostics run off the server task: edits typed meanwhile supersede them, and only the latest version publishes.
  for (const version of [2, 3, 4]) {
    await send(
      "textDocument/didChange",
      {
        textDocument: { uri: versionedUri, version },
        contentChanges: [{ text: `<%= f(${version},,) %>\n` }],
      },
      true,
    );
  }
  await new Promise((r) => setTimeout(r, 400));
  const lastPublished = publishedFor(versionedUri).at(-1)?.params;
  check(
    "diagnostics for the latest edit only",
    [
      lastPublished?.version,
      publishedFor(versionedUri).some((n) => [2, 3].includes(n.params.version)),
    ],
    [4, false],
  );
  // Two apps in one workspace: every lookup resolves against the app the file belongs to.
  const multiRoot = fs.realpathSync(
    fs.mkdtempSync(path.join(os.tmpdir(), "rt-multi-")),
  );
  const multi = startServer();
  const apps = ["a", "b"].map((name) => {
    const dir = path.join(multiRoot, `app_${name}`);
    const route = name === "a" ? "users" : "posts";
    fs.mkdirSync(path.join(dir, "app/views/shared"), { recursive: true });
    fs.mkdirSync(path.join(dir, "app/views/pages"), { recursive: true });
    fs.mkdirSync(path.join(dir, "config/locales"), { recursive: true });
    fs.writeFileSync(
      path.join(dir, "Gemfile.lock"),
      "GEM\n  specs:\n    actionview (7.1.3)\n",
    );
    fs.writeFileSync(
      path.join(dir, "app/views/shared/_card.html.erb"),
      `<p>${name}</p>\n`,
    );
    fs.writeFileSync(
      path.join(dir, "config/locales/en.yml"),
      `en:\n  greet:\n    hello: "Hi ${name}"\n    only_${name}: "x"\n`,
    );
    const page = fileUri(path.join(dir, "app/views/pages/show.html.erb"));
    const text = `<%= render "shared/card" %>\n<%= t("greet.hello") %>\n<%= ${route}_p %>\n<%= ${route}_path %>\n<%= t("greet.on") %>\n`;
    return { name, dir, route, page, text };
  });
  await multi.send("initialize", {
    processId: null,
    rootUri: fileUri(apps[0].dir),
    workspaceFolders: apps.map(({ name, dir }) => ({
      name,
      uri: fileUri(dir),
    })),
    capabilities: {},
  });
  await multi.send("initialized", {}, true);
  for (const { dir, route, page, text } of apps) {
    const routes = [
      {
        name: route,
        verb: "GET",
        path: `/${route}(.:format)`,
        action: `${route}#index`,
        source: "config/routes.rb:2",
      },
    ];
    await multi.send("rubyTemplates/routes", { root: dir, routes }, true);
    await multi.send(
      "textDocument/didOpen",
      { textDocument: { uri: page, languageId: "erb", version: 1, text } },
      true,
    );
  }
  const ask = (method, u, line, character) =>
    multi.send(method, {
      textDocument: { uri: u },
      position: { line, character },
    });
  const where = (r) =>
    (r.result || []).map(
      (l) =>
        `${path.relative(multiRoot, fileURLToPath(l.uri)).replaceAll(path.sep, "/")}:${l.range.start.line}`,
    );
  const lookups = [];
  for (const { name, route, page, text } of apps) {
    lookups.push([
      where(await ask("textDocument/definition", page, 0, 14)),
      where(await ask("textDocument/definition", page, 1, 10)),
      (
        await ask("textDocument/hover", page, 1, 10)
      ).result?.contents.value.includes(`Hi ${name}`),
      (
        await ask(
          "textDocument/completion",
          page,
          2,
          text.split("\n")[2].length - 3,
        )
      ).result?.map((i) => i.label),
      (
        await ask("textDocument/hover", page, 3, 6)
      ).result?.contents.value.split("\n")[0],
      where(await ask("textDocument/definition", page, 3, 6)),
      (await ask("textDocument/completion", page, 4, 15)).result?.map(
        (i) => i.label,
      ),
    ]);
  }
  check("two apps in one workspace", lookups, [
    [
      ["app_a/app/views/shared/_card.html.erb:0"],
      ["app_a/config/locales/en.yml:2"],
      true,
      ["users_path"],
      "`users_path` → `/users`",
      ["app_a/config/routes.rb:1"],
      ["greet.only_a"],
    ],
    [
      ["app_b/app/views/shared/_card.html.erb:0"],
      ["app_b/config/locales/en.yml:2"],
      true,
      ["posts_path"],
      "`posts_path` → `/posts`",
      ["app_b/config/routes.rb:1"],
      ["greet.only_b"],
    ],
  ]);
  await multi.send("shutdown", {});
  await multi.send("exit", {}, true);
  fs.rmSync(multiRoot, { recursive: true, force: true });
  // Per-language switches from the client's initializationOptions: no diagnostics, no formatting edits.
  const off = startServer();
  await off.send("initialize", {
    processId: null,
    rootUri: fileUri(root),
    capabilities: {},
    initializationOptions: {
      erb: { diagnostics: { enabled: false }, format: { enabled: false } },
      haml: { format: { enabled: false } },
    },
  });
  await off.send("initialized", {}, true);
  const open = (u, languageId, text) =>
    off.send(
      "textDocument/didOpen",
      { textDocument: { uri: u, languageId, version: 1, text } },
      true,
    );
  await open(uri, "erb", text);
  await open(reindentUri, "haml", "%div\n    %p\n");
  await open(hamlUri, "haml", hamlText);
  await open(editUri, "slim", "div\n    p x\n");
  await new Promise((r) => setTimeout(r, 300));
  const diagnosticsOf = (u) =>
    off.notes
      .filter(
        (n) =>
          n.method === "textDocument/publishDiagnostics" && n.params.uri === u,
      )
      .at(-1)?.params.diagnostics.length;
  check("erb diagnostics off", diagnosticsOf(uri) ?? 0, 0);
  expect(
    "haml diagnostics stay on",
    diagnosticsOf(hamlUri) > 0,
    diagnosticsOf(hamlUri),
  );
  const formatResult = async (u) =>
    (
      await off.send("textDocument/formatting", {
        textDocument: { uri: u },
        options: { tabSize: 2, insertSpaces: true },
      })
    ).result;
  check("haml format off", await formatResult(reindentUri), null);
  const rangeOff = await off.send("textDocument/rangeFormatting", {
    textDocument: { uri: reindentUri },
    options: { tabSize: 2, insertSpaces: true },
    range: { start: { line: 1, character: 0 }, end: { line: 1, character: 0 } },
  });
  check("haml range format off", rangeOff.result, null);
  expect(
    "slim format stays on",
    (await formatResult(editUri))?.[0]?.newText === "div\n  p x\n",
  );
  await off.send("shutdown", {});
  await off.send("exit", {}, true);
  // `erb.format.alignComments` moves a comment's lines with the line it opens on.
  const aligned = startServer();
  await aligned.send("initialize", {
    processId: null,
    rootUri: fileUri(root),
    capabilities: {},
    initializationOptions: { erb: { format: { alignComments: true } } },
  });
  await aligned.send("initialized", {}, true);
  const commentUri = fileUri(path.join(views, "users", "comment.html.erb"));
  await aligned.send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: commentUri,
        languageId: "erb",
        version: 1,
        text: "<div>\n<%# a\n  b %>\n</div>\n",
      },
    },
    true,
  );
  const alignedEdits = (
    await aligned.send("textDocument/formatting", {
      textDocument: { uri: commentUri },
      options: { tabSize: 2, insertSpaces: true },
    })
  ).result;
  check(
    "erb comments aligned",
    alignedEdits?.[0]?.newText,
    "<div>\n  <%# a\n    b %>\n</div>\n",
  );
  const alignedRange = (
    await aligned.send("textDocument/rangeFormatting", {
      textDocument: { uri: commentUri },
      options: { tabSize: 2, insertSpaces: true },
      range: {
        start: { line: 2, character: 0 },
        end: { line: 2, character: 0 },
      },
    })
  ).result;
  check(
    "erb comments kept in a range without their first line",
    alignedRange ?? [],
    [],
  );
  // Layout/EmptyLines removes lines in Format Document, never in Format Selection.
  const blankDir = path.join(views, "blank");
  fs.mkdirSync(blankDir);
  fs.writeFileSync(
    path.join(blankDir, ".ruby-templates.toml"),
    "[Layout.EmptyLines]\nenabled = true\nmax = 0\n",
  );
  const blankUri = fileUri(path.join(blankDir, "a.html.erb"));
  await aligned.send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: blankUri,
        languageId: "erb",
        version: 1,
        text: "<div>\n<p>a</p>\n\n\n<p>b</p>\n\n</div>\n",
      },
    },
    true,
  );
  const blankOptions = { tabSize: 2, insertSpaces: true };
  check(
    "empty lines cut in the document",
    (
      await aligned.send("textDocument/formatting", {
        textDocument: { uri: blankUri },
        options: blankOptions,
      })
    ).result?.[0]?.newText,
    "<div>\n  <p>a</p>\n  <p>b</p>\n</div>\n",
  );
  const blankRange = (
    await aligned.send("textDocument/rangeFormatting", {
      textDocument: { uri: blankUri },
      options: blankOptions,
      range: {
        start: { line: 0, character: 0 },
        end: { line: 6, character: 0 },
      },
    })
  ).result;
  check(
    "empty lines kept in a range",
    blankRange?.map((e) => [e.range.start.line, e.range.end.line, e.newText]),
    [
      [1, 1, "  <p>a</p>"],
      [4, 4, "  <p>b</p>"],
    ],
  );
  // The in-line rules keep every line, so Format Selection applies them to the lines it covers.
  const inlineDir = path.join(views, "inline");
  fs.mkdirSync(inlineDir);
  fs.writeFileSync(
    path.join(inlineDir, ".ruby-templates.toml"),
    [
      "Layout.SpaceInsideErbTag",
      "Style.ErbCommentSyntax",
      "Layout.SpaceAfterScriptMarker",
    ]
      .map((rule) => `[${rule}]\nenabled = true\n`)
      .join(""),
  );
  const inlineTexts = {
    "a.html.erb": ["<% # a %>\n<%=b%>\n<%=c%>\n", [[1, "<%= b %>"]]],
    "a.html.haml": ["=a\n=b\n=c\n", [[1, "= b"]]],
    "a.html.slim": ["p=a\np=b\np=c\n", [[1, "p= b"]]],
  };
  for (const [name, [text, lines]] of Object.entries(inlineTexts)) {
    const uri = fileUri(path.join(inlineDir, name));
    await aligned.send(
      "textDocument/didOpen",
      {
        textDocument: {
          uri,
          languageId: name.split(".").pop(),
          version: 1,
          text,
        },
      },
      true,
    );
    const edits = (
      await aligned.send("textDocument/rangeFormatting", {
        textDocument: { uri },
        options: blankOptions,
        range: {
          start: { line: 1, character: 0 },
          end: { line: 1, character: 3 },
        },
      })
    ).result;
    check(
      `${name}: a range gets the in-line rules on its lines only`,
      edits?.map((e) => [e.range.start.line, e.newText]),
      lines,
    );
  }

  // The rules around bodies and branches add and remove lines: not in Format Selection.
  const aroundDir = path.join(views, "around");
  fs.mkdirSync(aroundDir);
  const aroundToml = path.join(aroundDir, ".ruby-templates.toml");
  const aroundOpen = async (name, text) => {
    const uri = fileUri(path.join(aroundDir, name));
    await aligned.send(
      "textDocument/didOpen",
      {
        textDocument: {
          uri,
          languageId: name.split(".").pop(),
          version: 1,
          text,
        },
      },
      true,
    );
    return uri;
  };
  const aroundTexts = {
    "a.html.haml": "%div\n    %p x\n\n    %i y\n\n",
    "a.html.slim": "div\n    p x\n\n    i y\n\n",
    "a.html.erb":
      "<div>\n<% if a %>\n\n<p>x</p>\n\n<% else %>\n<p>y</p>\n<% end %>\n\n</div>\n",
  };
  for (const style of ["empty_lines", "no_empty_lines"]) {
    fs.writeFileSync(
      aroundToml,
      ["BlockBody", "ElementBody", "Branches"]
        .map(
          (rule) =>
            `[Layout.EmptyLinesAround${rule}]\nenabled = true\nstyle = "${style}"\n`,
        )
        .join(""),
    );
    for (const [name, text] of Object.entries(aroundTexts)) {
      const uri = await aroundOpen(`${style}-${name}`, text);
      const lines = text.split("\n").length;
      const edits = (
        await aligned.send("textDocument/rangeFormatting", {
          textDocument: { uri },
          options: blankOptions,
          range: {
            start: { line: 0, character: 0 },
            end: { line: lines, character: 0 },
          },
        })
      ).result;
      // Each edit replaces the text of one line; applied, the file keeps its lines and its content.
      const result = text.split("\n");
      for (const e of edits ?? []) {
        result.splice(
          e.range.start.line,
          e.range.end.line - e.range.start.line + 1,
          ...e.newText.split("\n"),
        );
      }
      check(
        `${style} ${name}: a range keeps the lines and the content`,
        [result.length, result.join("\n").replace(/\s/g, "")],
        [lines, text.replace(/\s/g, "")],
      );
    }
  }

  // .ruby-templates.toml: rules and switches per folder, and its own problems.
  const rulesRoot = fs.realpathSync(
    fs.mkdtempSync(path.join(os.tmpdir(), "rt-rules-")),
  );
  fs.mkdirSync(path.join(rulesRoot, "sub"));
  const rulesToml = path.join(rulesRoot, ".ruby-templates.toml");
  const quiet =
    "[Html.UnclosedElement]\nenabled = false\n[erb]\nformat.enabled = false\n";
  fs.writeFileSync(rulesToml, quiet);
  fs.writeFileSync(
    path.join(rulesRoot, "sub", ".ruby-templates.toml"),
    "[Html.UnclosedElement]\nenabled = true\n[erb]\nformat.enabled = true\n",
  );
  const unclosed = "<div>\n<%= x %>\n";
  const rulesServer = startServer();
  await rulesServer.send("initialize", {
    processId: null,
    rootUri: fileUri(rulesRoot),
    capabilities: {},
  });
  await rulesServer.send("initialized", {}, true);
  const rulesOpen = (name, text) =>
    rulesServer.send(
      "textDocument/didOpen",
      {
        textDocument: {
          uri: fileUri(path.join(rulesRoot, name)),
          languageId: "erb",
          version: 1,
          text,
        },
      },
      true,
    );
  const rulesPublished = (u) =>
    rulesServer.notes
      .filter(
        (n) =>
          n.method === "textDocument/publishDiagnostics" && n.params.uri === u,
      )
      .at(-1)?.params.diagnostics;
  const until = async (label, ready) => {
    for (let i = 0; i < 60; i++) {
      if (ready()) return;
      await new Promise((r) => setTimeout(r, 50));
    }
    failures++;
    console.log(`FAIL timed out waiting for ${label}`);
  };
  await rulesOpen("a.html.erb", unclosed);
  await rulesOpen("sub/b.html.erb", unclosed);
  await until(
    "the first diagnostics",
    () =>
      rulesPublished(fileUri(path.join(rulesRoot, "a.html.erb"))) &&
      rulesPublished(fileUri(path.join(rulesRoot, "sub/b.html.erb"))),
  );
  const rulesCodes = (name) =>
    rulesPublished(fileUri(path.join(rulesRoot, name)))?.map((d) => d.code);
  check("toml: rule off", rulesCodes("a.html.erb"), []);
  check("toml: nearest file wins", rulesCodes("sub/b.html.erb"), [
    "Html/UnclosedElement",
  ]);
  const reindent = "<div>\n<p>a</p>\n</div>\n";
  const rulesFormat = async (name, text) =>
    (
      await rulesServer.send("textDocument/formatting", {
        textDocument: { uri: fileUri(path.join(rulesRoot, name)) },
        options: { tabSize: 2, insertSpaces: true },
      })
    ).result;
  await rulesOpen("c.html.erb", reindent);
  await rulesOpen("sub/d.html.erb", reindent);
  check("toml: format off", await rulesFormat("c.html.erb"), null);
  check(
    "toml: format on in the subfolder",
    (await rulesFormat("sub/d.html.erb"))?.[0]?.newText,
    "<div>\n  <p>a</p>\n</div>\n",
  );
  fs.writeFileSync(rulesToml, "[Html.UnclosedElement]\nenabeld = false\n");
  await rulesServer.send(
    "workspace/didChangeWatchedFiles",
    { changes: [{ uri: fileUri(rulesToml), type: 2 }] },
    true,
  );
  const tomlUri = fileUri(rulesToml);
  await until(
    "the toml problems",
    () =>
      rulesPublished(tomlUri)?.length && rulesCodes("a.html.erb")?.length === 1,
  );
  check(
    "toml: problems are published on the file",
    rulesPublished(tomlUri)?.map((d) => [d.range.start.line, d.message]),
    [
      [
        1,
        "unknown key `enabeld` in [Html.UnclosedElement]; it takes enabled, include, exclude",
      ],
    ],
  );
  check(
    "toml: problems are logged",
    rulesServer.notes.some(
      (n) =>
        n.method === "window/logMessage" &&
        n.params.message.includes("unknown key `enabeld`"),
    ),
    true,
  );
  check("toml: rule back on after the edit", rulesCodes("a.html.erb"), [
    "Html/UnclosedElement",
  ]);
  fs.writeFileSync(rulesToml, quiet);
  await rulesServer.send(
    "workspace/didChangeWatchedFiles",
    { changes: [{ uri: tomlUri, type: 2 }] },
    true,
  );
  await until(
    "the toml problems to clear",
    () =>
      rulesPublished(tomlUri)?.length === 0 &&
      rulesCodes("a.html.erb")?.length === 0,
  );
  check("toml: problems cleared when fixed", rulesPublished(tomlUri), []);
  check("toml: rule off again", rulesCodes("a.html.erb"), []);
  await rulesServer.send("shutdown", {});
  await rulesServer.send("exit", {}, true);
  fs.rmSync(rulesRoot, { recursive: true, force: true });
  // VS Code percent-encodes `@` and `+` and the server must answer with URIs that decode to the same files.
  const oddDir = path.join(views, "my app @1+2");
  fs.mkdirSync(oddDir, { recursive: true });
  fs.writeFileSync(path.join(oddDir, "_note.html.erb"), "<i></i>\n");
  const oddText = '<%= render "note" %>\n';
  fs.writeFileSync(path.join(oddDir, "show.html.erb"), oddText);
  const oddUri = fileUri(root) + "/app/views/my%20app%20%401%2B2/show.html.erb";
  await send(
    "textDocument/didOpen",
    {
      textDocument: {
        uri: oddUri,
        languageId: "erb",
        version: 1,
        text: oddText,
      },
    },
    true,
  );
  check(
    "definition in a dir with space, @ and +",
    (await req("textDocument/definition", oddUri, 0, 14)).result?.map((l) =>
      fs.realpathSync(fileURLToPath(l.uri)),
    ),
    [path.join(oddDir, "_note.html.erb")],
  );
  await aligned.send("shutdown", {});
  await aligned.send("exit", {}, true);
  await send("shutdown", {});
  await send("exit", {}, true);
  fs.rmSync(root, { recursive: true, force: true });
  console.log(failures ? `${failures} check(s) failed` : "all checks passed");
  process.exit(failures ? 1 : 0);
})();
setTimeout(() => {
  console.error("timeout");
  process.exit(1);
}, 8000);
