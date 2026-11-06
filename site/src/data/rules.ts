// The formatting and lint rules: the ones the extension has today and the ones planned. Names, options and the
// config file are a draft. `checklist()` turns the list into a Markdown task list for a tracking issue:
//   cd site && bun -e 'import { checklist } from "./src/data/rules.ts"; console.log(checklist())'
import { diagnostics, LANGUAGE_NAMES, type Language } from "./diagnostics";

export type Status = "available" | "partial" | "planned";
export type RuleType = "autofix" | "warning";

export const DEPARTMENTS = [
  { id: "layout", title: "Layout", prefixes: ["Layout"] },
  { id: "style", title: "Style", prefixes: ["Style"] },
  { id: "rails", title: "Rails", prefixes: ["Rails"] },
  {
    id: "turbo-stimulus",
    title: "Turbo and Stimulus",
    prefixes: ["Turbo", "Stimulus"],
  },
  { id: "html", title: "HTML and accessibility", prefixes: ["Html", "A11y"] },
] as const;

export interface Fault {
  line: number;
  column: number;
  message: string;
  code: string;
}

/** An autofix rule shows `before` and `after`; a warning shows what is flagged and what is fine. */
export interface Example {
  lang: Language;
  /** Caption of the `before` listing. */
  file: string;
  /** Caption of the `after` listing, when it differs. */
  afterFile?: string;
  before: string;
  after: string;
  /** Real messages for the flagged template; only for checks that exist today. */
  faults?: Fault[];
  /** Context the listings need, such as the partial a `render` talks to; backticks quote code. */
  note?: string;
}

export interface Rule {
  name: string;
  /** Backticks quote code. */
  summary: string;
  languages: Language[];
  /** The languages that have it today, when that is fewer than `languages`. */
  availableIn?: Language[];
  type: RuleType;
  /** Whether what the rule does leaves the rendered page as it is. */
  safe: boolean;
  /** Why; a warning that is safe needs none, since it only reports. */
  safeNote?: string;
  status: Status;
  /** Planned rules need one; a partial or available rule has one when the rest is scheduled. */
  phase?: number;
  /** For a rule that exists in part: how it works now. */
  today?: string;
  /** The diagnostic code on the diagnostics page that covers it today. */
  code?: string;
  /** JSON for the setting that controls it today. */
  setting?: { label: string; json: string };
  /** The planned `.ruby-templates.yml` section, for rules with options. */
  config?: string;
  examples: Example[];
}

const text = (...lines: string[]) => `${lines.join("\n")}\n`;

const ERB_FILE = "app/views/users/index.html.erb";
const HAML_FILE = "app/views/users/index.html.haml";
const SLIM_FILE = "app/views/users/index.html.slim";

/** The template, message and position the diagnostics reference shows for `code`. */
function reported(code: string) {
  const entry = diagnostics.find((d) => d.code === code);
  if (!entry) {
    throw new Error(
      `site/src/data/rules.ts refers to the diagnostic \`${code}\`, which diagnostics.ts doesn't have`,
    );
  }
  const { line, column, message } = entry;
  return {
    file: entry.example.file,
    before: entry.example.code,
    faults: [{ line, column, message, code }],
  };
}

export const rules: Rule[] = [
  // ---- Layout ----
  {
    name: "Layout/EmptyLines",
    summary: "At most a set number of blank lines in a row.",
    languages: ["erb", "haml", "slim"],
    type: "autofix",
    safe: true,
    safeNote:
      "Browsers collapse the whitespace between elements, so a shorter run of blank lines renders the same. Content where whitespace counts (`<pre>`, `<textarea>`, filters) is left alone, as the formatter does today.",
    status: "planned",
    phase: 1,
    config: text("Layout/EmptyLines:", "  Enabled: true", "  Max: 1"),
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<h1>Users</h1>",
          "",
          "",
          "",
          "<ul>",
          "  <li>Ada</li>",
          "</ul>",
        ),
        after: text("<h1>Users</h1>", "", "<ul>", "  <li>Ada</li>", "</ul>"),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("%h1 Users", "", "", "", "%ul", "  %li Ada"),
        after: text("%h1 Users", "", "%ul", "  %li Ada"),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("h1 Users", "", "", "", "ul", "  li Ada"),
        after: text("h1 Users", "", "ul", "  li Ada"),
      },
    ],
  },
  {
    name: "Layout/EmptyLinesAroundBlockBody",
    summary:
      "Blank lines at the start and end of the body of an ERB or Ruby block.",
    languages: ["erb", "haml", "slim"],
    type: "autofix",
    safe: true,
    safeNote: "Only blank lines between tags change, which render as nothing.",
    status: "planned",
    phase: 1,
    config: text(
      "Layout/EmptyLinesAroundBlockBody:",
      "  Enabled: true",
      "  EnforcedStyle: no_empty_lines # or empty_lines, or keep",
    ),
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<% if @admin_section %>",
          "",
          "  <% @reports.each do |report| %>",
          "",
          "    <p><%= report.title %></p>",
          "",
          "",
          "",
          "  <% end %>",
          "",
          "",
          "<% end %>",
        ),
        after: text(
          "<% if @admin_section %>",
          "  <% @reports.each do |report| %>",
          "    <p><%= report.title %></p>",
          "  <% end %>",
          "<% end %>",
        ),
        note: "With `no_empty_lines`. `empty_lines` would instead keep exactly one blank line after the opening tag and before the closing one.",
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          "- if @admin_section",
          "",
          "  - @reports.each do |report|",
          "",
          "    %p= report.title",
          "",
          "",
          "%footer Done",
        ),
        after: text(
          "- if @admin_section",
          "  - @reports.each do |report|",
          "    %p= report.title",
          "%footer Done",
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          "- if @admin_section",
          "",
          "  - @reports.each do |report|",
          "",
          "    p = report.title",
          "",
          "",
          "footer Done",
        ),
        after: text(
          "- if @admin_section",
          "  - @reports.each do |report|",
          "    p = report.title",
          "footer Done",
        ),
      },
    ],
  },
  {
    name: "Layout/EmptyLinesAroundElementBody",
    summary: "Blank lines at the start and end of the body of an HTML element.",
    languages: ["erb", "haml", "slim"],
    type: "autofix",
    safe: true,
    safeNote: "The blank lines sit between tags, where they render as nothing.",
    status: "planned",
    phase: 1,
    config: text(
      "Layout/EmptyLinesAroundElementBody:",
      "  Enabled: true",
      "  EnforcedStyle: no_empty_lines # or empty_lines, or keep",
    ),
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<section class="users">',
          "",
          "  <h2>Users</h2>",
          "  <p>Active this week</p>",
          "",
          "",
          "</section>",
        ),
        after: text(
          '<section class="users">',
          "  <h2>Users</h2>",
          "  <p>Active this week</p>",
          "</section>",
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          "%section.users",
          "",
          "  %h2 Users",
          "  %p Active this week",
          "",
          "",
          "%footer Done",
        ),
        after: text(
          "%section.users",
          "  %h2 Users",
          "  %p Active this week",
          "%footer Done",
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          "section.users",
          "",
          "  h2 Users",
          "  p Active this week",
          "",
          "",
          "footer Done",
        ),
        after: text(
          "section.users",
          "  h2 Users",
          "  p Active this week",
          "footer Done",
        ),
      },
    ],
  },
  {
    name: "Layout/EmptyLinesAroundBranches",
    summary: "Blank lines before and after an `else`, `elsif` or `when`.",
    languages: ["erb", "haml", "slim"],
    type: "autofix",
    safe: true,
    safeNote: "The blank lines sit between tags, where they render as nothing.",
    status: "planned",
    phase: 1,
    config: text(
      "Layout/EmptyLinesAroundBranches:",
      "  Enabled: true",
      "  EnforcedStyle: no_empty_lines # or empty_lines, or keep",
    ),
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<% if @user.admin? %>",
          "  <p>Admin</p>",
          "",
          "<% else %>",
          "",
          "  <p>Member</p>",
          "<% end %>",
        ),
        after: text(
          "<% if @user.admin? %>",
          "  <p>Admin</p>",
          "<% else %>",
          "  <p>Member</p>",
          "<% end %>",
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          "- if @user.admin?",
          "  %p Admin",
          "",
          "- else",
          "",
          "  %p Member",
        ),
        after: text("- if @user.admin?", "  %p Admin", "- else", "  %p Member"),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          "- if @user.admin?",
          "  p Admin",
          "",
          "- else",
          "",
          "  p Member",
        ),
        after: text("- if @user.admin?", "  p Admin", "- else", "  p Member"),
      },
    ],
  },
  {
    name: "Layout/SpaceInHtmlTag",
    summary: "One space between attributes, none before `>` or around `=`.",
    languages: ["erb"],
    type: "autofix",
    safe: true,
    safeNote: "Whitespace inside a start tag doesn't change the element.",
    status: "planned",
    phase: 1,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<div  class="card"   id="user_<%= user.id %>" >',
          '  <img src = "<%= user.avatar_url %>"  alt="">',
          "</div>",
        ),
        after: text(
          '<div class="card" id="user_<%= user.id %>">',
          '  <img src="<%= user.avatar_url %>" alt="">',
          "</div>",
        ),
      },
    ],
  },
  {
    name: "Layout/SpaceInsideErbTag",
    summary:
      "One space inside the delimiters of an ERB tag: `<%= x %>`, not `<%=x%>` or `<%=  x  %>`.",
    languages: ["erb"],
    type: "autofix",
    safe: true,
    safeNote:
      "The spaces are outside the Ruby code, so the compiled template is the same.",
    status: "partial",
    phase: 1,
    today:
      'A code action does it, not the formatter. "Add spaces inside this ERB tag" is offered on the tags under the cursor, and "Add spaces inside every ERB tag" is the source action `source.erbSpacing`, which `editor.codeActionsOnSave` can run. It handles one-line code and output tags; comments (`<%#`), literal `<%%` and tags that span lines are left alone. Format Document and format on save do not apply it. Phase 1 makes it a formatter rule.',
    setting: {
      label: "Run it on save",
      json: `{
  "[erb]": {
    "editor.codeActionsOnSave": { "source.erbSpacing": "explicit" }
  }
}`,
    },
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<p><%=user.name%></p>",
          "<p><%=  user.email  %></p>",
          "<%if user.admin?%>",
          "  <span>Admin</span>",
          "<%end%>",
        ),
        after: text(
          "<p><%= user.name %></p>",
          "<p><%= user.email %></p>",
          "<% if user.admin? %>",
          "  <span>Admin</span>",
          "<% end %>",
        ),
      },
    ],
  },
  {
    name: "Layout/SpaceAfterScriptMarker",
    summary: "A space after the `=` or `-` that starts a Ruby line.",
    languages: ["haml", "slim"],
    type: "autofix",
    safe: true,
    safeNote:
      "The marker and the code are the same tokens with a space between.",
    status: "planned",
    phase: 1,
    examples: [
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("-if @user", '  =render "users/card", user: @user'),
        after: text("- if @user", '  = render "users/card", user: @user'),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("-if @user", '  =render "users/card", user: @user'),
        after: text("- if @user", '  = render "users/card", user: @user'),
      },
    ],
  },
  {
    name: "Layout/IndentationStyle",
    summary: "Indent with tabs or with spaces.",
    languages: ["erb"],
    type: "autofix",
    safe: true,
    safeNote:
      "Leading whitespace between tags renders as nothing. Content where whitespace counts is left alone.",
    status: "partial",
    phase: 1,
    today:
      "The width is available: the editor's tab size, then `.editorconfig` (`indent_size`, or `tab_width` for `indent_size = tab`), then 2. The formatter always writes spaces; choosing tabs is the planned part.",
    config: text(
      "Layout/IndentationStyle:",
      "  Enabled: true",
      "  EnforcedStyle: tabs # or spaces, or keep",
    ),
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<ul>",
          "  <% @users.each do |user| %>",
          "    <li><%= user.name %></li>",
          "  <% end %>",
          "</ul>",
        ),
        after: text(
          "<ul>",
          "\t<% @users.each do |user| %>",
          "\t\t<li><%= user.name %></li>",
          "\t<% end %>",
          "</ul>",
        ),
        note: "With `EnforcedStyle: tabs`.",
      },
    ],
  },
  {
    name: "Layout/TrailingWhitespace",
    summary: "No spaces or tabs at the end of a line.",
    languages: ["erb", "haml", "slim"],
    type: "autofix",
    safe: true,
    safeNote:
      "Whitespace that is content stays: `<pre>` and `<textarea>` bodies, ERB comments, Haml filters, and Slim text lines (`| text`, text after a tag).",
    status: "available",
    phase: 1,
    today:
      "The formatter does it in ERB, Haml and Slim, on Format Document, on save and in `ruby-templates format`. Phase 1 only moves it into the config file.",
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<ul>  ",
          '  <li class="user">  ',
          "    <%= user.name %>\t",
          "  </li>",
          "</ul> ",
        ),
        after: text(
          "<ul>",
          '  <li class="user">',
          "    <%= user.name %>",
          "  </li>",
          "</ul>",
        ),
        note: "The spaces and tabs at the line ends are invisible here.",
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("%ul  ", "  %li.user  ", "    = user.name \t"),
        after: text("%ul", "  %li.user", "    = user.name"),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          "ul  ",
          "  li.user  ",
          '    a href="/users"   ',
          "      = user.name",
        ),
        after: text(
          "ul",
          "  li.user",
          '    a href="/users"',
          "      = user.name",
        ),
      },
    ],
  },
  {
    name: "Layout/TrailingEmptyLines",
    summary: "The file ends with exactly one line break.",
    languages: ["erb", "haml", "slim"],
    availableIn: ["haml", "slim"],
    type: "autofix",
    safe: true,
    safeNote: "Whitespace after the last tag renders as nothing.",
    status: "partial",
    phase: 1,
    today:
      "Haml and Slim files get one final line break and lose the blank lines before it. ERB files keep the end of the file as it is: blank lines stay, and no line break is added. Phase 1 moves the rule into the config and adds ERB.",
    examples: [
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("%h1 Users", "", ""),
        after: text("%h1 Users"),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("h1 Users", "", ""),
        after: text("h1 Users"),
      },
    ],
  },
  {
    name: "Layout/ElementContentOnOwnLine",
    summary:
      "The content of a block-level element on its own indented line between the open and close tags.",
    languages: ["erb", "haml", "slim"],
    type: "autofix",
    safe: false,
    safeNote:
      'It adds whitespace. Around block-level content that is invisible, but inside an inline element such as `<a>` or `<span>` the new line break shows as a space, so only the tags in the list are touched, and `<a class="btn"><%= name %></a>` stays as it is.',
    status: "planned",
    phase: 3,
    config: text(
      "Layout/ElementContentOnOwnLine:",
      "  Enabled: true",
      "  Tags: [div, section, article, header, footer, ul, ol]",
    ),
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<div class="card" id="<%= dom_id(user) %>"><%= user.name %></div>',
        ),
        after: text(
          '<div class="card" id="<%= dom_id(user) %>">',
          "  <%= user.name %>",
          "</div>",
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("%section.card#user= user.name"),
        after: text("%section.card#user", "  = user.name"),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("section.card#user = user.name"),
        after: text("section.card#user", "  = user.name"),
      },
    ],
  },
  {
    name: "Layout/AttributesPerLine",
    summary: "A start tag longer than the limit gets one attribute per line.",
    languages: ["erb"],
    type: "autofix",
    safe: true,
    safeNote: "Whitespace between attributes doesn't change the element.",
    status: "planned",
    phase: 4,
    config: text(
      "Layout/AttributesPerLine:",
      "  Enabled: true",
      "  MaxLineLength: 100",
    ),
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<input type="email" name="user[email]" id="user_email" class="form-control form-control-lg" placeholder="Email" required>',
        ),
        after: text(
          "<input",
          '  type="email"',
          '  name="user[email]"',
          '  id="user_email"',
          '  class="form-control form-control-lg"',
          '  placeholder="Email"',
          "  required>",
        ),
        note: "With a limit of 100 columns. Where the closing `>` goes is not settled.",
      },
    ],
  },
  {
    name: "Layout/LineLength",
    summary: "A line longer than the limit.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 3,
    config: text("Layout/LineLength:", "  Enabled: true", "  Max: 100"),
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<p class="lead text-muted"><%= link_to "Read the full announcement", announcement_path(@announcement), class: "link" %></p>',
        ),
        after: text(
          '<p class="lead text-muted">',
          '  <%= link_to "Read the full announcement", announcement_path(@announcement), class: "link" %>',
          "</p>",
        ),
        note: "With a limit of 100 columns.",
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          '%p.lead.text-muted= link_to "Read the full announcement", announcement_path(@announcement), class: "link"',
        ),
        after: text(
          "%p.lead.text-muted",
          '  = link_to "Read the full announcement", announcement_path(@announcement), class: "link"',
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          'p.lead.text-muted = link_to "Read the full announcement", announcement_path(@announcement), class: "link"',
        ),
        after: text(
          "p.lead.text-muted",
          '  = link_to "Read the full announcement", announcement_path(@announcement), class: "link"',
        ),
      },
    ],
  },

  // ---- Style ----
  {
    name: "Style/ErbCommentSyntax",
    summary:
      "An ERB comment written `<%#`, not as Ruby code that starts with `#`.",
    languages: ["erb"],
    type: "autofix",
    safe: true,
    safeNote:
      "Both forms output nothing. Only comments on one line are rewritten.",
    status: "planned",
    phase: 1,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<% # TODO: show the avatar %>",
          "<p><%= user.name %></p>",
        ),
        after: text("<%# TODO: show the avatar %>", "<p><%= user.name %></p>"),
      },
    ],
  },
  {
    name: "Style/AttributeQuotes",
    summary:
      "Attribute values in double quotes (or single, by choice), with none left unquoted.",
    languages: ["erb"],
    type: "autofix",
    safe: true,
    safeNote:
      "The attribute values are the same. A value that contains the quote character is left alone.",
    status: "planned",
    phase: 3,
    config: text(
      "Style/AttributeQuotes:",
      "  Enabled: true",
      "  EnforcedStyle: double_quotes # or single_quotes",
    ),
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<div class='card' data-controller=toggle>",
          "  <%= user.name %>",
          "</div>",
        ),
        after: text(
          '<div class="card" data-controller="toggle">',
          "  <%= user.name %>",
          "</div>",
        ),
      },
    ],
  },
  {
    name: "Style/BooleanAttributes",
    summary: "A boolean attribute written without a value.",
    languages: ["erb"],
    type: "autofix",
    safe: true,
    safeNote:
      'Only the presence of a boolean attribute counts, so `disabled="disabled"` and `disabled` are the same. `checked="false"` is not touched, since it means checked.',
    status: "planned",
    phase: 3,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<input type="checkbox" checked="checked" disabled="disabled">',
        ),
        after: text('<input type="checkbox" checked disabled>'),
      },
    ],
  },
  {
    name: "Style/VoidTagClosing",
    summary: "Void elements written `<br>`, or `<br />`, throughout.",
    languages: ["erb"],
    type: "autofix",
    safe: true,
    safeNote: "HTML ignores the slash on a void element.",
    status: "planned",
    phase: 3,
    config: text(
      "Style/VoidTagClosing:",
      "  Enabled: true",
      "  EnforcedStyle: no_slash # or slash",
    ),
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text('<img src="<%= user.avatar_url %>" alt=""/>', "<br/>"),
        after: text('<img src="<%= user.avatar_url %>" alt="">', "<br>"),
      },
    ],
  },
  {
    name: "Style/TagNameCase",
    summary:
      "Lowercase tag names, with the camelCase SVG elements as the SVG standard spells them.",
    languages: ["erb", "slim"],
    type: "autofix",
    safe: true,
    safeNote:
      "HTML tag names are case-insensitive, and the HTML parser maps SVG names to their camelCase form.",
    status: "planned",
    phase: 3,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<DIV class="icon">',
          '  <svg viewBox="0 0 8 8">',
          '    <lineargradient id="fade"></lineargradient>',
          "  </svg>",
          "</DIV>",
        ),
        after: text(
          '<div class="icon">',
          '  <svg viewBox="0 0 8 8">',
          '    <linearGradient id="fade"></linearGradient>',
          "  </svg>",
          "</div>",
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("DIV.icon", "  SPAN Hi"),
        after: text("div.icon", "  span Hi"),
      },
    ],
  },
  {
    name: "Style/ImplicitDiv",
    summary: "No `div` where a class or id already implies it.",
    languages: ["haml", "slim"],
    type: "autofix",
    safe: true,
    safeNote: "Haml and Slim render a bare `.card` or `#main` as a `div`.",
    status: "planned",
    phase: 3,
    examples: [
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("%div.card", "  %div#details", "    %p= user.bio"),
        after: text(".card", "  #details", "    %p= user.bio"),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("div.card", "  div#details", "    p = user.bio"),
        after: text(".card", "  #details", "    p = user.bio"),
      },
    ],
  },
  {
    name: "Style/ClassesBeforeIds",
    summary: "Classes before the id in a Haml tag.",
    languages: ["haml"],
    type: "autofix",
    safe: true,
    safeNote:
      "Haml renders the same attributes whichever way round they are written.",
    status: "planned",
    phase: 3,
    examples: [
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("%section#profile.card.wide", "  %p= user.bio"),
        after: text("%section.card.wide#profile", "  %p= user.bio"),
      },
    ],
  },
  {
    name: "Style/HashAttributes",
    summary:
      "Haml attributes written as a Ruby hash, or in HTML style, throughout.",
    languages: ["haml"],
    type: "autofix",
    safe: true,
    safeNote:
      "Only attributes with literal values are converted; anything else is left as it is.",
    status: "planned",
    phase: 3,
    config: text(
      "Style/HashAttributes:",
      "  Enabled: true",
      "  EnforcedStyle: html # or ruby_hash",
    ),
    examples: [
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('%a.btn{href: "/users", target: "_blank"} Users'),
        after: text('%a.btn(href="/users" target="_blank") Users'),
        note: "With `EnforcedStyle: html`; `ruby_hash` converts the other way.",
      },
    ],
  },
  {
    name: "Style/ErbTrimMode",
    summary: "Both `%>` and `-%>` in one template.",
    languages: ["erb"],
    type: "warning",
    safe: false,
    safeNote:
      "There is no autofix. In Rails, `-%>` also removes the line break after an output tag, so changing one to the other can change the page.",
    status: "planned",
    phase: 3,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<% @users.each do |user| -%>",
          "  <li><%= user.name %></li>",
          "<% end %>",
        ),
        after: text(
          "<% @users.each do |user| %>",
          "  <li><%= user.name %></li>",
          "<% end %>",
        ),
      },
    ],
  },

  // ---- Rails ----
  {
    name: "Rails/StrictLocals",
    summary:
      "A `render` whose locals don't match the partial's `locals:` comment.",
    languages: ["erb", "haml", "slim", "jbuilder"],
    type: "warning",
    safe: true,
    status: "available",
    code: "strict-locals",
    today:
      'With Rails 7.1 or later in `Gemfile.lock`, a literal render that resolves to a partial with a `locals:` comment is compared with it, using Rails\' messages (`missing local`, `unknown local`, `no locals accepted`). When a render matches several files (a variant next to the base file, or two handlers), a local is missing when every file requires it and unknown when none takes it. Quick fixes "Remove `x` from the render" and "Declare `x` in the locals of …" (it edits all the files).',
    setting: {
      label: "Turn it off",
      json: `{
  "rubyTemplates.erb.disabledDiagnostics": ["strict-locals"]
}`,
    },
    examples: [
      {
        lang: "erb",
        file: "app/views/users/show.html.erb",
        before: text('<%= render "users/card", person: @user %>'),
        after: text('<%= render "users/card", user: @user %>'),
        faults: [
          {
            line: 1,
            column: 13,
            message: "missing local: :user for app/views/users/_card.html.erb",
            code: "strict-locals",
          },
        ],
        note: "`_card.html.erb` starts with `<%# locals: (user:) -%>`.",
      },
    ],
  },
  {
    name: "Rails/MissingPartial",
    summary: "A `render` of a partial or template that isn't in `app/views`.",
    languages: ["erb", "haml", "slim", "jbuilder"],
    type: "warning",
    safe: true,
    status: "available",
    code: "unresolved-render",
    today:
      'Checks literal names in `render "users/card"`, `render partial:`, `render template:` and Jbuilder\'s `json.partial!`, with Rails\' "Missing partial" or "Missing template" message. The quick fix "Create partial `users/_profile.html.erb`" creates the file next to the others, in the format and template language of the view.',
    setting: {
      label: "Turn it off",
      json: `{
  "rubyTemplates.erb.disabledDiagnostics": ["unresolved-render"]
}`,
    },
    examples: [
      {
        lang: "erb",
        file: "app/views/users/show.html.erb",
        before: text('<%= render "users/profile", user: @user %>'),
        after: text('<%= render "users/profile", user: @user %>'),
        afterFile:
          "app/views/users/show.html.erb, with app/views/users/_profile.html.erb in place",
        faults: reported("unresolved-render").faults,
      },
    ],
  },
  {
    name: "Rails/SilentRender",
    summary:
      "A `render` in a tag that outputs nothing, so the partial never reaches the page.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 2,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text('<% render "users/card", user: @user %>'),
        after: text('<%= render "users/card", user: @user %>'),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('- render "users/card", user: @user'),
        after: text('= render "users/card", user: @user'),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('- render "users/card", user: @user'),
        after: text('= render "users/card", user: @user'),
      },
    ],
  },
  {
    name: "Rails/SilentHelper",
    summary:
      "A helper that returns markup, such as `link_to` or `image_tag`, called in a tag that outputs nothing.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 2,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text('<% link_to "Home", root_path %>'),
        after: text('<%= link_to "Home", root_path %>'),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('- link_to "Home", root_path'),
        after: text('= link_to "Home", root_path'),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('- link_to "Home", root_path'),
        after: text('= link_to "Home", root_path'),
      },
    ],
  },
  {
    name: "Rails/InstanceVariableInPartial",
    summary:
      "An instance variable inside a partial, which only works for the controller that sets it.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 2,
    examples: [
      {
        lang: "erb",
        file: "app/views/users/_card.html.erb",
        before: text("<p><%= @user.name %></p>"),
        after: text("<%# locals: (user:) %>", "<p><%= user.name %></p>"),
        afterFile: "app/views/users/_card.html.erb, called with user: @user",
      },
      {
        lang: "haml",
        file: "app/views/users/_card.html.haml",
        before: text("%p= @user.name"),
        after: text("%p= user.name"),
        afterFile: "app/views/users/_card.html.haml, called with user: @user",
      },
      {
        lang: "slim",
        file: "app/views/users/_card.html.slim",
        before: text("p = @user.name"),
        after: text("p = user.name"),
        afterFile: "app/views/users/_card.html.slim, called with user: @user",
      },
    ],
  },
  {
    name: "Rails/UnusedStrictLocal",
    summary: "A local in the `locals:` comment that the partial never uses.",
    languages: ["erb"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 2,
    examples: [
      {
        lang: "erb",
        file: "app/views/users/_card.html.erb",
        before: text(
          "<%# locals: (user:, compact: false) %>",
          "<p><%= user.name %></p>",
        ),
        after: text("<%# locals: (user:) %>", "<p><%= user.name %></p>"),
      },
    ],
  },
  {
    name: "Rails/RenderInLoop",
    summary:
      "A `render` of the same partial inside a loop; `render collection:` does it in one call.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 2,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<% @users.each do |user| %>",
          '  <%= render "users/card", user: user %>',
          "<% end %>",
        ),
        after: text(
          '<%= render partial: "users/card", collection: @users, as: :user %>',
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          "- @users.each do |user|",
          '  = render "users/card", user: user',
        ),
        after: text(
          '= render partial: "users/card", collection: @users, as: :user',
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          "- @users.each do |user|",
          '  = render "users/card", user: user',
        ),
        after: text(
          '= render partial: "users/card", collection: @users, as: :user',
        ),
      },
    ],
  },
  {
    name: "Rails/DebugOutput",
    summary:
      "Debugging left in a view: `p`, `pp`, `puts`, `debug`, `binding.irb` and `binding.pry`.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 2,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<%= debug @user %>",
          "<% binding.irb %>",
          "<p><%= @user.name %></p>",
        ),
        after: text("<p><%= @user.name %></p>"),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("= debug @user", "- binding.irb", "%p= @user.name"),
        after: text("%p= @user.name"),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("= debug @user", "- binding.irb", "p = @user.name"),
        after: text("p = @user.name"),
      },
    ],
  },
  {
    name: "Rails/OutputControlFlow",
    summary:
      "Control flow in an output tag: `<%= if … %>` where `<% if … %>` is meant.",
    languages: ["erb"],
    type: "warning",
    safe: true,
    status: "partial",
    phase: 2,
    code: "ruby-syntax",
    today:
      "The template doesn't compile, so it is reported as a Ruby syntax error with Prism's messages, as `ruby-syntax`. Nothing says that the `=` is the cause. The planned rule is a message that names it.",
    setting: {
      label: "Turn it off",
      json: `{
  "rubyTemplates.erb.disabledDiagnostics": ["ruby-syntax"]
}`,
    },
    examples: [
      {
        lang: "erb",
        file: "app/views/users/show.html.erb",
        before: text("<%= if @user %>", "  <p>hi</p>", "<% end %>"),
        after: text("<% if @user %>", "  <p>hi</p>", "<% end %>"),
        faults: [
          {
            line: 1,
            column: 5,
            message: "expected an `end` to close the conditional clause",
            code: "ruby-syntax",
          },
          {
            line: 1,
            column: 14,
            message: "expected `then` or `;` or '\\n'",
            code: "ruby-syntax",
          },
          {
            line: 1,
            column: 14,
            message:
              "unexpected ')', assuming it is closing the parent parentheses",
            code: "ruby-syntax",
          },
        ],
      },
    ],
  },
  {
    name: "Rails/UnsafeOutput",
    summary:
      "Output that skips Rails' HTML escaping: `raw`, `.html_safe` and `<%==`.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 3,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<%= raw @user.bio %>",
          "<%= @user.bio.html_safe %>",
          "<%== @user.bio %>",
          '<a href="<%= raw @user.website %>">Website</a>',
          "<script>var bio = <%= raw @user.bio.to_json %>;</script>",
        ),
        after: text(
          "<%= sanitize @user.bio %>",
          "<%= sanitize @user.bio %>",
          "<%= sanitize @user.bio %>",
          '<a href="<%= @user.website %>">Website</a>',
          "<script>var bio = <%= @user.bio.to_json %>;</script>",
        ),
        note: "The same rule covers `raw` and `html_safe` in attributes and `<script>` bodies, as the last two lines show.",
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("!= @user.bio", "= raw @user.bio"),
        after: text("= sanitize @user.bio", "= sanitize @user.bio"),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("== @user.bio", "= raw @user.bio"),
        after: text("= sanitize @user.bio", "= sanitize @user.bio"),
      },
    ],
  },
  {
    name: "Rails/UnusedBlockArgument",
    summary: "A block argument the block never uses.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 2,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<% @users.each do |user, index| %>",
          "  <li><%= user.name %></li>",
          "<% end %>",
        ),
        after: text(
          "<% @users.each do |user| %>",
          "  <li><%= user.name %></li>",
          "<% end %>",
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("- @users.each do |user, index|", "  %li= user.name"),
        after: text("- @users.each do |user|", "  %li= user.name"),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("- @users.each do |user, index|", "  li = user.name"),
        after: text("- @users.each do |user|", "  li = user.name"),
      },
    ],
  },
  {
    name: "Rails/I18nMissingKey",
    summary:
      'A lazy translation key, such as `t(".title")`, with no entry in `config/locales`.',
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 4,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text('<h1><%= t(".titel") %></h1>'),
        after: text('<h1><%= t(".title") %></h1>'),
        note: "`config/locales/en.yml` has `en.users.index.title`.",
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('%h1= t(".titel")'),
        after: text('%h1= t(".title")'),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('h1 = t(".titel")'),
        after: text('h1 = t(".title")'),
      },
    ],
  },
  {
    name: "Rails/UnknownRoute",
    summary:
      "A `*_path` or `*_url` helper that `config/routes.rb` doesn't define.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 4,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text('<%= link_to "Users", usrs_path %>'),
        after: text('<%= link_to "Users", users_path %>'),
        note: "`config/routes.rb` has `resources :users`.",
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('= link_to "Users", usrs_path'),
        after: text('= link_to "Users", users_path'),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('= link_to "Users", usrs_path'),
        after: text('= link_to "Users", users_path'),
      },
    ],
  },

  // ---- Turbo and Stimulus ----
  {
    name: "Turbo/PreferTurboAttributes",
    summary:
      "The Turbo attributes in place of the Rails UJS ones: `data-turbo-confirm`, `data-turbo-method` and `data-turbo-submits-with`.",
    languages: ["erb", "haml", "slim"],
    type: "autofix",
    safe: false,
    safeNote:
      "It renames HTML attributes. The page behaves the same only in an app that runs Turbo and not Rails UJS, which is what the rule assumes.",
    status: "planned",
    phase: 2,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<%= link_to "Delete", user_path(user), data: { confirm: "Sure?", method: :delete } %>',
          '<%= form.submit "Save", data: { disable_with: "Saving..." } %>',
          '<a href="/users/1" data-confirm="Sure?" data-method="delete">Delete</a>',
        ),
        after: text(
          '<%= link_to "Delete", user_path(user), data: { turbo_confirm: "Sure?", turbo_method: :delete } %>',
          '<%= form.submit "Save", data: { turbo_submits_with: "Saving..." } %>',
          '<a href="/users/1" data-turbo-confirm="Sure?" data-turbo-method="delete">Delete</a>',
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          '= link_to "Delete", user_path(user), data: { confirm: "Sure?", method: :delete }',
        ),
        after: text(
          '= link_to "Delete", user_path(user), data: { turbo_confirm: "Sure?", turbo_method: :delete }',
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          '= link_to "Delete", user_path(user), data: { confirm: "Sure?", method: :delete }',
        ),
        after: text(
          '= link_to "Delete", user_path(user), data: { turbo_confirm: "Sure?", turbo_method: :delete }',
        ),
      },
    ],
  },
  {
    name: "Turbo/PermanentRequiresId",
    summary:
      "An element with `data-turbo-permanent` and no `id`, which Turbo needs to match it across pages.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 2,
    examples: [
      {
        lang: "erb",
        file: "app/views/layouts/application.html.erb",
        before: text(
          "<div data-turbo-permanent>",
          "  <%= render 'shared/player' %>",
          "</div>",
        ),
        after: text(
          '<div id="player" data-turbo-permanent>',
          "  <%= render 'shared/player' %>",
          "</div>",
        ),
      },
      {
        lang: "haml",
        file: "app/views/layouts/application.html.haml",
        before: text(
          "%div{data: {turbo_permanent: true}}",
          "  = render 'shared/player'",
        ),
        after: text(
          "#player{data: {turbo_permanent: true}}",
          "  = render 'shared/player'",
        ),
      },
      {
        lang: "slim",
        file: "app/views/layouts/application.html.slim",
        before: text(
          "div data-turbo-permanent=true",
          "  = render 'shared/player'",
        ),
        after: text(
          "#player data-turbo-permanent=true",
          "  = render 'shared/player'",
        ),
      },
    ],
  },
  {
    name: "Stimulus/UnknownController",
    summary:
      "A `data-controller` name with no controller file in `app/javascript/controllers`.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 2,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<div data-controller="serch">',
          "  <%= text_field_tag :q %>",
          "</div>",
        ),
        after: text(
          '<div data-controller="search">',
          "  <%= text_field_tag :q %>",
          "</div>",
        ),
        note: "`app/javascript/controllers/search_controller.js` exists.",
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          '%div{data: {controller: "serch"}}',
          "  = text_field_tag :q",
        ),
        after: text(
          '%div{data: {controller: "search"}}',
          "  = text_field_tag :q",
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('div data-controller="serch"', "  = text_field_tag :q"),
        after: text('div data-controller="search"', "  = text_field_tag :q"),
      },
    ],
  },
  {
    name: "Stimulus/UnknownTargetOrAction",
    summary:
      "A `data-*-target` or `data-action` that names a target or method its controller doesn't have.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 4,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<div data-controller="search">',
          '  <input data-search-target="querry" data-action="input->search#sbumit">',
          "</div>",
        ),
        after: text(
          '<div data-controller="search">',
          '  <input data-search-target="query" data-action="input->search#submit">',
          "</div>",
        ),
        note: '`search_controller.js` has `static targets = ["query"]` and a `submit()` method.',
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          '%div{data: {controller: "search"}}',
          '  %input{data: {search_target: "querry", action: "input->search#sbumit"}}',
        ),
        after: text(
          '%div{data: {controller: "search"}}',
          '  %input{data: {search_target: "query", action: "input->search#submit"}}',
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          'div data-controller="search"',
          '  input data-search-target="querry" data-action="input->search#sbumit"',
        ),
        after: text(
          'div data-controller="search"',
          '  input data-search-target="query" data-action="input->search#submit"',
        ),
      },
    ],
  },

  // ---- HTML and accessibility ----
  {
    name: "Html/ImgAlt",
    summary: "An image without alt text.",
    languages: ["erb", "haml", "slim"],
    availableIn: ["erb"],
    type: "warning",
    safe: true,
    status: "partial",
    phase: 3,
    code: "img-missing-alt",
    today:
      'In ERB with an HTML host, for `<img>` and for `image_tag` in an output tag without `alt:`. An empty `alt=""` counts as alt text. Haml and Slim templates are not checked.',
    setting: {
      label: "Turn it off",
      json: `{
  "rubyTemplates.erb.disabledDiagnostics": ["img-missing-alt"]
}`,
    },
    examples: [
      {
        lang: "erb",
        file: "app/views/users/show.html.erb",
        before: text(
          '<img src="<%= @user.avatar_url %>">',
          "<%= image_tag @user.avatar_url %>",
        ),
        after: text(
          '<img src="<%= @user.avatar_url %>" alt="<%= @user.name %>">',
          "<%= image_tag @user.avatar_url, alt: @user.name %>",
        ),
        faults: [
          {
            line: 1,
            column: 1,
            message:
              "`<img>` without `alt`: the alt attribute must be specified (HTML 4.8.4.4.1)",
            code: "img-missing-alt",
          },
          {
            line: 2,
            column: 5,
            message:
              "`image_tag` without `alt:`: the alt attribute must be specified (HTML 4.8.4.4.1)",
            code: "img-missing-alt",
          },
        ],
      },
    ],
  },
  {
    name: "Html/DuplicateId",
    summary: "The same static id twice in one template.",
    languages: ["erb", "haml", "slim"],
    availableIn: ["erb"],
    type: "warning",
    safe: true,
    status: "partial",
    phase: 3,
    code: "duplicate-id",
    today:
      "In ERB with an HTML host, at the second occurrence. Ids that hold ERB, ids in different branches of a conditional and content inside `<template>` are left alone. Haml and Slim templates are not checked.",
    setting: {
      label: "Turn it off",
      json: `{
  "rubyTemplates.erb.disabledDiagnostics": ["duplicate-id"]
}`,
    },
    examples: [
      {
        lang: "erb",
        ...reported("duplicate-id"),
        after: text(
          '<div id="profile">',
          "  <%= @user.name %>",
          "</div>",
          '<div id="settings"></div>',
        ),
      },
    ],
  },
  {
    name: "Html/DuplicateAttribute",
    summary: "An attribute given twice on one tag.",
    languages: ["erb", "haml", "slim"],
    availableIn: ["slim"],
    type: "warning",
    safe: true,
    status: "partial",
    phase: 3,
    code: "syntax-error",
    today:
      "Slim itself rejects a repeated attribute other than `class`, and the language server reports Slim's error, `Multiple href attributes specified`, as `syntax-error`. ERB and Haml templates are not checked; phase 3 adds them and a code of its own.",
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<div class="card" class="wide">',
          "  <%= user.name %>",
          "</div>",
        ),
        after: text('<div class="card wide">', "  <%= user.name %>", "</div>"),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('%a{href: "/users", href: "/people"} Users'),
        after: text('%a{href: "/users"} Users'),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('a href="/users" href="/people" Users'),
        after: text('a href="/users" Users'),
        faults: [
          {
            line: 1,
            column: 1,
            message: "Multiple href attributes specified",
            code: "syntax-error",
          },
        ],
      },
    ],
  },
  {
    name: "Html/NestedInteractive",
    summary:
      "An interactive element inside another: a link in a link, a form in a form, including `link_to` and `form_with`.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 4,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<%= link_to user_path(user) do %>",
          '  <%= link_to "Edit", edit_user_path(user) %>',
          "<% end %>",
        ),
        after: text(
          "<%= link_to user_path(user) do %>",
          "  <%= user.name %>",
          "<% end %>",
          '<%= link_to "Edit", edit_user_path(user) %>',
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('%a{href: "/users"}', '  %a{href: "/users/new"} New'),
        after: text('%a{href: "/users"} Users', '%a{href: "/users/new"} New'),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('a href="/users"', '  a href="/users/new" New'),
        after: text('a href="/users" Users', 'a href="/users/new" New'),
      },
    ],
  },
  {
    name: "Html/AnchorHref",
    summary:
      "An `<a>` without an `href`, which is not a link and cannot be reached with the keyboard.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 3,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text('<a class="btn" data-action="click->menu#open">Menu</a>'),
        after: text(
          '<button class="btn" data-action="click->menu#open">Menu</button>',
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('%a.btn{data: {action: "click->menu#open"}} Menu'),
        after: text('%button.btn{data: {action: "click->menu#open"}} Menu'),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('a.btn data-action="click->menu#open" Menu'),
        after: text('button.btn data-action="click->menu#open" Menu'),
      },
    ],
  },
  {
    name: "A11y/IframeTitle",
    summary:
      "An `<iframe>` without a `title`, which screen readers use to name it.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 3,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<iframe src="https://www.youtube.com/embed/<%= video.youtube_id %>"></iframe>',
        ),
        after: text(
          '<iframe src="https://www.youtube.com/embed/<%= video.youtube_id %>" title="<%= video.title %>"></iframe>',
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          '%iframe{src: "https://www.youtube.com/embed/#{video.youtube_id}"}',
        ),
        after: text(
          '%iframe{src: "https://www.youtube.com/embed/#{video.youtube_id}", title: video.title}',
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          'iframe src="https://www.youtube.com/embed/#{video.youtube_id}"',
        ),
        after: text(
          'iframe src="https://www.youtube.com/embed/#{video.youtube_id}" title=video.title',
        ),
      },
    ],
  },
  {
    name: "A11y/GenericLinkText",
    summary:
      'Link text that says nothing out of context: "click here", "here", "read more".',
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "planned",
    phase: 3,
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<p>Our new pricing is live. <%= link_to "Click here", pricing_path %></p>',
        ),
        after: text(
          '<p><%= link_to "See the new pricing", pricing_path %></p>',
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('%p= link_to "Click here", pricing_path'),
        after: text('%p= link_to "See the new pricing", pricing_path'),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('p = link_to "Click here", pricing_path'),
        after: text('p = link_to "See the new pricing", pricing_path'),
      },
    ],
  },
];

export const departmentOf = (rule: Rule) => {
  const prefix = rule.name.split("/")[0];
  const department = DEPARTMENTS.find((d) =>
    (d.prefixes as readonly string[]).includes(prefix),
  );
  if (!department)
    throw new Error(`rules.ts: \`${rule.name}\` is in no department`);
  return department;
};

const listFormat = new Intl.ListFormat("en", { type: "conjunction" });
export const languageList = (languages: Language[]) =>
  listFormat.format(languages.map((lang) => LANGUAGE_NAMES[lang]));

/** The id of a rule's section: `Layout/EmptyLines` is `layout-emptylines`. */
export const ruleId = (rule: Rule) => rule.name.toLowerCase().replace("/", "-");

export const statusLabel = (rule: Rule) =>
  ({ available: "Available", partial: "Partial", planned: "Planned" })[
    rule.status
  ] + (rule.status === "planned" ? `, phase ${rule.phase}` : "");

/** The rules as a Markdown task list grouped by phase; the ones the extension does today are ticked. */
export function checklist(list: Rule[] = rules): string {
  const groups = Map.groupBy(
    list,
    (rule) =>
      rule.phase ?? (rule.status === "available" ? "available" : "none"),
  );
  const heading = (key: number | string) =>
    typeof key === "number"
      ? `Phase ${key}`
      : key === "available"
        ? "Available, no phase"
        : "Not scheduled yet";
  const keys = [...groups.keys()].sort((a, b) => rank(a) - rank(b));
  return keys
    .map((key) => {
      const items = groups.get(key)!.map((rule) => {
        const mark = rule.status === "available" ? "x" : " ";
        const partial = rule.status === "partial" ? " (partial)" : "";
        return `- [${mark}] ${rule.name} — ${rule.summary}${partial}`;
      });
      return `### ${heading(key)}\n\n${items.join("\n")}`;
    })
    .join("\n\n");
}

const rank = (key: number | string) =>
  typeof key === "number" ? key : key === "none" ? 1000 : 1001;

/** Fails the build on a rule the page can't show properly. */
function checkRules(list: Rule[]) {
  const names = new Set<string>();
  const problems = list.flatMap((rule) => {
    const found: string[] = [];
    if (names.has(rule.name)) found.push("is listed twice");
    names.add(rule.name);
    departmentOf(rule);
    if (rule.status === "planned" && !rule.phase)
      found.push("is planned without a phase");
    if (rule.status !== "planned" && !rule.today)
      found.push("exists but doesn't say how it works today");
    if (rule.status === "planned" && (rule.today || rule.code || rule.setting))
      found.push("is planned but describes today's behaviour");
    if (!rule.safe && !rule.safeNote)
      found.push("is unsafe without saying why");
    if (rule.availableIn && rule.status !== "partial")
      found.push("lists availableIn but isn't partial");
    if (rule.availableIn?.some((lang) => !rule.languages.includes(lang)))
      found.push("is available in a language it doesn't list");
    if (rule.code && !diagnostics.some((d) => d.code === rule.code))
      found.push(`links the unknown diagnostic \`${rule.code}\``);
    if (!rule.examples.length) found.push("has no example");
    return found.map((problem) => `${rule.name} ${problem}`);
  });
  if (problems.length)
    throw new Error(`site/src/data/rules.ts:\n  ${problems.join("\n  ")}`);
}

checkRules(rules);
