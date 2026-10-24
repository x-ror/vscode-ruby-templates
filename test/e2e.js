// Minimal LSP smoke test against a fake Rails tree: definition, hover, completion, diagnostics, symbols, rename.
import cp from "child_process";
import fs from "fs";
import path from "path";
import os from "os";
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
const root = fs.mkdtempSync(path.join(os.tmpdir(), "rt-e2e-"));
const views = path.join(root, "app", "views");
fs.mkdirSync(path.join(views, "users"), { recursive: true });
fs.mkdirSync(path.join(views, "shared"), { recursive: true });
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
  "class BadgeComponent; end\n",
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
  "",
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
const uri = "file://" + file;
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
const hamlUri = "file://" + hamlFile;
const slimFile = path.join(views, "users", "show.html.slim");
const slimText =
  'mjml:\n  x\nmarkdown2:\n  y\n- else\n- end\nbr\n  p\n- form_with model: @u do |f|\n  = f.text_field :x\n= link_to("x"\n';
fs.writeFileSync(slimFile, slimText);
const slimUri = "file://" + slimFile;
const outlineFile = path.join(views, "users", "outline.html.slim");
const outlineText =
  'main#page\n  = content_for :aside do\n    nav = render "card"\n  a(href="/"\n    class="x")\n    | Home\n\njavascript:\n  go();\n';
fs.writeFileSync(outlineFile, outlineText);
const outlineUri = "file://" + outlineFile;
// Unsaved text differs from disk; VS Code sends `+` percent-encoded.
const phoneFile = path.join(views, "users", "edit.html+phone.slim");
fs.writeFileSync(phoneFile, '\n\n= render "card"\n');
const phoneText = '= render "card"\n';
const phoneUri = "file://" + phoneFile.replace("+", "%2B");

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
const rel = (p) => p.replace("file://" + root, "");
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
    rootUri: "file://" + root,
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
  const quoteUri = "file://" + path.join(views, "users", "quote.html.erb");
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
  // Lines 4 and 13 are tags left open for the completion checks; each swallows the tags up to the next `%>`.
  check("diagnostics", diagnostics(uri), [
    "4: Unclosed ERB tag: missing `%>` before the next `<%`",
    "13: Unclosed ERB tag: missing `%>` before the next `<%`",
    "7: Partial `shared/missing` not found",
  ]);

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
    "10: Unknown Haml filter `:jav`",
    "13: Illegal nesting: content can't be both given on the same line as %p and nested within it.",
    "12: Partial `shared/missing` not found",
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
      "3:1 unknown-engine",
      "5:1 unmatched-branch",
      "8:3 void-children",
      "9:3 silent-output-helper",
      "11:1 unclosed-ruby",
    ],
  );

  const editUri = "file://" + path.join(views, "users", "edit.html.slim");
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
  expect("slim formatting, tabs", tabs.result === null, tabs.result);
  // Without dynamic registration nothing is advertised, so ERB/Haml never see a formatter that does nothing.
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
  const streamUri =
    "file://" + path.join(views, "users", "create.turbo_stream.erb");
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
  const jbuilderUri =
    "file://" + path.join(views, "users", "index.json.jbuilder");
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
        oldUri: "file://" + card,
        newUri: "file://" + card.replace("_card", "_tile"),
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
    "/app/views/users/index.html.erb 0:12 tile",
    "/app/views/users/index.html.erb 9:12 tile",
    "/app/views/users/outline.html.slim 2:18 tile",
    "/app/views/users/show.html.haml 1:16 tile",
  ]);
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
