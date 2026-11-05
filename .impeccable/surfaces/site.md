---
version: 1
slug: "site"
primary_target: "site"
related_targets: []
---

# Surface: Ruby Templates site (`site/`)

Scope: the GitHub Pages site. Landing page (`/`) is Persuade; guides, examples, diagnostics reference and changelog are Read and inherit the same world with less performance.

Audience and job: a Rails developer deciding whether to install the extension; the first viewport must prove it understands their templates (a real view, the server's own diagnostics, Rails-aware lookups). Primary action: install from the VS Code Marketplace. Proof: real templates highlighted with the extension's grammars, real diagnostic messages, screenshots in `images/screenshots`. No invented numbers, users or testimonials.

Avoid (user): stock Starlight look, marketing gloss (gradients, hype copy), ruby/gem cliché, terminal costume.

Hard rules (user): custom CSS in BEM; correct semantic HTML; WCAG 2.2 AA.

## Direction contract

THESIS: "Rails is omakase": the site is the chef's printed tasting card. One extension, menu already chosen; each course is a real Rails template served with what the server says about it. It refuses the centred-hero-over-feature-cards docs landing.

OWN-WORLD: sumi-black counter as the page ground, bright neutral-white menu card stock carrying all reading, crimson chef's seal (the icon's red) reserved for the install seal and fault marks only, indigo noren band for top navigation. Hairline menu rules, large course numerals, one sturdy grotesk for the card, the editor's mono for code. Errors wear one fault mark (shape + label in the margin), never colour alone. Dark theme: the card turns charcoal, the counter deepens, the seal lightens to keep AA.

STORY: the visitor reads the card top to bottom: what this is (one extension for ERB, Haml, Slim), sees course 01 plated (an ERB view with the server's real diagnostics), scans the remaining courses (highlighting, checks, Rails navigation, formatting, CI), presses the seal to install, or follows a course into the docs.

FIRST VIEWPORT: indigo noren band (wordmark, Docs, Examples, Diagnostics, Changelog, GitHub) over the black counter; one white menu card filling the content width. Card head: "Ruby Templates" wordmark, the omakase line, the course index 01–05 as links, and the crimson seal "Install for VS Code" at the head's right with Open VSX and npx as quiet text links. Below the head, course 01 runs full card width: a real `users/index.html.erb` with three fault marks in the gutter and their messages set under the code. The next course's rule and numeral show at the fold.

FORM: Omakase menu card, my list position 5 of 7, seed:524b314ada5d38bd. Signature interaction: the seal presses (ink settles, no bounce; static under reduced motion). Raises: errors by shape and scale, never colour alone (Massin); diagnostics reference as a finite roster with fixed furniture per code (character catalog); one fault grammar everywhere (depot blind); crimson reserved for seal and faults (VU meter); sections named by the Rails code they cover (industrial quotes); every course, example and diagnostic deep-linkable (cityscape).

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
