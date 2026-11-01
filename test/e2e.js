// Minimal LSP smoke test against a fake Rails tree: definition, hover, completion, diagnostics, symbols, rename.
import cp from "child_process";
import fs from "fs";
import path from "path";
import os from "os";
import { pathToFileURL } from "url";
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
fs.mkdirSync(path.join(root, "db"), { recursive: true });
fs.writeFileSync(
  path.join(root, "db", "schema.rb"),
  'create_table "users", force: :cascade do |t|\n  t.string "name"\n  t.string "nickname"\n  t.string "email"\nend\n',
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
      notes.filter(
        (n) =>
          n.method === "textDocument/publishDiagnostics" &&
          n.params.uri === u &&
          n.params.diagnostics.length > 0,
      );
    for (let waited = 0; waited < 2000 && !published().length; waited += 50) {
      await new Promise((r) => setTimeout(r, 50));
    }
    untitled[lang] = {
      errors: replies.filter((r) => r.error).length,
      symbols: replies[0].result?.length > 0,
      definition: replies[2].result ?? null,
      diagnostics: published().length > 0,
    };
  }
  const untitledOk = {
    errors: 0,
    symbols: true,
    definition: null,
    diagnostics: true,
  };
  check("untitled documents", untitled, {
    erb: untitledOk,
    haml: untitledOk,
    slim: untitledOk,
  });
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
  const strictFixes = async (u, text) => {
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: u, languageId: "erb", version: 1, text } },
      true,
    );
    const at = {
      start: { line: 0, character: 12 },
      end: { line: 0, character: 22 },
    };
    const { result } = await send("textDocument/codeAction", {
      textDocument: { uri: u },
      range: at,
      context: {
        diagnostics: [{ range: at, code: "strict-locals", message: "" }],
      },
    });
    return result
      .filter((a) => a.kind === "quickfix")
      .map((a) => {
        const [[target, [e]]] = Object.entries(a.edit.changes);
        return `${a.title} | ${rel(target)} ${e.range.start.line}:${e.range.start.character}-${e.range.end.character} "${e.newText}"`;
      });
  };
  const extraUri = fileUri(path.join(views, "strict", "extra.html.erb"));
  check(
    "strict locals quick fixes",
    [
      await strictFixes(strictUri, '<%= render "strict/row" %>\n'),
      await strictFixes(extraUri, '<%= render "strict/row", a: 1, c: 2 %>\n'),
    ],
    [
      [
        'Pass `a` to the render | /app/views/strict/show.html.erb 0:23-23 ", a: nil"',
      ],
      [
        'Remove `c` from the render | /app/views/strict/extra.html.erb 0:29-35 ""',
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
      "3:3 ruby-syntax",
      "4:1 ruby-syntax",
      "5:3 void-children",
      "8:1 ruby-syntax",
      "8:1 ruby-syntax",
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
      "2:11 ruby-syntax",
      "5:5 ruby-syntax",
      "4:3 ruby-syntax",
      "5:3 ruby-syntax",
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
        { range: missing, code: "unresolved-render", message: "not found" },
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
    erb: '<%= render "strict/row", a: 1, | %>\n',
    slim: '= render "strict/row",\n    b: 2,\n    |\n',
    haml: '= render("strict/row", |)\n',
  };
  const renderKeys = {};
  for (const [lang, marked] of Object.entries(keyDocs)) {
    const keysUri = fileUri(path.join(views, "strict", `keys.html.${lang}`));
    const text = marked.replace("|", "");
    await send(
      "textDocument/didOpen",
      { textDocument: { uri: keysUri, languageId: lang, version: 1, text } },
      true,
    );
    const before = marked.slice(0, marked.indexOf("|")).split("\n");
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
    haml: ["a:  (local (required))", "b:  (local = 1)"],
  });
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
        text: 'json.array! @users, partial: "users/user", as: :user\njson.partial! "users/user", user: @user\n',
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
