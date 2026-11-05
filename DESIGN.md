---
name: Ruby Templates
description: The documentation site for one VS Code extension for Rails views, set as a printed omakase menu card.
colors:
  card: "#fcfcfa"
  ink: "#1a1a1f"
  ink-muted: "#55545c"
  hairline: "#dcdbe1"
  seal: "#c21f39"
  seal-ink: "#ffffff"
  fault: "#b3172f"
  link: "#34405f"
  code-bg: "#f4f4f1"
  selection: "#d6dcf3"
  card-dark: "#1c1b20"
  ink-dark: "#ecebef"
  ink-muted-dark: "#a9a8b1"
  hairline-dark: "#34333b"
  fault-dark: "#ff8696"
  link-dark: "#b9c1dc"
  code-bg-dark: "#22272e"
  selection-dark: "#34407a"
typography:
  display:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "clamp(2.75rem, 1.6rem + 4vw, 4.5rem)"
    fontWeight: 820
    lineHeight: 0.98
    letterSpacing: "-0.035em"
  docs-title:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "clamp(2.25rem, 1.6rem + 2.4vw, 3.25rem)"
    fontWeight: 800
    lineHeight: 1.12
    letterSpacing: "-0.03em"
  headline:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "clamp(1.625rem, 1.25rem + 1.4vw, 2.375rem)"
    fontWeight: 760
    lineHeight: 1.12
    letterSpacing: "-0.025em"
  numeral:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "clamp(2rem, 1.5rem + 2vw, 3.25rem)"
    fontWeight: 300
    lineHeight: 1
    letterSpacing: "-0.03em"
    fontFeature: "tnum"
  title:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "clamp(1.5rem, 1.25rem + 1vw, 2rem)"
    fontWeight: 760
    lineHeight: 1.15
    letterSpacing: "-0.02em"
  body:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "1.0625rem"
    fontWeight: 400
    lineHeight: 1.6
  label:
    fontFamily: "Schibsted Grotesk Variable, system-ui, sans-serif"
    fontSize: "0.875rem"
    fontWeight: 650
  code:
    fontFamily: "JetBrains Mono Variable, ui-monospace, monospace"
    fontSize: "0.875rem"
    lineHeight: 1.75
rounded:
  focus: "2px"
  mark: "3px"
  panel: "6px"
  seal-glyph: "7px"
  seal: "10px"
spacing:
  page-gutter: "clamp(1.25rem, 5vw, 4.5rem)"
  page-max: "78rem"
  course-top: "2.25rem"
  course-bottom: "2.5rem"
  course-gap: "2.5rem"
  stack: "1.25rem"
  row: "0.75rem"
components:
  seal:
    backgroundColor: "{colors.seal}"
    textColor: "{colors.seal-ink}"
    rounded: "{rounded.seal}"
    padding: "0.75rem 1.375rem 0.75rem 0.75rem"
  seal-glyph:
    textColor: "{colors.seal-ink}"
    rounded: "{rounded.seal-glyph}"
    size: "2.75rem"
  fault-mark:
    backgroundColor: "{colors.fault}"
    textColor: "{colors.card}"
    rounded: "{rounded.mark}"
    size: "1.5rem"
  listing:
    backgroundColor: "{colors.code-bg}"
    textColor: "{colors.ink}"
    typography: "{typography.code}"
    rounded: "{rounded.panel}"
  listing-file:
    textColor: "{colors.ink-muted}"
    padding: "0.5rem 1rem"
  fault-list:
    backgroundColor: "{colors.card}"
    textColor: "{colors.ink}"
    padding: "0.25rem 1rem 0.75rem"
  course-index-link:
    textColor: "{colors.ink}"
    padding: "0.75rem 1rem"
  course-index-link-hover:
    backgroundColor: "{colors.code-bg}"
  site-header-link:
    textColor: "{colors.ink-muted}"
  site-header-link-hover:
    textColor: "{colors.ink}"
  course-link:
    textColor: "{colors.link}"
  text-link:
    textColor: "{colors.link}"
---

# Design System: Ruby Templates

## Overview

**Creative North Star: "The Omakase Menu Card"**

The site is the chef's printed tasting card: one extension, the menu already chosen. Every page, landing and docs alike, is a single card-coloured reading surface ruled with hairlines, the way a printed menu is ruled. The header is the card itself, divided from the content by one rule. Sections are courses, numbered in a light numeral beside a heavy grotesk heading, and each course is a real Rails template served with what the language server says about it.

Colour is almost absent. Ink, a muted ink and a hairline carry the structure; a quiet slate-indigo carries links and focus; crimson appears in exactly two places, the install seal and the fault mark. Proof is typographic and literal: code is highlighted with the extension's own TextMate grammars in GitHub's light and dark-dimmed themes, recoloured so no syntax token is red, and diagnostics are the server's real messages, each pinned to its line by a numbered mark.

Density is that of a reference document, not a marketing page: generous measure limits, tight headings, tabular numbers, no panels, no gradients, no decorative imagery. The dark theme turns the card charcoal and keeps every rule the same.

**Key Characteristics:**

- One card-coloured surface for landing and docs; the header shares it, separated by a 1px hairline.
- Hairline single rules inside a section, a triple-line double rule between courses.
- Crimson only on the seal and fault marks.
- Numbered courses with the numeral beside the heading.
- Every course, code line, problem and diagnostic code is deep-linkable by anchor.
- Two self-hosted variable faces: Schibsted Grotesk for text, JetBrains Mono for code.

## Colors

A near-monochrome card in ink and hairline, one quiet slate-indigo for links and focus, and one reserved crimson.

### Primary

- **Chef's Seal Crimson** (seal): the install seal's ground, with white ink. Same value in both themes. The red of the extension icon.
- **Fault Crimson** (fault / fault-dark): the fault mark's ground and, as a 9% tint, the background of a line that carries a fault. Lightens to a pink in dark mode to keep contrast on the charcoal card.

### Secondary

- **Quiet Slate Indigo** (link / link-dark): every text link, the fault-list location links, Starlight's accent, and the focus ring. Underlined at 1px, thickening to 2px on hover; never a fill.

### Neutral

- **Menu Card** (card / card-dark): the only page ground, landing and docs, header and sidebar included.
- **Ink** (ink / ink-dark): headings, body text in the docs, table rows, code text outside tokens.
- **Muted Ink** (ink-muted / ink-muted-dark): ledes, course text, course numerals, nav links at rest, file names, labels.
- **Hairline** (hairline / hairline-dark): every rule and border.
- **Code Ground** (code-bg / code-bg-dark): listings, setting snippets, the current sidebar page, course-index hover.
- **Selection** (selection / selection-dark): text selection, with ink.

### Named Rules

**The Reserved Crimson Rule.** Crimson (seal, fault) appears only on the install seal and on fault marks, including the faint tint of a faulted code line. No crimson headings, borders, links, badges or hover states.

**The Quiet Link Rule.** The slate indigo is a link and focus colour only. It never fills a surface, a band or a button.

**The Target Wash Rule.** A targeted line or problem (`:target`) is washed with the focus colour (16% on a line, 12% on a problem), never with crimson.

**The Red-Free Code Rule.** Syntax highlighting never uses red: GitHub's reds become purple (#6f42c1 light, #dcbdfb dark), and muted tokens are darkened or lightened to 4.5:1, so a red thing in a listing is always a fault.

## Typography

**Display Font:** Schibsted Grotesk Variable (with system-ui, sans-serif)
**Body Font:** Schibsted Grotesk Variable
**Label/Mono Font:** JetBrains Mono Variable (with ui-monospace, monospace)

**Character:** One sturdy grotesk set heavy and tight for headings and plainly for reading, beside the editor's mono for everything that is code: file names, diagnostic codes, settings, inline `code`. Both are self-hosted through Fontsource and loaded on every page.

### Hierarchy

- **Display** (820, fluid 2.75 to 4.5rem, 0.98): the landing page's one h1, balanced.
- **Headline** (760, fluid 1.625 to 2.375rem, 1.12): course headings, max 28ch unless the course is wide. Docs page titles (Starlight's h1) take the display voice at 800, -0.03em, fluid 2.25 to 3.25rem, 1.12.
- **Numeral** (300, fluid 2 to 3.25rem, tabular, muted): course numbers beside their heading.
- **Title** (760, fluid 1.5 to 2rem): diagnostics roster section titles; diagnostic codes themselves in mono 700 at 1.25rem.
- **Body** (400, 1.0625rem, 1.6): landing text; course text at max 68ch, ledes at 62ch, `text-wrap: pretty`.
- **Label** (650, 0.875rem): table column heads, roster terms, fault locations. Sentence case; no uppercase, no tracking.
- **Code** (mono, 0.875rem, 1.75): listings; file captions and fault codes at 0.8125rem muted.

### Named Rules

**The Weight Not Case Rule.** Emphasis comes from weight (550 to 820) and negative tracking on headings, never from uppercase or letter-spaced labels.

**The Tabular Count Rule.** Line numbers, course numerals and docs tables use tabular figures.

## Layout

Single column, max 78rem, centred, with a fluid gutter (clamp 1.25rem, 5vw, 4.5rem) shared by the header, card and footer. At 56rem the card head splits into intro and the seal column, and each course becomes a two-column grid: a 5.5rem numeral column and the body. Courses pad 2.25rem above and 2.5rem below. At 64rem listings with problems may set the problem list beside the code (1.35fr to 1fr), and paired listings (the tasting, before/after) go two-up. Below 40rem the course index stays one row that scrolls horizontally (labels do not wrap) so the first course stays in view, the header links tighten to a 0.25rem by 1rem gap, and table row heads wrap. Roster fields go label-beside-detail (8rem) from 50rem.

Anchored elements set `scroll-margin` so a deep link lands below the header (6rem for lines and problems; nav height plus 1.5rem in the docs). Smooth scrolling only without reduced motion.

## Elevation & Depth

Flat. Depth is conveyed by rules and the code ground, not by shadow. The single exception is the seal, which carries a soft crimson-dark shadow that shrinks as it is pressed; Starlight's pagination shadows are removed.

### Shadow Vocabulary

- **Seal at rest** (`box-shadow: 0 6px 14px -4px var(--rt-seal-shadow)`): the install seal only.
- **Seal pressed** (`0 3px 8px -3px` on hover, `0 1px 2px -1px` on active): the stamp settling.

### Named Rules

**The One Raised Thing Rule.** Only the install seal casts a shadow. Panels, listings, nav and pagination are flat.

## Shapes

Mostly square. Small radii only where an object is a thing in itself: listings and setting snippets (6px), the header icon (6px), the fault mark (3px), the seal (10px) with its inner glyph frame (7px). Rules are 1px hairlines; between courses and roster sections the rule is a 3px double hairline, the printed-menu divider. The fault mark is a rounded square rotated 45 degrees, drawn behind an upright number inside an unrotated 24px target.

## Components

### Seal (install action)

The chef's stamp: the one primary action on the landing page.

- **Shape:** gently rounded (10px), with a 2.75rem framed glyph (`<%=` in mono 800, 2px white frame, 7px radius) before the label.
- **Colour:** crimson ground, white label at 700, 1.0625rem.
- **Press:** with motion allowed, hover lowers it 1px and shrinks the shadow; active lowers 2px and scales to 0.985 (180ms, ease-out-expo). Static under reduced motion. Same focus ring as everything else.
- **Companions:** Open VSX and npm as quiet text links beneath, never as secondary buttons.

### Fault Mark

The one sign every diagnostic wears.

- **Shape:** an unrotated 1.5rem (24px) target; behind its number a 1.0625rem rounded square (3px) rotated 45 degrees is drawn as a pseudo-element. The number stays upright, in the grotesk at 800, 0.6875rem.
- **Colour:** fault ground, card-coloured number.
- **Behaviour:** in a listing's gutter it is a link to its problem (`aria-label` "Problem n on line m"); in the problem list it is decorative and the location links back to the line.

### Listing

A template file with line numbers, highlighting and its problems.

- **Container:** code ground, 1px hairline border, 6px radius; a figure whose caption is the file name in muted mono. The scrolling code is a focusable region named after the file.
- **Lines:** a 2.75rem line-number column, a 1.75rem fault gutter, then the code; every line has an anchor id. A faulted line takes a 9% fault tint; a targeted line a 16% focus wash.
- **Problem list:** an ordered list on the card ground under the code (or beside it from 64rem): mark, linked "Line n, column m", the server's message, and the diagnostic code in muted mono, rows divided by hairlines.
- **Highlighting:** the extension's own grammars through Shiki. The themes are GitHub light and dark-dimmed, recoloured by one shared module that both Expressive Code and the landing listings use. Each token carries both theme colours and switches with the theme.

### Course

A numbered section of the card.

- **Structure:** a `section` labelled by its h2; the numeral sits in the left column (aria-hidden), the heading is a self-link to the section anchor, then muted course text, the proof, and a single "more" link in slate indigo (1px underline, 2px on hover).
- **Divider:** a 3px double hairline between consecutive courses.

### Course Index

The card's list of courses: an ordered list in a hairline-ruled row of cells, each a numeral (muted, 400, tabular) and the course name (600). Hover fills the cell with the code ground.

### Navigation

- **Landing header:** wordmark (icon plus "Ruby Templates" at 750) and a plain list of links in muted ink at 550; hover turns ink and draws a 2px underline border. Card ground, hairline below.
- **Docs (Starlight):** same card ground for header and sidebar, hairline below the header; the current page is marked by 650 weight on the code ground, not a coloured pill; previous/next are plain hairline links without shadow.

### Pairing Table

What each Rails construct gets: a captioned table, row headers in the left 34%, a 1px ink rule under the column heads and hairlines between rows.

### Diagnostics Roster

A finite roster with fixed furniture per code: an index of every code grouped by language set, then one article per code (mono heading that self-links), a summary, and the same four fields every time (Languages, When it fires, Example as a Listing with its fault mark, Turn it off as a settings snippet).

## Do's and Don'ts

### Do:

- **Do** keep landing and docs on the one card ground, with the header separated by a single 1px hairline.
- **Do** mark every diagnostic with the numbered fault mark plus a numbered problem list entry giving line, column, message and code; never by colour alone.
- **Do** give every course, code line, problem and diagnostic code an anchor id, and link marks to problems and problems back to lines.
- **Do** number courses and put the numeral beside the heading, in the left column.
- **Do** use the one focus ring: 2px solid slate indigo (focus), offset 2px everywhere and -2px inside scrolling code blocks.
- **Do** name custom CSS in BEM (`block__element--modifier`) and build with semantic HTML: `section` labelled by its heading, `figure`/`figcaption` for listings, `ol` for ordered things, `dl` for roster furniture, captioned tables with scoped headers.
- **Do** meet WCAG 2.2 AA in both themes, including every syntax token at 4.5:1 on the code ground and on a fault line's tint, 24px fault-mark targets, and scrolling code blocks that are focusable named regions (`tabindex="0" role="region" aria-label`).
- **Do** highlight template code with the extension's own grammars and quote the server's real messages.

### Don't:

- **Don't** use crimson anywhere but the install seal and fault marks.
- **Don't** put an eyebrow, kicker or uppercase label above a heading; the course numeral beside it is the only marker.
- **Don't** reintroduce a dark counter frame, a coloured navigation band or a coloured surface behind the card.
- **Don't** fill surfaces or buttons with the link colour.
- **Don't** add shadows to anything but the seal, or card-style panels around content.
- **Don't** load display or text faces from a third-party CDN; both faces are self-hosted.
