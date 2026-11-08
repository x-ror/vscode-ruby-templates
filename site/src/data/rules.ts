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
  /** The project files the note speaks of, by path; the examples test writes them next to the template. */
  files?: Record<string, string>;
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
const PARTIAL_ERB_FILE = "app/views/users/_card.html.erb";
const PARTIAL_HAML_FILE = "app/views/users/_card.html.haml";
const PARTIAL_SLIM_FILE = "app/views/users/_card.html.slim";

const IVAR_MESSAGE =
  "`@user` is read in a partial, so it works only where the controller sets it; pass it as a local";
const LOCAL_MESSAGE =
  "`compact` is declared in `locals:` but the partial never uses it";
const LOOP_MESSAGE =
  'Partial "users/card" is rendered once per item of a loop; use `render partial: "users/card", collection: @users, as: :user`';
const ARGUMENT_MESSAGE =
  "Block argument `index` is never used; rename it `_index`";
const PERMANENT_MESSAGE =
  "`data-turbo-permanent` needs an `id`: Turbo matches the element across pages by it";
const unsafeMessage = (
  subject: string,
  code: string,
  hint = "use `sanitize` if it holds user HTML",
) => `\`${subject}\` outputs \`${code}\` without escaping; ${hint}`;
const IMG_MESSAGE =
  "`<img>` without `alt`: the alt attribute must be specified (HTML 4.8.4.4.1)";
const IMAGE_TAG_MESSAGE =
  "`image_tag` without `alt:`: the alt attribute must be specified (HTML 4.8.4.4.1)";
const DUPLICATE_ID_MESSAGE =
  "Duplicate id `profile` (first on line 1): the value must be unique amongst all the IDs in the element's tree (HTML 3.2.6)";
const HREF_MESSAGE =
  "`<a>` without `href` is not a link and the keyboard can't reach it; use a `<button>` for an action";
const TITLE_MESSAGE =
  "`<iframe>` without `title`: screen readers announce it to name the frame (WCAG 4.1.2)";
const LINK_TEXT_MESSAGE =
  "Link text `Click here` says nothing out of context; name where the link goes (WCAG 2.4.9, F84)";
const CONTROLLER_MESSAGE =
  "No Stimulus controller `serch` in app/javascript/controllers; did you mean `search`?";

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
    safeNote:
      "Whitespace inside a start tag doesn't change the element. The formatter checks that every tag has the same name, attributes and values, that a blank stays where an ERB tag in a tag had one, and that the text between tags is byte for byte the same (the blanks inside ERB tags aside), and leaves the tags alone when it does not.",
    status: "available",
    onByDefault: [],
    config: text("[Layout.SpaceInHtmlTag]", "enabled = true"),
    today:
      'In the start and end tags of ERB templates, a run of blanks between the name and the attributes or between attributes becomes one space, blanks around the `=` of an attribute and before `>` go, and `<br  />` keeps one space before `/>`. A run with a newline, a quoted value, an unquoted value, an ERB tag (`<div <%= attrs %>  class="a">` loses only the extra blank; the last space next to an ERB tag is never added or removed, and blanks beside an ERB tag in a name or value stay), a tag whose attributes are not plain enough to read (a quote after an ERB tag, say), and everything that is not markup (the bodies of `<script>`, `<style>`, `<textarea>`, `<title>`, `<xmp>`, `<iframe>`, `<noscript>`, `<noembed>`, `<noframes>` and `<plaintext>`, comments, CDATA, `a < b` text, `<%%` literals) stay. A run with a line break, a bare `\\r` included, is never touched. Format selection applies it to the lines it covers.',
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
      {
        lang: "erb",
        file: ERB_FILE,
        before: '<div  class="card"   id="u"><%= u.name %></div  >',
        after: '<div class="card" id="u"><%= u.name %></div>',
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
    status: "available",
    onByDefault: ["erb"],
    config: text(
      "[Layout.IndentationStyle]",
      "enabled = true",
      'style = "tabs" # or "spaces"',
    ),
    today:
      'ERB only; Haml and Slim always use spaces, since Slim rejects mixed indentation and Haml wants it consistent. Each indent level the formatter writes is one tab with `style = "tabs"`, or the width in spaces (the default of 2) with `"spaces"`. Only the leading whitespace of the lines the formatter indents changes: `<pre>`, `<textarea>`, `<script>`, `<style>`, `<xmp>` and `<listing>` bodies, Ruby inside tags, quoted attribute values and comments keep every byte. The lines of a tag that spans several lines move with its first line and keep their indentation relative to it, as written, so a tag attribute lined up with spaces stays lined up. Without `style`, the style comes from the first of these that has one: the `editor.insertSpaces` setting, then `.editorconfig` (`indent_style = tab` or `space`), then spaces. In an editor `insertSpaces` always decides, because the language server protocol always sends it, as it does the tab size; `.editorconfig` counts only on the command line, or through an EditorConfig extension that sets `insertSpaces`. `style` in `.ruby-templates.toml` beats all of them, and `enabled = false` always writes spaces. The width is the editor\'s tab size, then `.editorconfig` (`indent_size`, or `tab_width` for `indent_size = tab`), then 2; it only matters for spaces. Format selection applies the style to the lines it covers.',
    setting: {
      label: "Always indent with spaces",
      code: `[Layout.IndentationStyle]
enabled = false`,
    },
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
    onByDefault: ["erb", "haml", "slim"],
    config: text("[Layout.TrailingWhitespace]", "enabled = true"),
    today:
      "The formatter does it in ERB, Haml and Slim, on Format Document, on save and in `ruby-templates format`. It is on for all three; with `enabled = false` the spaces and tabs at the line ends stay as written, whitespace-only lines included.",
    setting: {
      label: "Turn it off",
      code: `[Layout.TrailingWhitespace]
enabled = false`,
    },
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
    status: "available",
    code: "Style/ErbTrimMode",
    today:
      "Every closer of the kind the template uses less is reported, `-%>` when there are as many of each. Comments (`<%#`) and literal `<%%` tags are not counted, nor are closers with more on their line (`-%>` only trims a line break). Off by default, as a style preference.",
    onByDefault: [],
    config: text("[Style.ErbTrimMode]", "enabled = true"),
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
        faults: [
          {
            line: 1,
            column: 26,
            message: "`-%>` here, but most closers in this template are `%>`",
            code: "Style/ErbTrimMode",
          },
        ],
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
    status: "available",
    code: "Rails/SilentRender",
    today:
      'In a tag that outputs nothing (`<% %>` in ERB, `-` in Haml and Slim), a `render` with arguments, also the block form (`render layout: "x" do`). Not reported when the result is assigned or passed on, or when the call is the last statement of a block, since the helper that takes the block may use its value. Only in Rails projects, in templates under app/views, since Padrino and Middleman write helper output to the page from `<% %>`. The quick fix turns the tag into an output tag when the call is alone in its tag or line.',
    setting: {
      label: "Turn it off",
      code: `[Rails.SilentRender]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text('<% render "users/card", user: @user %>'),
        after: text('<%= render "users/card", user: @user %>'),
        faults: [
          {
            line: 1,
            column: 4,
            message: "`render` in `<% %>` outputs nothing; use `<%=`",
            code: "Rails/SilentRender",
          },
        ],
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('- render "users/card", user: @user'),
        after: text('= render "users/card", user: @user'),
        faults: [
          {
            line: 1,
            column: 3,
            message: "`render` in `-` outputs nothing; use `=`",
            code: "Rails/SilentRender",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('- render "users/card", user: @user'),
        after: text('= render "users/card", user: @user'),
        faults: [
          {
            line: 1,
            column: 3,
            message: "`render` in `-` outputs nothing; use `=`",
            code: "Rails/SilentRender",
          },
        ],
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
    status: "available",
    code: "Rails/SilentHelper",
    today:
      "In a tag that outputs nothing, a call to one of these helpers, without a receiver: `link_to`, `link_to_if`, `link_to_unless`, `link_to_unless_current`, `button_to`, `mail_to`, `image_tag`, `content_tag`, `tag` and `tag.div`-style calls for HTML elements, `form_with`, `form_for`, `form_tag`, `fields_for`, `label_tag`, `text_field_tag`, `hidden_field_tag`, `submit_tag`, `simple_format`, the `number_to_*` formatters, `time_ago_in_words` and `distance_of_time_in_words`. Helpers with side effects, such as `content_for` and `provide`, helpers of your own and calls on a receiver (`form.fields_for`) are left alone. Only in Rails projects, in templates under app/views, since Padrino and Middleman write helper output to the page from `<% %>`. The quick fix turns the tag into an output tag when the call is alone in its tag or line.",
    setting: {
      label: "Turn it off",
      code: `[Rails.SilentHelper]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text('<% link_to "Home", root_path %>'),
        after: text('<%= link_to "Home", root_path %>'),
        faults: [
          {
            line: 1,
            column: 4,
            message:
              "`link_to` returns markup that `<% %>` throws away; use `<%=`",
            code: "Rails/SilentHelper",
          },
        ],
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('- link_to "Home", root_path'),
        after: text('= link_to "Home", root_path'),
        faults: [
          {
            line: 1,
            column: 3,
            message: "`link_to` returns markup that `-` throws away; use `=`",
            code: "Rails/SilentHelper",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('- link_to "Home", root_path'),
        after: text('= link_to "Home", root_path'),
        faults: [
          {
            line: 1,
            column: 3,
            message: "`link_to` returns markup that `-` throws away; use `=`",
            code: "Rails/SilentHelper",
          },
        ],
      },
    ],
  },
  {
    name: "Rails/InstanceVariableInPartial",
    summary:
      "An instance variable read inside a partial, which only works for the controllers that set it.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "available",
    code: "Rails/InstanceVariableInPartial",
    today:
      "The first read of each `@name` in a partial (a file starting with `_`), also inside `#{}`. Not reported: a variable the partial assigns itself (`@count = 0`, `@x ||= …`), `defined?(@name)`, ActionView's own (`@output_buffer`, `@virtual_path`, `@view_flow`, anything starting `@_`) and partials under app/views/layouts. Only in Rails projects, in templates under app/views.",
    setting: {
      label: "Turn it off",
      code: `[Rails.InstanceVariableInPartial]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        file: PARTIAL_ERB_FILE,
        before: text("<p><%= @user.name %></p>"),
        after: text("<%# locals: (user:) %>", "<p><%= user.name %></p>"),
        faults: [
          {
            line: 1,
            column: 8,
            message: IVAR_MESSAGE,
            code: "Rails/InstanceVariableInPartial",
          },
        ],
      },
      {
        lang: "haml",
        file: PARTIAL_HAML_FILE,
        before: text("%p= @user.name"),
        after: text("-# locals: (user:)", "%p= user.name"),
        faults: [
          {
            line: 1,
            column: 5,
            message: IVAR_MESSAGE,
            code: "Rails/InstanceVariableInPartial",
          },
        ],
      },
      {
        lang: "slim",
        file: PARTIAL_SLIM_FILE,
        before: text("p = @user.name"),
        after: text("/# locals: (user:)", "p = user.name"),
        faults: [
          {
            line: 1,
            column: 5,
            message: IVAR_MESSAGE,
            code: "Rails/InstanceVariableInPartial",
          },
        ],
      },
    ],
  },
  {
    name: "Rails/UnusedStrictLocal",
    summary: "A local in the `locals:` comment that the partial never uses.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "available",
    code: "Rails/UnusedStrictLocal",
    today:
      'A keyword in the `locals:` comment (Rails 7.1+) that the template never reads, assigns or passes on: not even as `local_assigns[:name]`, `local_assigns.fetch(:name)`, `binding.local_variable_get(:name)`, `defined?(name)` or in the default of another local. Nothing is reported when the template uses `local_assigns` as a whole (`render "x", local_assigns`, `**local_assigns`), `binding`, `local_variables` or `eval`, and `**rest` and `&block` are never reported. The quick fix removes the local and its comma from the comment.',
    setting: {
      label: "Turn it off",
      code: `[Rails.UnusedStrictLocal]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        file: PARTIAL_ERB_FILE,
        before: text(
          "<%# locals: (user:, compact: false) %>",
          "<p><%= user.name %></p>",
        ),
        after: text("<%# locals: (user:) %>", "<p><%= user.name %></p>"),
        faults: [
          {
            line: 1,
            column: 21,
            message: LOCAL_MESSAGE,
            code: "Rails/UnusedStrictLocal",
          },
        ],
      },
      {
        lang: "haml",
        file: PARTIAL_HAML_FILE,
        before: text("-# locals: (user:, compact: false)", "%p= user.name"),
        after: text("-# locals: (user:)", "%p= user.name"),
        faults: [
          {
            line: 1,
            column: 20,
            message: LOCAL_MESSAGE,
            code: "Rails/UnusedStrictLocal",
          },
        ],
      },
      {
        lang: "slim",
        file: PARTIAL_SLIM_FILE,
        before: text("/# locals: (user:, compact: false)", "p = user.name"),
        after: text("/# locals: (user:)", "p = user.name"),
        faults: [
          {
            line: 1,
            column: 20,
            message: LOCAL_MESSAGE,
            code: "Rails/UnusedStrictLocal",
          },
        ],
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
    status: "available",
    code: "Rails/RenderInLoop",
    today:
      'Inside `collection.each do |item|` or `each_with_index`, a `render "partial/name", name: item` that is all the loop does: a string literal for the partial, the block variable as its only local, and nothing else in the body (no wrapping element, no other output or code). Not reported for `map` and other iterators, `render partial:`, a partial name that is not a literal or a render with more locals. In Haml and Slim the render must be the only line nested under the loop.',
    setting: {
      label: "Turn it off",
      code: `[Rails.RenderInLoop]
enabled = false`,
    },
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
        faults: [
          {
            line: 2,
            column: 7,
            message: LOOP_MESSAGE,
            code: "Rails/RenderInLoop",
          },
        ],
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
        faults: [
          {
            line: 2,
            column: 5,
            message: LOOP_MESSAGE,
            code: "Rails/RenderInLoop",
          },
        ],
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
        faults: [
          {
            line: 2,
            column: 5,
            message: LOOP_MESSAGE,
            code: "Rails/RenderInLoop",
          },
        ],
      },
    ],
  },
  {
    name: "Rails/DebugOutput",
    summary:
      "Debugging left in a view: `p`, `pp`, `puts`, `debug`, `byebug`, `debugger`, `binding.irb` and `binding.pry`.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "available",
    code: "Rails/DebugOutput",
    today:
      "`p`, `pp`, `puts` and `debug` with arguments, `byebug`, `debugger`, `binding.irb` and `binding.pry`, when the call is a whole statement or the whole of an output tag. A local variable named `p`, a method on a receiver (`form.p`, `obj.debug`), a call used as an argument and a call under an `if` or `unless` that tests `Rails.env` are not reported. Only in Rails projects, in templates under app/views.",
    setting: {
      label: "Turn it off",
      code: `[Rails.DebugOutput]
enabled = false`,
    },
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
        faults: [
          {
            line: 1,
            column: 5,
            message: "`debug` is debugging left in the view",
            code: "Rails/DebugOutput",
          },
          {
            line: 2,
            column: 4,
            message: "`binding.irb` is debugging left in the view",
            code: "Rails/DebugOutput",
          },
        ],
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("= debug @user", "- binding.irb", "%p= @user.name"),
        after: text("%p= @user.name"),
        faults: [
          {
            line: 1,
            column: 3,
            message: "`debug` is debugging left in the view",
            code: "Rails/DebugOutput",
          },
          {
            line: 2,
            column: 3,
            message: "`binding.irb` is debugging left in the view",
            code: "Rails/DebugOutput",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("= debug @user", "- binding.irb", "p = @user.name"),
        after: text("p = @user.name"),
        faults: [
          {
            line: 1,
            column: 3,
            message: "`debug` is debugging left in the view",
            code: "Rails/DebugOutput",
          },
          {
            line: 2,
            column: 3,
            message: "`binding.irb` is debugging left in the view",
            code: "Rails/DebugOutput",
          },
        ],
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
    status: "available",
    code: "Rails/OutputControlFlow",
    today:
      "An ERB output tag (`<%=`, `<%==`) that starts with `if`, `unless`, `while`, `until`, `case`, `for` or `begin` and doesn't close it in the same tag, or with `else`, `elsif`, `when`, `end`, `rescue` or `ensure`. The quick fix removes the `=`. The Ruby syntax errors the `=` causes are not shown (`for x in y do` and `while c do` compile, but print the loop's value), so the message is the only one; other errors in the template still are. A complete expression (`<%= if a then b else c end %>`, `<%= x if y %>`) and `form_with … do` blocks are fine. Reported in every ERB template, as the syntax errors are. Haml and Slim accept `= if` with a nested block.",
    setting: {
      label: "Turn it off",
      code: `[Rails.OutputControlFlow]
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
            column: 3,
            message: "`if` in an output tag `<%=`: use `<% if … %>`",
            code: "Rails/OutputControlFlow",
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
    status: "available",
    code: "Rails/UnsafeOutput",
    today:
      '`raw x`, `x.html_safe` and `safe_concat x` are reported on the method name, and the unescaped output markers `<%==` (ERB), `!=` and `!~` (Haml) and `==` (Slim, also before an attribute value) on the marker. A `to_json` value gets the hint to use `json_escape`. Not reported when the value is certainly safe: a literal without interpolation (`"<br>".html_safe`, `raw "&nbsp;"`), a `t`/`translate` call with a literal key ending in `_html` or `.html`, or a call to `sanitize`, `safe_join`, `strip_tags`, `json_escape`, `j`, `escape_javascript`, `h`, `html_escape`, `escape_once`, `link_to`, `button_to`, `mail_to`, `content_tag`, `image_tag`, `simple_format`, `render`, `capture` or `tag.div` and the other `tag` elements. `<%== render … %>` is redundant, not unsafe, so it is quiet. Haml\'s `!==` and Slim\'s `#{{x}}` are not looked at. Only in Rails projects, in templates under app/views.',
    setting: {
      label: "Turn it off",
      code: `[Rails.UnsafeOutput]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          "<%= raw @user.bio %>",
          "<%= @user.bio.html_safe %>",
          "<%== @user.bio %>",
          '<a href="<%= raw @user.website %>">Website</a>',
          '<div data-bio="<%= raw @user.bio.to_json %>"></div>',
        ),
        after: text(
          "<%= sanitize @user.bio %>",
          "<%= sanitize @user.bio %>",
          "<%= sanitize @user.bio %>",
          '<a href="<%= @user.website %>">Website</a>',
          '<div data-bio="<%= @user.bio.to_json %>"></div>',
        ),
        faults: [
          {
            line: 1,
            column: 5,
            message: unsafeMessage("raw", "@user.bio"),
            code: "Rails/UnsafeOutput",
          },
          {
            line: 2,
            column: 15,
            message: unsafeMessage("html_safe", "@user.bio"),
            code: "Rails/UnsafeOutput",
          },
          {
            line: 3,
            column: 1,
            message: unsafeMessage("<%==", "@user.bio"),
            code: "Rails/UnsafeOutput",
          },
          {
            line: 4,
            column: 14,
            message: unsafeMessage("raw", "@user.website"),
            code: "Rails/UnsafeOutput",
          },
          {
            line: 5,
            column: 20,
            message: unsafeMessage(
              "raw",
              "@user.bio.to_json",
              "`to_json` doesn't escape quotes for an attribute; output it without `raw`",
            ),
            code: "Rails/UnsafeOutput",
          },
        ],
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("!= @user.bio", "= raw @user.bio"),
        after: text("= sanitize @user.bio", "= sanitize @user.bio"),
        faults: [
          {
            line: 1,
            column: 1,
            message: unsafeMessage("!=", "@user.bio"),
            code: "Rails/UnsafeOutput",
          },
          {
            line: 2,
            column: 3,
            message: unsafeMessage("raw", "@user.bio"),
            code: "Rails/UnsafeOutput",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("== @user.bio", "= raw @user.bio"),
        after: text("= sanitize @user.bio", "= sanitize @user.bio"),
        faults: [
          {
            line: 1,
            column: 1,
            message: unsafeMessage("==", "@user.bio"),
            code: "Rails/UnsafeOutput",
          },
          {
            line: 2,
            column: 3,
            message: unsafeMessage("raw", "@user.bio"),
            code: "Rails/UnsafeOutput",
          },
        ],
      },
    ],
  },
  {
    name: "Rails/UnusedBlockArgument",
    summary: "A block argument the block never uses.",
    languages: ["erb", "haml", "slim"],
    type: "warning",
    safe: true,
    status: "available",
    code: "Rails/UnusedBlockArgument",
    today:
      "A plain or optional block parameter that nothing in the block reads, assigns or interpolates, nested blocks included. Not reported: names starting with `_`, `it` and numbered parameters, destructured, `*rest`, keyword and `&block` parameters, or a block that calls `binding`, `local_variables` or `eval`. The message suggests removing the argument when it is the block's only one, and the quick fix prefixes it with `_`.",
    setting: {
      label: "Turn it off",
      code: `[Rails.UnusedBlockArgument]
enabled = false`,
    },
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
        faults: [
          {
            line: 1,
            column: 26,
            message: ARGUMENT_MESSAGE,
            code: "Rails/UnusedBlockArgument",
          },
        ],
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("- @users.each do |user, index|", "  %li= user.name"),
        after: text("- @users.each do |user|", "  %li= user.name"),
        faults: [
          {
            line: 1,
            column: 25,
            message: ARGUMENT_MESSAGE,
            code: "Rails/UnusedBlockArgument",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("- @users.each do |user, index|", "  li = user.name"),
        after: text("- @users.each do |user|", "  li = user.name"),
        faults: [
          {
            line: 1,
            column: 25,
            message: ARGUMENT_MESSAGE,
            code: "Rails/UnusedBlockArgument",
          },
        ],
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
      "It renames HTML attributes, which changes the page: the new ones work only in an app that runs Turbo, the old ones only with Rails UJS, so an app that still relies on UJS for a link or button loses it. That is why the rule is off until enabled. The formatter reads every element's attributes before and after and refuses the rewrite unless only the renamed ones differ, then checks the renamed text as it checks any file, so nothing else changes.",
    status: "available",
    onByDefault: [],
    config: text("[Turbo.PreferTurboAttributes]", "enabled = true"),
    today:
      'On links (`<a>`, `link_to`, `tag.a`, `content_tag :a`), `data-method` becomes `data-turbo-method`, and `data-confirm` becomes `data-turbo-confirm` when the link sends a method, since Turbo asks to confirm only such a link. On submit buttons (`<button>` with no `type` or `type="submit"`, `<input type="submit">`, `submit_tag`, `button_tag`, `button_to`, and `submit` or `button` on the builder of a `form_with`, `form_for` or `fields_for` block), `data-disable-with` becomes `data-turbo-submits-with`. In the options of a helper, the keys of `data: { }` are renamed in the spelling they have (`confirm:` becomes `turbo_confirm:`, `"disable-with" =>` becomes `"turbo-submits-with" =>`), as is a key such as `"data-method" =>`. Left alone: a `data-confirm` on a link without a method, `button_to`\'s confirm, which Turbo reads from the form, `link_to … method: :delete`, which is not a data attribute, an attribute whose Turbo name the element has already, a button of another `type` or whose attributes are not all readable, `disable_with: false` and a `confirm:` with no value (which reads a variable), other elements, and keys and attribute names that are not literal. It runs before the other formatter rules, whose checks then apply to the renamed text. Format selection applies it to the lines it covers.',
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text(
          '<%= link_to "Delete", user_path(user), data: { confirm: "Sure?", method: :delete } %>',
          '<%= submit_tag "Save", data: { disable_with: "Saving..." } %>',
          '<a href="/users/1" data-confirm="Sure?" data-method="delete">Delete</a>',
        ),
        after: text(
          '<%= link_to "Delete", user_path(user), data: { turbo_confirm: "Sure?", turbo_method: :delete } %>',
          '<%= submit_tag "Save", data: { turbo_submits_with: "Saving..." } %>',
          '<a href="/users/1" data-turbo-confirm="Sure?" data-turbo-method="delete">Delete</a>',
        ),
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          '= link_to "Delete", user_path(user), data: { confirm: "Sure?", method: :delete }',
          '%input{type: "submit", data: {disable_with: "Saving..."}}',
        ),
        after: text(
          '= link_to "Delete", user_path(user), data: { turbo_confirm: "Sure?", turbo_method: :delete }',
          '%input{type: "submit", data: {turbo_submits_with: "Saving..."}}',
        ),
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          '= link_to "Delete", user_path(user), data: { confirm: "Sure?", method: :delete }',
          'a href="/users/1" data-method="delete" data-confirm="Sure?" Delete',
        ),
        after: text(
          '= link_to "Delete", user_path(user), data: { turbo_confirm: "Sure?", turbo_method: :delete }',
          'a href="/users/1" data-turbo-method="delete" data-turbo-confirm="Sure?" Delete',
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
    status: "available",
    code: "Turbo/PermanentRequiresId",
    today:
      "Reported on the attribute, in ERB start tags, Haml and Slim tags, and the literal options of `tag.div`, `content_tag`, `link_to` and the form and button helpers. Any value counts, as Turbo looks only for the attribute. An `id` of any value satisfies it, so does `#id` in Haml and Slim and an object reference such as `%div[@user]`. Not reported where attributes may come from code that can't be read (ERB inside the start tag, a Haml `**attrs` or `data: attrs`, a Slim `*attrs` or `data=` hash, a helper's `**options`) or when the value is not a literal (`data: { turbo_permanent: flag }`). Rails projects only, in templates under app/views.",
    setting: {
      label: "Turn it off",
      code: `[Turbo.PermanentRequiresId]
enabled = false`,
    },
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
        faults: [
          {
            line: 1,
            column: 6,
            message: PERMANENT_MESSAGE,
            code: "Turbo/PermanentRequiresId",
          },
        ],
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
        faults: [
          {
            line: 1,
            column: 13,
            message: PERMANENT_MESSAGE,
            code: "Turbo/PermanentRequiresId",
          },
        ],
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
        faults: [
          {
            line: 1,
            column: 5,
            message: PERMANENT_MESSAGE,
            code: "Turbo/PermanentRequiresId",
          },
        ],
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
    status: "available",
    code: "Stimulus/UnknownController",
    today:
      'Each name in a static `data-controller` (ERB, Haml and Slim attributes, `data: { controller: "…" }` in Haml and in the options of `tag`, `content_tag`, `link_to`, `button_to` and the form helpers) must be a file in app/javascript/controllers (`search_controller.js`, `admin/users_controller.js` for `admin--users`; `-controller`, `.ts`, `.jsx`, `.tsx` and `.mjs` too) or a name a script in app/javascript registers with `application.register("name", …)`; the closest controller within two edits is offered. Nothing is reported when the app has no app/javascript/controllers folder, when controllers are loaded from another folder (`eagerLoadControllersFrom("components", …)`, a context or glob outside the controllers folder, an importmap pin) or registered under a name the code computes, or for a name with ERB or interpolation in it. Rails projects only, in templates under app/views.',
    setting: {
      label: "Turn it off",
      code: `[Stimulus.UnknownController]
enabled = false`,
    },
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
        files: { "app/javascript/controllers/search_controller.js": "" },
        faults: [
          {
            line: 1,
            column: 23,
            message: CONTROLLER_MESSAGE,
            code: "Stimulus/UnknownController",
          },
        ],
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
        files: { "app/javascript/controllers/search_controller.js": "" },
        faults: [
          {
            line: 1,
            column: 26,
            message: CONTROLLER_MESSAGE,
            code: "Stimulus/UnknownController",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('div data-controller="serch"', "  = text_field_tag :q"),
        after: text('div data-controller="search"', "  = text_field_tag :q"),
        files: { "app/javascript/controllers/search_controller.js": "" },
        faults: [
          {
            line: 1,
            column: 22,
            message: CONTROLLER_MESSAGE,
            code: "Stimulus/UnknownController",
          },
        ],
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
    type: "warning",
    safe: true,
    status: "available",
    code: "Html/ImgAlt",
    today:
      "For `<img>` (ERB start tags, `%img` and `img`) and for `image_tag` and `tag.img` without `alt:`, in templates that output HTML (not `.js.erb`, `.text.erb` or `.xml.haml`). An empty `alt=\"\"` counts as alt text, and so does `generator-unable-to-provide-required-alt`. Not reported where attributes may come from code that can't be read (ERB inside the start tag, a Haml `**attrs`, a Slim `*attrs`, a helper's `**options` or options in a variable).",
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
      {
        lang: "haml",
        file: HAML_FILE,
        before: text(
          "%img{src: @user.avatar_url}",
          "= image_tag @user.avatar_url",
        ),
        after: text(
          "%img{src: @user.avatar_url, alt: @user.name}",
          "= image_tag @user.avatar_url, alt: @user.name",
        ),
        faults: [
          { line: 1, column: 2, message: IMG_MESSAGE, code: "Html/ImgAlt" },
          {
            line: 2,
            column: 3,
            message: IMAGE_TAG_MESSAGE,
            code: "Html/ImgAlt",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text(
          "img src=@user.avatar_url",
          "= image_tag @user.avatar_url",
        ),
        after: text(
          "img src=@user.avatar_url alt=@user.name",
          "= image_tag @user.avatar_url, alt: @user.name",
        ),
        faults: [
          { line: 1, column: 1, message: IMG_MESSAGE, code: "Html/ImgAlt" },
          {
            line: 2,
            column: 3,
            message: IMAGE_TAG_MESSAGE,
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
    type: "warning",
    safe: true,
    status: "available",
    code: "Html/DuplicateId",
    today:
      "At the second occurrence, in templates that output HTML. In Haml and Slim, `#id` and `id:` count. Ids that hold ERB or interpolation, ids in different branches of a conditional (ERB `if`/`else`, Haml and Slim `- if`/`- else`) and content inside `<template>` are left alone; in Haml and Slim, so are ids where one of them sits in a conditional, a loop or a block the other is not in. A Haml element with both `#id` and `id:` gets one joined id and is skipped.",
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
      {
        lang: "haml",
        file: HAML_FILE,
        before: text("#profile", "  = @user.name", "#profile"),
        after: text("#profile", "  = @user.name", "#settings"),
        faults: [
          {
            line: 3,
            column: 2,
            message: DUPLICATE_ID_MESSAGE,
            code: "Html/DuplicateId",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text("#profile", "  = @user.name", "#profile"),
        after: text("#profile", "  = @user.name", "#settings"),
        faults: [
          {
            line: 3,
            column: 2,
            message: DUPLICATE_ID_MESSAGE,
            code: "Html/DuplicateId",
          },
        ],
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
    type: "warning",
    safe: true,
    status: "available",
    code: "Html/DuplicateAttribute",
    today:
      'At the second one, for names written out in ERB start tags, Haml tags (the `(…)` list and the `{…}` hash, `data: { … }` keys included) and Slim tags, and in the literal options of `tag.div`, `content_tag`, `link_to` and the form and button helpers, in templates that output HTML. `class` and `id` are repeated on purpose in Haml, Slim and the helpers, which merge them (`.card{class: "wide"}`), so only plain ERB start tags are checked for them. An ERB start tag with ERB inside is left alone, since it may pick one of two attributes. Slim itself rejects a repeated attribute other than `class`, so there it is an error with Slim\'s message, `Multiple href attributes specified`.',
    setting: {
      label: "Turn it off",
      code: `[Html.DuplicateAttribute]
enabled = false`,
    },
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
        faults: [
          {
            line: 1,
            column: 19,
            message:
              "Duplicate attribute `class` on `<div>`: a start tag must not repeat an attribute (HTML 13.1.2.3)",
            code: "Html/DuplicateAttribute",
          },
        ],
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('%a{href: "/users", href: "/people"} Users'),
        after: text('%a{href: "/users"} Users'),
        faults: [
          {
            line: 1,
            column: 4,
            message: "`href` is given twice; only one value is rendered",
            code: "Html/DuplicateAttribute",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('a href="/users" href="/people" Users'),
        after: text('a href="/users" Users'),
        note: "Slim's own error, `Multiple href attributes specified`, is reported as `Syntax/Template`.",
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
    status: "available",
    code: "Html/AnchorHref",
    today:
      "On the tag name of `<a>` in ERB start tags and of `%a` and `a` in Haml and Slim, in templates that output HTML. `link_to` always has an `href` and is not checked. An `<a>` with a `name` (the old way to mark a place to jump to), a `tabindex` (the keyboard can reach it), an `id` and no text of its own (a target to jump to), or an attribute ending in `:href` (`:href`, `x-bind:href`, `xlink:href`) is left alone, and so is one inside an `<svg>`, and so is one whose attributes may come from code that can't be read (ERB inside the start tag, a Haml `**attrs` or `data: attrs`, a Slim `*attrs` or `data=` hash).",
    setting: {
      label: "Turn it off",
      code: `[Html.AnchorHref]
enabled = false`,
    },
    examples: [
      {
        lang: "erb",
        file: ERB_FILE,
        before: text('<a class="btn" data-action="click->menu#open">Menu</a>'),
        after: text(
          '<button class="btn" data-action="click->menu#open">Menu</button>',
        ),
        faults: [
          {
            line: 1,
            column: 2,
            message: HREF_MESSAGE,
            code: "Html/AnchorHref",
          },
        ],
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('%a.btn{data: {action: "click->menu#open"}} Menu'),
        after: text('%button.btn{data: {action: "click->menu#open"}} Menu'),
        faults: [
          {
            line: 1,
            column: 2,
            message: HREF_MESSAGE,
            code: "Html/AnchorHref",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('a.btn data-action="click->menu#open" Menu'),
        after: text('button.btn data-action="click->menu#open" Menu'),
        faults: [
          {
            line: 1,
            column: 1,
            message: HREF_MESSAGE,
            code: "Html/AnchorHref",
          },
        ],
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
    status: "available",
    code: "A11y/IframeTitle",
    today:
      "On the tag name of an `<iframe>` in ERB start tags, Haml and Slim tags, and of `tag.iframe` and `content_tag :iframe`, in templates that output HTML. An `aria-label` or `aria-labelledby` (also in an `aria: { label: … }` hash) names the frame as well. Not reported where attributes may come from code that can't be read (ERB inside the start tag, `**attrs`, `*attrs`).",
    setting: {
      label: "Turn it off",
      code: `[A11y.IframeTitle]
enabled = false`,
    },
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
        faults: [
          {
            line: 1,
            column: 2,
            message: TITLE_MESSAGE,
            code: "A11y/IframeTitle",
          },
        ],
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
        faults: [
          {
            line: 1,
            column: 2,
            message: TITLE_MESSAGE,
            code: "A11y/IframeTitle",
          },
        ],
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
        faults: [
          {
            line: 1,
            column: 1,
            message: TITLE_MESSAGE,
            code: "A11y/IframeTitle",
          },
        ],
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
    status: "available",
    code: "A11y/GenericLinkText",
    today:
      'On the text of an `<a>` that holds only text (ERB, Haml and Slim tags) and of a `link_to`, `tag.a` or `content_tag :a` given a string literal: "click here", "here", "read more", "learn more", "more", "more info", "more information", "link", "this link" and "click", in any case and ignoring punctuation. A link with an `aria-label`, `aria-labelledby` or `aria-describedby` (also in an `aria: { … }` hash), with ERB, interpolation, other elements or code in its content, or whose attributes may come from code that can\'t be read is left alone.',
    setting: {
      label: "Turn it off",
      code: `[A11y.GenericLinkText]
enabled = false`,
    },
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
        faults: [
          {
            line: 1,
            column: 42,
            message: LINK_TEXT_MESSAGE,
            code: "A11y/GenericLinkText",
          },
        ],
      },
      {
        lang: "haml",
        file: HAML_FILE,
        before: text('%p= link_to "Click here", pricing_path'),
        after: text('%p= link_to "See the new pricing", pricing_path'),
        faults: [
          {
            line: 1,
            column: 14,
            message: LINK_TEXT_MESSAGE,
            code: "A11y/GenericLinkText",
          },
        ],
      },
      {
        lang: "slim",
        file: SLIM_FILE,
        before: text('p = link_to "Click here", pricing_path'),
        after: text('p = link_to "See the new pricing", pricing_path'),
        faults: [
          {
            line: 1,
            column: 14,
            message: LINK_TEXT_MESSAGE,
            code: "A11y/GenericLinkText",
          },
        ],
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
