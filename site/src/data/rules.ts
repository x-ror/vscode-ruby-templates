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
  /** The `.ruby-templates.toml` table that turns it off. */
  setting?: { label: string; code: string };
  /** The languages where the rule runs without a `.ruby-templates.toml` entry; shown for formatting rules. */
  onByDefault?: Language[];
  /** The `.ruby-templates.toml` table that turns the rule on, with its options; planned until the rule is available. */
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
      "Browsers collapse the whitespace between elements, so a shorter run of blank lines renders the same. Content where whitespace counts is left alone: `<pre>`, `<textarea>`, `<script>`, `<style>`, quoted attribute values, comments and the Ruby in ERB tags in ERB, filters in Haml, text blocks and engine bodies in Slim. Two things can\'t be detected: elements that CSS styles `white-space: pre`, and blocks that capture their content and care about its whitespace (such as `<%= markdown do %>`). With `max = 0`, paragraphs there can join.",
    status: "available",
    onByDefault: [],
    today:
      "Runs of blank lines longer than `max` (a whole number, 1 by default; 0 removes them all) are cut to `max`, also at the start and the end of the file. Format selection leaves blank lines as they are. For Haml and Slim, the formatter checks that the compiled template stays the same, and keeps the blank lines of a file when it would not.",
    config: text("[Layout.EmptyLines]", "enabled = true", "max = 1"),
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
    safeNote:
      "Only blank lines between tags change, which render as whitespace between them, and browsers collapse that. Content where whitespace counts is left alone: `<pre>`, `<textarea>`, `<script>`, `<style>`, attribute values, comments and the Ruby in ERB tags in ERB, filters in Haml, text blocks and engine bodies in Slim. Two things can\'t be detected: elements that CSS styles `white-space: pre`, and blocks that capture their content and care about its whitespace (such as `<%= markdown do %>`).",
    status: "available",
    onByDefault: [],
    today:
      "`style` is `no_empty_lines` (the default) or `empty_lines`, which keeps exactly one blank line there and adds it where there is none. In ERB the body is what lies between the opening tag and the closing tag or `<% end %>`. Haml and Slim have no closing line, so only the start of the body is covered: the blank lines after `- if` or `- each do` are set, and those before the next line at the same or a lower depth are siblings\' separators and stay. A block with nothing in it loses its blank lines under `no_empty_lines` and is left alone under `empty_lines`. The lines before and after `else` and `when` belong to `Layout/EmptyLinesAroundBranches`, and `Layout/EmptyLines` `max` is applied after this rule, so `max = 0` wins over `empty_lines`. Format selection leaves blank lines as they are. For Haml and Slim, the formatter checks that the compiled template stays the same, and keeps the blank lines of a file when it would not.",
    config: text(
      "[Layout.EmptyLinesAroundBlockBody]",
      "enabled = true",
      'style = "no_empty_lines" # or empty_lines',
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
          "%footer Done",
        ),
        after: text(
          "- if @admin_section",
          "  - @reports.each do |report|",
          "    %p= report.title",
          "",
          "%footer Done",
        ),
        note: "Haml and Slim have no closing line, so only the blank lines after `- if` and `- each` go; the one before `%footer` separates siblings.",
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
          "footer Done",
        ),
        after: text(
          "- if @admin_section",
          "  - @reports.each do |report|",
          "    p = report.title",
          "",
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
    safeNote:
      "The blank lines sit between tags, where they render as whitespace that browsers collapse. Content where whitespace counts is left alone: `<pre>`, `<textarea>`, `<script>`, `<style>`, attribute values, comments and the Ruby in ERB tags in ERB, filters in Haml, text blocks and engine bodies in Slim. Elements that CSS styles `white-space: pre` can\'t be detected.",
    status: "available",
    onByDefault: [],
    today:
      "`style` is `no_empty_lines` (the default) or `empty_lines`, which keeps exactly one blank line there and adds it where there is none. In ERB the body is what lies between the start tag and the end tag. Haml and Slim have no closing line, so only the start of the body is covered: the blank lines after `%section` or `section` are set, and those before the next line at the same or a lower depth are siblings\' separators and stay. An element with nothing in it (`<div>` and `</div>` on two lines) loses its blank lines under `no_empty_lines` and is left alone under `empty_lines`. `Layout/EmptyLines` `max` is applied after this rule, so `max = 0` wins over `empty_lines`. Format selection leaves blank lines as they are. For Haml and Slim, the formatter checks that the compiled template stays the same, and keeps the blank lines of a file when it would not.",
    config: text(
      "[Layout.EmptyLinesAroundElementBody]",
      "enabled = true",
      'style = "no_empty_lines" # or empty_lines',
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
          "%footer Done",
        ),
        after: text(
          "%section.users",
          "  %h2 Users",
          "  %p Active this week",
          "",
          "%footer Done",
        ),
        note: "Haml and Slim have no closing line, so only the blank line after `%section.users` goes; the one before `%footer` separates siblings.",
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
          "footer Done",
        ),
        after: text(
          "section.users",
          "  h2 Users",
          "  p Active this week",
          "",
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
    safeNote:
      "The blank lines sit between tags, where they render as whitespace that browsers collapse. Content where whitespace counts is left alone, as for `Layout/EmptyLines`. Two things can\'t be detected: elements that CSS styles `white-space: pre`, and blocks that capture their content and care about its whitespace (such as `<%= markdown do %>`).",
    status: "available",
    onByDefault: [],
    today:
      "`style` is `no_empty_lines` (the default) or `empty_lines`, which keeps exactly one blank line before and after the line and adds it where there is none. It covers `else`, `elsif`, `when`, `in`, `rescue` and `ensure` lines in ERB, Haml and Slim. A branch with nothing in it (`<% if a %>` directly above `<% else %>`, or `- case a` above `- when 1`) loses its blank lines under `no_empty_lines` and is left alone under `empty_lines`. `Layout/EmptyLines` `max` is applied after this rule, so `max = 0` wins over `empty_lines`. Format selection leaves blank lines as they are. For Haml and Slim, the formatter checks that the compiled template stays the same, and keeps the blank lines of a file when it would not.",
    config: text(
      "[Layout.EmptyLinesAroundBranches]",
      "enabled = true",
      'style = "no_empty_lines" # or empty_lines',
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
      "The spaces are outside the Ruby code, so the compiled template is the same. The formatter checks that the trimmed Ruby of every tag is the same before and after, and keeps the spacing of a file when it is not.",
    status: "available",
    onByDefault: [],
    config: text("[Layout.SpaceInsideErbTag]", "enabled = true"),
    today:
      'Each code and output tag that sits on one line gets one space after its opening and one before its closing delimiter, the `-` of `<%-` and `-%>` and the `=` of `<%==` included. Tags that span lines, comments (`<%#`) and literal `<%%` tags stay. Format selection applies it to the lines it covers. The code action does the same without the rule: "Add spaces inside this ERB tag" is offered on the tags under the cursor, and "Add spaces inside every ERB tag" is the source action `source.erbSpacing`.',
    setting: {
      label: "Run the code action on save, without the rule",
      code: `"[erb]": {
  "editor.codeActionsOnSave": { "source.erbSpacing": "explicit" }
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
    summary: "A space after the `=` or `-` that starts Ruby code.",
    languages: ["haml", "slim"],
    type: "autofix",
    safe: true,
    safeNote:
      "The marker and the code are the same tokens with a space between. The formatter checks that the template compiles to the same Ruby before and after, and keeps the markers of a file when it does not.",
    status: "available",
    onByDefault: [],
    config: text("[Layout.SpaceAfterScriptMarker]", "enabled = true"),
    today:
      "Where the parser reads Ruby code, the marker gets a space when the code follows it directly: `-`, `=`, `~`, `!=`, `&=`, `!~` and `&~` in Haml, `-`, `=` and `==` (with `<` and `>`) in Slim, both at the start of a line and after a tag (`%p=x`, `p=x`). Text, comments (`-#` in Haml and Slim, `/` in Slim), filters and engine bodies, `|` text blocks, attribute values, `==` and `\\=` lines in Haml (text, not code) and the continuation lines of multi-line Ruby are not touched. Also skipped: the first line of a multi-line Haml script written with `|`, the code after an attribute hash that spans lines, and Slim's `p: =x`. Format selection applies it to the lines it covers.",
    examples: [
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          "-if @user",
          "  %h1=@user.name",
          '  =render "users/card", user: @user',
        ),
        after: text(
          "- if @user",
          "  %h1= @user.name",
          '  = render "users/card", user: @user',
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          "-if @user",
          "  h1=@user.name",
          '  =render "users/card", user: @user',
        ),
        after: text(
          "- if @user",
          "  h1= @user.name",
          '  = render "users/card", user: @user',
        ),
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
      "[Layout.IndentationStyle]",
      "enabled = true",
      'style = "tabs" # or spaces, or keep',
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
        note: 'With `style = "tabs"`.',
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
    type: "autofix",
    safe: false,
    safeNote:
      'Safe in Haml and Slim, which ignore the blank lines at the end. Not safe in ERB: Rails renders a template or partial byte for byte, so a final line break or the blank lines at the end become output. A partial `_icon.html.erb` that is `<svg></svg>` renders with a line break after it once the rule adds one, and that shows as a space in `<button><%= render "icon" %>Save</button>`.',
    status: "available",
    onByDefault: ["haml", "slim"],
    config: text("[Layout.TrailingEmptyLines]", "enabled = true"),
    today:
      "The blank lines at the end of the file go, and a file that doesn't end with a line break gets one, using the line ending the file mostly has. It is on for Haml and Slim and off for ERB, where `enabled = true` turns it on, and `include` limits it to some files. Format selection leaves the end of the file as it is.",
    setting: {
      label: "Turn it off (Haml and Slim)",
      code: `[Layout.TrailingEmptyLines]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text("<h1>Users</h1>", "", ""),
        after: text("<h1>Users</h1>"),
      },
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
      "[Layout.ElementContentOnOwnLine]",
      "enabled = true",
      'tags = ["div", "section", "article", "header", "footer", "ul", "ol"]',
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
      "[Layout.AttributesPerLine]",
      "enabled = true",
      "max_line_length = 100",
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
    config: text("[Layout.LineLength]", "enabled = true", "max = 100"),
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
      "Both forms output nothing. Only a tag that is nothing but comment and stands alone on its line is rewritten: beside other content, Rails comments out the output that follows a `<% #` tag on the same line, and `<%#` would bring it back.",
    status: "available",
    onByDefault: [],
    config: text("[Style.ErbCommentSyntax]", "enabled = true"),
    today:
      "`<% # note %>` becomes `<%# note %>`, keeping a `-%>` as it is. A tag over several lines is rewritten when every line is a comment or blank, since `<%#` comments out everything up to `%>`. `<%= # x %>` is not a comment and stays, `<%- # x -%>` is skipped to be conservative, and so is a tag with content before or after it on its line (it usually breaks the template: Rails comments out what follows it), and the encoding comment. The ERB formatter checks that the trimmed Ruby of every tag is the same afterwards.",
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
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<% # a comment over",
          "   # two lines %>",
          "<%- # a trimming tag %>",
        ),
        after: text(
          "<%# a comment over",
          "   # two lines %>",
          "<%- # a trimming tag %>",
        ),
        note: "The second tag stays: `<%-` is not a comment tag.",
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
      "[Style.AttributeQuotes]",
      "enabled = true",
      'style = "double_quotes" # or single_quotes',
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
      "[Style.VoidTagClosing]",
      "enabled = true",
      'style = "no_slash" # or slash',
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
      "[Style.HashAttributes]",
      "enabled = true",
      'style = "html" # or ruby_hash',
    ),
    examples: [
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('%a.btn{href: "/users", target: "_blank"} Users'),
        after: text('%a.btn(href="/users" target="_blank") Users'),
        note: 'With `style = "html"`; `"ruby_hash"` converts the other way.',
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
    code: "Rails/StrictLocals",
    today:
      'With Rails 7.1 or later in `Gemfile.lock`, a literal render that resolves to a partial with a `locals:` comment is compared with it, using Rails\' messages (`missing local`, `unknown local`, `no locals accepted`). When a render matches several files (a variant next to the base file, or two handlers), a local is missing when every file requires it and unknown when none takes it. Quick fixes "Remove `x` from the render" and "Declare `x` in the locals of …" (it edits all the files).',
    setting: {
      label: "Turn it off",
      code: `[Rails.StrictLocals]
enabled = false`,
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
            code: "Rails/StrictLocals",
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
    code: "Rails/MissingPartial",
    today:
      'Checks literal names in `render "users/card"`, `render partial:`, `render template:` and Jbuilder\'s `json.partial!`, with Rails\' "Missing partial" or "Missing template" message. The quick fix "Create partial `users/_profile.html.erb`" creates the file next to the others, in the format and template language of the view.',
    setting: {
      label: "Turn it off",
      code: `[Rails.MissingPartial]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        file: "app/views/users/show.html.erb",
        before: text('<%= render "users/profile", user: @user %>'),
        after: text('<%= render "users/profile", user: @user %>'),
        afterFile:
          "app/views/users/show.html.erb, with app/views/users/_profile.html.erb in place",
        faults: reported("Rails/MissingPartial").faults,
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
    code: "Syntax/Ruby",
    today:
      "The template doesn't compile, so it is reported as a Ruby syntax error with Prism's messages, as `Syntax/Ruby`. Nothing says that the `=` is the cause. The planned rule is a message that names it.",
    setting: {
      label: "Turn it off",
      code: `[Syntax.Ruby]
enabled = false`,
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
            code: "Syntax/Ruby",
          },
          {
            line: 1,
            column: 14,
            message: "expected `then` or `;` or '\\n'",
            code: "Syntax/Ruby",
          },
          {
            line: 1,
            column: 14,
            message:
              "unexpected ')', assuming it is closing the parent parentheses",
            code: "Syntax/Ruby",
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
    code: "Html/ImgAlt",
    today:
      'In ERB with an HTML host, for `<img>` and for `image_tag` in an output tag without `alt:`. An empty `alt=""` counts as alt text. Haml and Slim templates are not checked.',
    setting: {
      label: "Turn it off",
      code: `[Html.ImgAlt]
enabled = false`,
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
            code: "Html/ImgAlt",
          },
          {
            line: 2,
            column: 5,
            message:
              "`image_tag` without `alt:`: the alt attribute must be specified (HTML 4.8.4.4.1)",
            code: "Html/ImgAlt",
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
    code: "Html/DuplicateId",
    today:
      "In ERB with an HTML host, at the second occurrence. Ids that hold ERB, ids in different branches of a conditional and content inside `<template>` are left alone. Haml and Slim templates are not checked.",
    setting: {
      label: "Turn it off",
      code: `[Html.DuplicateId]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        ...reported("Html/DuplicateId"),
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
    name: "Html/UnexpectedCloseTag",
    summary: "An end tag with no open element of that name.",
    languages: ["erb"],
    type: "warning",
    safe: true,
    status: "available",
    code: "Html/UnexpectedCloseTag",
    today:
      "In ERB with an HTML host, including the end tag of a void element. A stray end tag in a partial is not reported, since a partial may close what another one opened.",
    setting: {
      label: "Turn it off",
      code: `[Html.UnexpectedCloseTag]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        ...reported("Html/UnexpectedCloseTag"),
        after: text(
          '<div class="profile">',
          "  <p><%= @user.name %></p>",
          "</div>",
        ),
      },
    ],
  },
  {
    name: "Html/MismatchedCloseTag",
    summary: "An end tag that doesn't close the innermost open element.",
    languages: ["erb"],
    type: "warning",
    safe: true,
    status: "available",
    code: "Html/MismatchedCloseTag",
    today:
      "In ERB with an HTML host, partials included. When the end tag crosses an open element, the browser could recover in more than one way, so checking stops there.",
    setting: {
      label: "Turn it off",
      code: `[Html.MismatchedCloseTag]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        ...reported("Html/MismatchedCloseTag"),
        after: text(
          '<div class="profile">',
          "  <span><%= @user.name %></span>",
          "</div>",
        ),
      },
    ],
  },
  {
    name: "Html/UnclosedElement",
    summary: "An element whose end tag never comes.",
    languages: ["erb"],
    type: "warning",
    safe: true,
    status: "available",
    code: "Html/UnclosedElement",
    today:
      "In ERB with an HTML host, at the start tag. Elements whose end tag HTML lets you omit, void elements, partials and files that end inside an ERB block are left alone.",
    setting: {
      label: "Turn it off",
      code: `[Html.UnclosedElement]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        ...reported("Html/UnclosedElement"),
        after: text(
          '<section class="users">',
          "  <h2>Users</h2>",
          "</section>",
        ),
      },
    ],
  },
  {
    name: "Html/VoidChildren",
    summary: "Content inside a void element such as `img` or `input`.",
    languages: ["haml", "slim"],
    type: "warning",
    safe: true,
    status: "available",
    code: "Html/VoidChildren",
    today:
      "Haml reports content on the tag's line or nested under it; Slim reports lines nested under a void element, in HTML templates only.",
    setting: {
      label: "Turn it off",
      code: `[Html.VoidChildren]
enabled = false`,
    },
    examples: [
      {
        lang: "slim",
        ...reported("Html/VoidChildren"),
        after: text('img src="/logo.png" alt="Logo"'),
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
    code: "Syntax/Template",
    today:
      "Slim itself rejects a repeated attribute other than `class`, and the language server reports Slim's error, `Multiple href attributes specified`, as `Syntax/Template`. ERB and Haml templates are not checked; phase 3 adds them and a code of its own.",
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
            code: "Syntax/Template",
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
