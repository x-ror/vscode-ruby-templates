// Every diagnostic code (rule name) the language server reports, with a template that triggers it and the message
// `ruby-templates check` prints for that template.
import fs from "node:fs";
import path from "node:path";

export type Language = "erb" | "haml" | "slim" | "jbuilder";

export const LANGUAGE_NAMES: Record<Language, string> = {
  erb: "ERB",
  haml: "Haml",
  slim: "Slim",
  jbuilder: "Jbuilder",
};

export interface Diagnostic {
  /** The rule name, `Department/Name`: the diagnostic's code. */
  code: string;
  /** The code before 0.11, still accepted in `disabledDiagnostics`; none for a rule that is newer. */
  formerly?: string;
  languages: Language[];
  summary: string;
  /** When it fires; backticks quote code, as in the server's messages. */
  explanation: string;
  example: { file: string; lang: string; code: string };
  message: string;
  line: number;
  column: number;
}

/** The id of a code's section on the diagnostics page: `Html/ImgAlt` is `html-imgalt`. */
export const codeAnchor = (code: string) =>
  code.toLowerCase().replace("/", "-");

export const diagnostics: Diagnostic[] = [
  {
    code: "Syntax/UnclosedErbTag",
    formerly: "unclosed-tag",
    languages: ["erb"],
    summary: "A `<%` with no `%>` after it.",
    explanation:
      "Erubi only compiles a tag it can close, so a `<%` without `%>` stays in the output as text, and the Ruby is printed instead of run. In HTML, `<` followed by `%` starts no tag (HTML 13.2.5.6). Reported only where the HTML tokenizer is between tags, not inside attribute values, comments or `<script>` and `<style>` bodies, and only in ERB with an HTML host (`.html.erb` and the like).",
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<p><%= @user.name %></p>
<p><%= @user.email</p>
`,
    },
    message:
      "invalid-first-character-of-tag-name: `<%` without `%>` is text, and `<` followed by `%` starts no tag (HTML 13.2.5.6)",
    line: 2,
    column: 4,
  },
  {
    code: "Html/UnexpectedCloseTag",
    formerly: "unexpected-close-tag",
    languages: ["erb"],
    summary: "An end tag with no open element of that name.",
    explanation:
      'The end tag closes nothing, so the browser drops it. Also reported for the end tag of a void element ("End tag `</br>` for a void element") and for a `</p>` after a block element such as `<div>` already closed the paragraph. A stray end tag in a partial is not reported, since a partial may close what another one opened. ERB with an HTML host only; checking stops after 20 tag problems in one file.',
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<div class="profile">
  <p><%= @user.name %></p>
</div>
</div>
`,
    },
    message:
      "Unexpected end tag `</div>`: no open element of that name (HTML 13.2.6.4.7)",
    line: 4,
    column: 1,
  },
  {
    code: "Html/MismatchedCloseTag",
    formerly: "mismatched-close-tag",
    languages: ["erb"],
    summary: "An end tag that doesn't close the innermost open element.",
    explanation:
      "The message names the element that is still open and its line. When the end tag crosses an open element (`<div><span></div>`), the browser could recover in more than one way, so checking stops there instead of guessing. Reported in partials too. ERB with an HTML host only.",
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<div class="profile">
  <span><%= @user.name %></div>
`,
    },
    message:
      "End tag `</div>` doesn't close the open `<span>` (line 2) (HTML 13.2.6.4.7)",
    line: 2,
    column: 26,
  },
  {
    code: "Html/UnclosedElement",
    formerly: "unclosed-element",
    languages: ["erb"],
    summary: "An element whose end tag never comes.",
    explanation:
      "Reported at the start tag when the file ends with the element still open. Elements whose end tag HTML lets you omit (`li`, `p`, `td`, `option`, …) and void elements are never reported. Partials are exempt, because a `_header` partial may open a `<div>` that `_footer` closes, and so is a file that ends inside an ERB block. ERB with an HTML host only.",
    example: {
      file: "app/views/users/index.html.erb",
      lang: "erb",
      code: `<section class="users">
  <h2>Users</h2>
`,
    },
    message: "`<section>` is never closed (HTML 13.2.6.4.7)",
    line: 1,
    column: 1,
  },
  {
    code: "Html/ImgAlt",
    formerly: "img-missing-alt",
    languages: ["erb", "haml", "slim"],
    summary: "An image without alt text.",
    explanation:
      'Fires for an `<img>` (`%img` in Haml, `img` in Slim) with no `alt` attribute and for an `image_tag` call whose options have no `alt:`; that one reads "`image_tag` without `alt:`". In ERB the call must be in an output tag. Not reported for an `<img>` with ERB where attributes go (`<img <%= attrs %>>`), a Haml `**attrs` or a Slim `*attrs`, an `image_tag` given a `**splat`, or `generator-unable-to-provide-required-alt`, since each may supply the text. An empty `alt=""` counts as alt text. Templates that output HTML only: not `.js.erb`, `.text.erb` or `.xml.haml`.',
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<img src="<%= @user.avatar_url %>">
`,
    },
    message:
      "`<img>` without `alt`: the alt attribute must be specified (HTML 4.8.4.4.1)",
    line: 1,
    column: 1,
  },
  {
    code: "Html/DuplicateId",
    formerly: "duplicate-id",
    languages: ["erb", "haml", "slim"],
    summary: "The same static id twice in one template.",
    explanation:
      "Reported at the second occurrence, naming the line of the first. Only literal values count: an id holding ERB or interpolation is left alone, and so is an element with both `#id` and `id:` in Haml (it gets one joined id). Two ids in different branches of a conditional don't clash, and content inside `<template>` isn't in the document yet. In Haml and Slim, ids are also left alone when one of them sits in a conditional, a loop or a block that the other is not in. Templates that output HTML only.",
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<div id="profile">
  <%= @user.name %>
</div>
<div id="profile"></div>
`,
    },
    message:
      "Duplicate id `profile` (first on line 1): the value must be unique amongst all the IDs in the element's tree (HTML 3.2.6)",
    line: 4,
    column: 10,
  },
  {
    code: "Html/DuplicateAttribute",
    languages: ["erb", "haml", "slim"],
    summary: "An attribute given twice on one tag.",
    explanation:
      "Reported at the second one, for names written out in ERB start tags, Haml tags (the `(…)` list and the `{…}` hash) and Slim tags, and in the literal options of Rails tag helpers. In ERB markup the page gets both, so the second is reported; in Haml and the helpers a repeated key renders once, so the value that is dropped is reported. `class` and `id` are left alone in Haml, Slim and the helpers, which merge them. An ERB start tag with ERB inside is left alone. Slim rejects a repeated attribute itself, which is reported as `Syntax/Template`. Templates that output HTML only.",
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<div class="card" class="wide">
  <%= @user.name %>
</div>
`,
    },
    message:
      "Duplicate attribute `class` on `<div>`: a start tag must not repeat an attribute (HTML 13.1.2.3)",
    line: 1,
    column: 19,
  },
  {
    code: "Html/AnchorHref",
    languages: ["erb", "haml", "slim"],
    summary:
      "An `<a>` without an `href`, which is not a link and cannot be reached with the keyboard.",
    explanation:
      "Reported on the tag name. `link_to` always has an `href` and is not checked. An `<a>` with a `name`, a `tabindex`, an `id` and no text of its own, or an attribute ending in `:href` (`:href`, `x-bind:href`), and one inside an `<svg>`, is left alone, as is one whose attributes may come from code that can't be read (ERB inside the start tag, `**attrs`, `*attrs`), is left alone. Templates that output HTML only.",
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<a class="btn" data-action="click->menu#open">Menu</a>
`,
    },
    message:
      "`<a>` without `href` is not a link and the keyboard can't reach it; use a `<button>` for an action",
    line: 1,
    column: 2,
  },
  {
    code: "A11y/IframeTitle",
    languages: ["erb", "haml", "slim"],
    summary:
      "An `<iframe>` without a `title`, which screen readers use to name it.",
    explanation:
      "Reported on the tag name, also for `tag.iframe` and `content_tag :iframe`. An `aria-label` or `aria-labelledby` (also in an `aria: { label: … }` hash) names the frame as well. Not reported where attributes may come from code that can't be read. Templates that output HTML only.",
    example: {
      file: "app/views/videos/show.html.erb",
      lang: "erb",
      code: `<iframe src="https://www.youtube.com/embed/<%= @video.youtube_id %>"></iframe>
`,
    },
    message:
      "`<iframe>` without `title`: screen readers announce it to name the frame (WCAG 4.1.2)",
    line: 1,
    column: 2,
  },
  {
    code: "A11y/GenericLinkText",
    languages: ["erb", "haml", "slim"],
    summary:
      'Link text that says nothing out of context: "click here", "here", "read more".',
    explanation:
      'Reported on the text, for an `<a>` that holds only text and for a `link_to`, `tag.a` or `content_tag :a` given a string literal. The texts are "click here", "here", "read more", "learn more", "more", "more info", "more information", "link", "this link" and "click", in any case and ignoring punctuation. A link with an `aria-label`, `aria-labelledby` or `aria-describedby`, or with ERB, interpolation or other elements inside, is left alone. Templates that output HTML only.',
    example: {
      file: "app/views/pricing/show.html.erb",
      lang: "erb",
      code: `<p>Our new pricing is live. <%= link_to "Click here", pricing_path %></p>
`,
    },
    message:
      "Link text `Click here` says nothing out of context; name where the link goes (WCAG 2.4.9, F84)",
    line: 1,
    column: 42,
  },
  {
    code: "Syntax/InconsistentIndent",
    formerly: "inconsistent-indent",
    languages: ["haml"],
    summary: "Indentation Haml can't follow.",
    explanation:
      'Haml\'s own indentation errors: a width other than the one the rest of the document uses, tabs mixed with spaces ("Indentation can\'t use both tabs and spaces."), or a line more than one level deeper than the one before ("The line was indented 2 levels deeper than the previous line."). Haml stops at its first error, so only that one is shown, and the Ruby isn\'t checked until it is fixed.',
    example: {
      file: "app/views/users/index.html.haml",
      lang: "haml",
      code: `%section
  %h2 Users
  %ul
   %li= @user.name
`,
    },
    message:
      "Inconsistent indentation: 3 spaces used for indentation, but the rest of the document was indented using 2 spaces.",
    line: 4,
    column: 1,
  },
  {
    code: "Syntax/TopLevelIndent",
    formerly: "top-level-indent",
    languages: ["haml"],
    summary: "The first line is indented.",
    explanation:
      "Haml measures every level from the first line, which must start at column 1.",
    example: {
      file: "app/views/users/show.html.haml",
      lang: "haml",
      code: `  %p= @user.name
`,
    },
    message: "Indenting at the beginning of the document is illegal.",
    line: 1,
    column: 1,
  },
  {
    code: "Syntax/IllegalNesting",
    formerly: "illegal-nesting",
    languages: ["haml"],
    summary: "Lines nested under something that can't hold them.",
    explanation:
      'Haml rejects lines nested under a tag that already has content on its own line, under plain text, and under a self-closing tag such as `%br/`; each has its own "Illegal nesting" message. Nesting under `!!!` has its own code, `Syntax/DoctypeChildren`.',
    example: {
      file: "app/views/users/show.html.haml",
      lang: "haml",
      code: `%p= @user.name
  %span Admin
`,
    },
    message:
      "Illegal nesting: content can't be both given on the same line as %p and nested within it.",
    line: 2,
    column: 3,
  },
  {
    code: "Syntax/DoctypeChildren",
    formerly: "doctype-children",
    languages: ["haml", "slim"],
    summary: "Lines nested under the doctype.",
    explanation:
      'The doctype line (`!!!` in Haml, `doctype` in Slim) takes no children; the `%html` or `html` that follows belongs at the same level. Slim reports it as "Unexpected indentation".',
    example: {
      file: "app/views/layouts/application.html.haml",
      lang: "haml",
      code: `!!! 5
  %html
`,
    },
    message: "Illegal nesting: nesting within a header command is illegal.",
    line: 2,
    column: 3,
  },
  {
    code: "Syntax/UnmatchedBranch",
    formerly: "unmatched-branch",
    languages: ["haml"],
    summary:
      "An `else`, `elsif` or `when` that doesn't line up with its block.",
    explanation:
      'In Haml a branch keyword must stand at the indentation of the `- if` or `- case` it continues. One at another level is reported with the expected and actual levels; one with no open block before it reads "Got "else" with no preceding "if"". The first `when` may sit one level deeper than its `case`.',
    example: {
      file: "app/views/users/index.html.haml",
      lang: "haml",
      code: `- if @users.any?
  = render @users
  - else
    %p No users
`,
    },
    message: '"else" is indented at wrong level: expected 0, but was at 1.',
    line: 3,
    column: 3,
  },
  {
    code: "Syntax/ExplicitEnd",
    formerly: "explicit-end",
    languages: ["haml", "slim"],
    summary: "A `- end` line.",
    explanation:
      'Haml and Slim close a Ruby block by un-indenting, and both refuse an explicit `- end`. Slim\'s message is shown here; Haml\'s starts with "You don\'t need to use "- end" in Haml." and goes on with an example. Skim templates, whose code is CoffeeScript, are not checked for it.',
    example: {
      file: "app/views/users/show.html.slim",
      lang: "slim",
      code: `- if @user.admin?
  p Admin
- end
`,
    },
    message: "Explicit end statements are forbidden",
    line: 3,
    column: 1,
  },
  {
    code: "Syntax/IllegalElement",
    formerly: "illegal-element",
    languages: ["haml"],
    summary: "A `.` or `#` with no class or id name after it.",
    explanation:
      "Haml needs a name after each `.` and `#` in a tag (`%p.lead`, `#main`).",
    example: {
      file: "app/views/users/show.html.haml",
      lang: "haml",
      code: `%p. Hello
`,
    },
    message: "Illegal element: classes and ids must have values.",
    line: 1,
    column: 1,
  },
  {
    code: "Syntax/UnknownFilter",
    formerly: "unknown-filter",
    languages: ["haml"],
    summary: "A filter Haml doesn't have.",
    explanation:
      "The name after `:` isn't one of Haml's filters (`:javascript`, `:css`, `:markdown`, `:plain`, `:ruby`, …), usually a typo. Haml raises this when it compiles the template. A name that isn't a word at all is a `Syntax/Template` (\"Invalid filter name\").",
    example: {
      file: "app/views/users/show.html.haml",
      lang: "haml",
      code: `:mardown
  # Users
`,
    },
    message: "FilterCompiler for 'mardown' was not found",
    line: 1,
    column: 1,
  },
  {
    code: "Syntax/SelfClosingContent",
    formerly: "self-closing-content",
    languages: ["haml"],
    summary: "Content after a tag closed with `/`.",
    explanation:
      "A tag ending in `/` renders as `<br />` and can hold nothing on its own line.",
    example: {
      file: "app/views/users/show.html.haml",
      lang: "haml",
      code: `%br/ Hello
`,
    },
    message: "Self-closing tags can't have content.",
    line: 1,
    column: 1,
  },
  {
    code: "Syntax/UnclosedAttribute",
    formerly: "unclosed-attribute",
    languages: ["haml", "slim"],
    summary: "An attribute list or value that is never closed.",
    explanation:
      'Slim reports the bracket it expected ("Expected closing delimiter )", or `]` or `}`), or "Unexpected end of file" for a quote left open. Haml reports "Unbalanced brackets." for a `{` or `(` attribute list that doesn\'t close, and "Invalid attribute list" for one it can\'t read.',
    example: {
      file: "app/views/users/show.html.slim",
      lang: "slim",
      code: `a href=(user_path(@user) Profile
`,
    },
    message: "Expected closing delimiter )",
    line: 1,
    column: 33,
  },
  {
    code: "Syntax/Template",
    formerly: "syntax-error",
    languages: ["haml", "slim"],
    summary: "Any other error the Haml or Slim parser raises.",
    explanation:
      'The parser errors that have no code of their own. In Haml: "Invalid tag", "Invalid filter name" and "There\'s no Ruby code for = to evaluate." In Slim: "Unknown line indicator", "Illegal shortcut", "Invalid empty attribute", "Expected attribute", "Expected tag" for a block expansion with nothing after it (`ul:`), "Unexpected text after closed tag", "Multiple id attributes specified" (for any repeated attribute) and "Invalid doctype".',
    example: {
      file: "app/views/users/show.html.haml",
      lang: "haml",
      code: `%p=
`,
    },
    message: "There's no Ruby code for = to evaluate.",
    line: 1,
    column: 1,
  },
  {
    code: "Html/VoidChildren",
    formerly: "void-children",
    languages: ["haml", "slim"],
    summary: "Content inside a void element such as `img` or `input`.",
    explanation:
      'Void elements have no end tag, so the content would end up after them. Haml reports content on the tag\'s line or nested under it. Slim reports lines nested under a void element, and "Unexpected indentation" under a tag closed with `/`. In Slim it applies to HTML templates only, not to `.xml.slim` and other formats.',
    example: {
      file: "app/views/users/show.html.slim",
      lang: "slim",
      code: `img src="/logo.png"
  | Logo
`,
    },
    message: "`img` is a void element and can't have children",
    line: 2,
    column: 3,
  },
  {
    code: "Syntax/BadDedent",
    formerly: "bad-dedent",
    languages: ["slim"],
    summary: "A line un-indented to a level no open line has.",
    explanation:
      "When a line moves left, Slim needs it to land on the indentation of a line still open above it. Here `li Two` stops between `ul` and `li One`.",
    example: {
      file: "app/views/users/index.html.slim",
      lang: "slim",
      code: `section
  ul
    li One
   li Two
`,
    },
    message: "Malformed indentation",
    line: 4,
    column: 4,
  },
  {
    code: "Syntax/UnknownEngine",
    formerly: "unknown-engine",
    languages: ["slim"],
    summary: "An embedded engine Slim doesn't know.",
    explanation:
      'A lone `name:` line opens an embedded engine. Slim\'s own are `ruby`, `javascript`, `css`, `sass`, `scss`, `coffee`, `markdown`, `textile`, `rdoc` and `asciidoc`; for any other Slim raises "Expected tag". Engines your app registers (such as `mjml`) go in `rubyTemplates.slim.extraEngines`.',
    example: {
      file: "app/views/users/show.html.slim",
      lang: "slim",
      code: `markdwn:
  # Users
`,
    },
    message: "Expected tag",
    line: 1,
    column: 9,
  },
  {
    code: "Syntax/Ruby",
    formerly: "ruby-syntax",
    languages: ["erb", "haml", "slim", "jbuilder"],
    summary: "Ruby in the template that doesn't parse.",
    explanation:
      "The template is compiled the way Erubi, Haml or Slim compile it, and the result is parsed with Prism for the project's Ruby version; the message is Prism's. Haml and Slim code is checked only once the template itself parses. Also covers the `locals:` comment of strict locals (Rails 7.1 and later), for instance a local without a colon: `# locals: (user)` gives \"`user` set as non-keyword argument for … Locals can only be set as keyword arguments.\" In Jbuilder and Builder views only that comment is checked, and in Skim templates nothing, since their code is CoffeeScript.",
    example: {
      file: "app/views/users/index.html.erb",
      lang: "erb",
      code: `<ul>
  <% @users.each do |user| %>
    <li><%= user.name %></li>
</ul>
`,
    },
    message:
      "unexpected end-of-input, assuming it is closing the parent top level context",
    line: 4,
    column: 1,
  },
  {
    code: "Rails/MissingPartial",
    formerly: "unresolved-render",
    languages: ["erb", "haml", "slim", "jbuilder"],
    summary: "A `render` of a partial or template that isn't in app/views.",
    explanation:
      'Checks literal names in `render "users/card"`, `render partial:`, `render template:` and Jbuilder\'s `json.partial!`, in a Rails app (`actionview` in `Gemfile.lock`) and in templates under app/views. A name with a folder is checked when that folder exists in app/views; a bare name (`render "form"`) only in an action template, not a partial, whose controller chain is in the app, and the message then lists each folder Rails searches. Folders of view-shipping gems such as `devise/` are skipped. The message is Rails\' "Missing partial" or "Missing template", without the formats and view paths Rails adds.',
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<%= render "users/profile", user: @user %>
`,
    },
    message: "Missing partial users/_profile",
    line: 1,
    column: 13,
  },
  {
    code: "Rails/StrictLocals",
    formerly: "strict-locals",
    languages: ["erb", "haml", "slim", "jbuilder"],
    summary:
      "A `render` whose locals don't match the partial's `locals:` comment.",
    explanation:
      'With Rails 7.1 or later in `Gemfile.lock`, a literal render that resolves to exactly one partial is compared with that partial\'s strict locals. Here `_card.html.erb` starts with `<%# locals: (user:) -%>`. The messages are Rails\': "missing local", "missing keywords", "unknown local", "unknown keywords", and "no locals accepted" for a partial declaring `locals: ()`. From Rails 7.2 they end with the partial\'s path. The `layout:` of a render isn\'t checked, since it gets the call\'s locals.',
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<%= render "users/card", person: @user %>
`,
    },
    message: "missing local: :user for app/views/users/_card.html.erb",
    line: 1,
    column: 13,
  },
  {
    code: "Rails/SilentRender",
    languages: ["erb", "haml", "slim"],
    summary: "A `render` in a tag that outputs nothing.",
    explanation:
      "A `<% %>` tag in ERB, and a `-` line in Haml and Slim, run Ruby and print nothing, so the partial `render` returns is thrown away. Reported for a `render` that is a whole statement, also in its block form. Not reported when the result is assigned or passed on. Rails projects only, in templates under app/views. The quick fix changes the tag to `<%=` (`=` in Haml and Slim) when the call is alone in its tag or line.",
    example: {
      file: "app/views/users/index.html.erb",
      lang: "erb",
      code: `<% render "users/card", user: @user %>
`,
    },
    message: "`render` in `<% %>` outputs nothing; use `<%=`",
    line: 1,
    column: 4,
  },
  {
    code: "Rails/SilentHelper",
    languages: ["erb", "haml", "slim"],
    summary:
      "A helper that returns markup, called in a tag that outputs nothing.",
    explanation:
      "`link_to`, `image_tag`, `content_tag`, `form_with` and the other helpers on the list return markup and do nothing else, so calling one in a `<% %>` tag (a `-` line in Haml and Slim) has no effect. Helpers with side effects (`content_for`, `provide`), your own helpers and calls on a receiver are not reported. Rails projects only, in templates under app/views. The quick fix changes the tag to `<%=` (`=` in Haml and Slim) when the call is alone in its tag or line.",
    example: {
      file: "app/views/users/index.html.erb",
      lang: "erb",
      code: `<% link_to "Home", root_path %>
`,
    },
    message: "`link_to` returns markup that `<% %>` throws away; use `<%=`",
    line: 1,
    column: 4,
  },
  {
    code: "Rails/DebugOutput",
    languages: ["erb", "haml", "slim"],
    summary: "Debugging left in a view.",
    explanation:
      "`p`, `pp`, `puts` and `debug` with arguments, `byebug`, `debugger`, `binding.irb` and `binding.pry`, as a statement or as the whole of an output tag. A local variable named `p`, a method on a receiver, a call used as an argument and a call under an `if` or `unless` that tests `Rails.env` are not reported. Rails projects only, in templates under app/views.",
    example: {
      file: "app/views/users/index.html.erb",
      lang: "erb",
      code: `<%= debug @user %>
<p><%= @user.name %></p>
`,
    },
    message: "`debug` is debugging left in the view",
    line: 1,
    column: 5,
  },
  {
    code: "Rails/InstanceVariableInPartial",
    languages: ["erb", "haml", "slim"],
    summary: "An instance variable read inside a partial.",
    explanation:
      'A partial that reads `@user` works only for the controllers that set it. Pass it as a local instead (`render "users/card", user: @user`). Reported once per variable and file. Not reported when the partial assigns the variable itself, for `defined?(@user)`, for ActionView\'s own variables, and in partials under app/views/layouts. Rails projects only, in templates under app/views.',
    example: {
      file: "app/views/users/_card.html.erb",
      lang: "erb",
      code: `<p><%= @user.name %></p>
`,
    },
    message:
      "`@user` is read in a partial, so it works only where the controller sets it; pass it as a local",
    line: 1,
    column: 8,
  },
  {
    code: "Rails/UnusedStrictLocal",
    languages: ["erb", "haml", "slim"],
    summary: "A local in the `locals:` comment that the partial never uses.",
    explanation:
      "Reads, assignments, `local_assigns[:name]`, `local_assigns.fetch(:name)`, `binding.local_variable_get(:name)`, `defined?(name)` and the default of another local count as use. Nothing is reported when the template passes `local_assigns` on as a whole or calls `binding`, `local_variables` or `eval`. The quick fix removes the local from the comment. Rails 7.1 or newer, in Rails projects, in templates under app/views.",
    example: {
      file: "app/views/users/_card.html.erb",
      lang: "erb",
      code: `<%# locals: (user:, compact: false) %>
<p><%= user.name %></p>
`,
    },
    message: "`compact` is declared in `locals:` but the partial never uses it",
    line: 1,
    column: 21,
  },
  {
    code: "Rails/UnusedBlockArgument",
    languages: ["erb", "haml", "slim"],
    summary: "A block argument the block never uses.",
    explanation:
      "Looks at plain and optional block parameters, and at the whole block, which spans every tag between `do |…|` and `end`. Names starting with `_`, `it` and numbered parameters, destructured, `*rest`, keyword and `&block` parameters, and blocks that call `binding`, `local_variables` or `eval` are not reported. The quick fix prefixes the name with `_`. Rails projects only, in templates under app/views.",
    example: {
      file: "app/views/users/index.html.erb",
      lang: "erb",
      code: `<% @users.each do |user, index| %>
  <li><%= user.name %></li>
<% end %>
`,
    },
    message: "Block argument `index` is never used; rename it `_index`",
    line: 1,
    column: 26,
  },
  {
    code: "Rails/RenderInLoop",
    languages: ["erb", "haml", "slim"],
    summary: "A partial rendered once per item inside a loop.",
    explanation:
      'Reported for `collection.each do |item|` and `each_with_index` whose whole body is `render "partial", name: item`, with a string literal for the partial. `render partial: "partial", collection: collection, as: :name` renders the same markup in one call. Loops that do anything else, other locals and other iterators are not reported. Rails projects only, in templates under app/views.',
    example: {
      file: "app/views/users/index.html.erb",
      lang: "erb",
      code: `<% @users.each do |user| %>
  <%= render "users/card", user: user %>
<% end %>
`,
    },
    message:
      'Partial "users/card" is rendered once per item of a loop; use `render partial: "users/card", collection: @users, as: :user`',
    line: 2,
    column: 7,
  },
  {
    code: "Rails/OutputControlFlow",
    languages: ["erb"],
    summary: "Control flow in an output tag.",
    explanation:
      "`<%= if @user %>` is a syntax error, or prints a loop's value (`<%= for x in y do %>`): an output tag holds an expression, so a tag that opens `if`, `unless`, `while`, `until`, `case`, `for` or `begin` without closing it, or that starts with `else`, `elsif`, `when`, `end`, `rescue` or `ensure`, needs `<%` instead. Reported on the `=`, with a quick fix that removes it; the Ruby syntax errors it would cause are not shown, the template's other errors are. A complete expression such as `<%= if a then b else c end %>` is not reported.",
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<%= if @user %>
  <p>hi</p>
<% end %>
`,
    },
    message: "`if` in an output tag `<%=`: use `<% if … %>`",
    line: 1,
    column: 3,
  },
  {
    code: "Rails/UnsafeOutput",
    languages: ["erb", "haml", "slim"],
    summary: "Output that skips Rails' HTML escaping.",
    explanation:
      "`raw x`, `x.html_safe` and `safe_concat x` are reported on the method name, and the unescaped output markers `<%==` (ERB), `!=` and `!~` (Haml) and `==` (Slim) on the marker. A `to_json` value gets the hint to use `json_escape`. Not reported when the value is certainly safe: a literal without interpolation, a `t` call with a literal key ending in `_html` or `.html`, or a call to `sanitize`, `safe_join`, `strip_tags`, `json_escape`, `j`, `escape_javascript`, `h`, `html_escape`, `link_to`, `content_tag`, `render`, `capture` and the like. Rails projects only, in templates under app/views.",
    example: {
      file: "app/views/users/show.html.erb",
      lang: "erb",
      code: `<%= raw @user.bio %>
`,
    },
    message:
      "`raw` outputs `@user.bio` without escaping; use `sanitize` if it holds user HTML",
    line: 1,
    column: 5,
  },
  {
    code: "Turbo/PermanentRequiresId",
    languages: ["erb", "haml", "slim"],
    summary: "An element with `data-turbo-permanent` and no `id`.",
    explanation:
      "Turbo keeps a permanent element across visits by matching its `id` on the next page, so without one the attribute does nothing. Read from ERB start tags, Haml and Slim tags, and the options of `tag.div`, `content_tag`, `link_to` and the form and button helpers; any value counts. An `id` of any kind (`#id` in Haml and Slim, an object reference such as `%div[@user]`) satisfies it. Nothing is reported where attributes may come from code that can't be read: ERB inside the start tag, a Haml `**attrs` or `data: attrs`, a Slim `*attrs` or `data=` hash, a helper's `**options`, or a value that isn't a literal. Rails projects only, in templates under app/views.",
    example: {
      file: "app/views/layouts/application.html.erb",
      lang: "erb",
      code: `<div data-turbo-permanent>
  <%= render 'shared/player' %>
</div>
`,
    },
    message:
      "`data-turbo-permanent` needs an `id`: Turbo matches the element across pages by it",
    line: 1,
    column: 6,
  },
  {
    code: "Stimulus/UnknownController",
    languages: ["erb", "haml", "slim"],
    summary: "A `data-controller` name that no Stimulus controller has.",
    explanation:
      'Each name in a static `data-controller` (also `data: { controller: "…" }` in Haml and in the options of `tag`, `content_tag`, `link_to`, `button_to` and the form helpers) must be a controller file in app/javascript/controllers (`search_controller.js`, `admin/users_controller.js` for `admin--users`; `-controller`, `.ts`, `.jsx`, `.tsx` and `.mjs` too) or a name a script in app/javascript registers with `application.register("name", …)`. The closest controller within two edits is offered. Nothing is reported when the app has no app/javascript/controllers folder, loads controllers from another folder (`eagerLoadControllersFrom("components", …)`, a context or glob outside the controllers folder, an importmap pin), registers one under a name the code computes, and not for a name with ERB or interpolation. Rails projects only, in templates under app/views.',
    example: {
      file: "app/views/users/index.html.erb",
      lang: "erb",
      code: `<div data-controller="serch">
  <%= text_field_tag :q %>
</div>
`,
    },
    message:
      "No Stimulus controller `serch` in app/javascript/controllers; did you mean `search`?",
    line: 1,
    column: 23,
  },
  {
    code: "Style/ErbTrimMode",
    languages: ["erb"],
    summary: "Both `%>` and `-%>` in one template.",
    explanation:
      "Every closer of the kind the template uses less is reported, `-%>` when there are as many of each. Comments (`<%#`) and literal `<%%` tags are not counted. Off by default: turn it on with `enabled = true` under `[Style.ErbTrimMode]` in .ruby-templates.toml.",
    example: {
      file: "app/views/users/index.html.erb",
      lang: "erb",
      code: `<% @users.each do |user| -%>
  <li><%= user.name %></li>
<% end %>
`,
    },
    message: "`-%>` here, but most closers in this template are `%>`",
    line: 1,
    column: 26,
  },
];

/** The codes each language's `disabledDiagnostics` setting in `package.json` accepts. */
function settingCodes(): Map<string, string[]> {
  // Builds run from site/; this module is bundled elsewhere, so its own path says nothing.
  const manifest = JSON.parse(
    fs.readFileSync(path.resolve(process.cwd(), "..", "package.json"), "utf8"),
  );
  const properties: Record<
    string,
    { items?: { anyOf?: { enum?: string[] }[] } }
  > = manifest.contributes.configuration.properties;
  return new Map(
    Object.entries(properties).flatMap(([key, value]) => {
      const lang = key.match(
        /^rubyTemplates\.(\w+)\.disabledDiagnostics$/,
      )?.[1];
      return lang ? [[lang, value.items?.anyOf?.[0]?.enum ?? []]] : [];
    }),
  );
}

/** Fails the build unless the entries cover exactly the codes the settings accept, language by language. */
function checkCoverage(entries: Diagnostic[], settings: Map<string, string[]>) {
  const setting = (lang: string) => `rubyTemplates.${lang}.disabledDiagnostics`;
  const missing = [...settings].flatMap(([lang, codes]) =>
    codes
      .filter(
        (code) =>
          !entries.some(
            (d) => d.code === code && d.languages.includes(lang as Language),
          ),
      )
      .map(
        (code) =>
          `no entry for \`${code}\` in ${LANGUAGE_NAMES[lang as Language] ?? lang} (${setting(lang)})`,
      ),
  );
  const stale = entries.flatMap((d) =>
    d.languages
      .filter((lang) => !settings.get(lang)?.includes(d.code))
      .map(
        (lang) =>
          `entry \`${d.code}\` lists ${LANGUAGE_NAMES[lang]}, but ${setting(lang)} has no such code`,
      ),
  );
  const problems = [...missing, ...stale];
  if (problems.length) {
    throw new Error(
      `site/src/data/diagnostics.ts is out of step with package.json:\n  ${problems.join("\n  ")}`,
    );
  }
}

checkCoverage(diagnostics, settingCodes());
