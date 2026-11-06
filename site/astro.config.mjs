// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import { LANGS } from "./src/lib/grammars.mjs";
import { THEMES } from "./src/lib/themes.mjs";
import { readableLines } from "./src/lib/readable-lines.mjs";

export default defineConfig({
  site: "https://x-ror.github.io",
  base: "/vscode-ruby-templates",
  integrations: [
    starlight({
      title: "Ruby Templates",
      description:
        "Rails view tooling for VS Code: one extension for ERB, Haml and Slim.",
      logo: { src: "../images/icon.png", alt: "" },
      favicon: "/favicon.png",
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/x-ror/vscode-ruby-templates",
        },
      ],
      editLink: {
        baseUrl:
          "https://github.com/x-ror/vscode-ruby-templates/edit/main/site/",
      },
      head: [{ tag: "meta", attrs: { name: "author", content: "x-ror" } }],
      components: {
        MarkdownContent: "./src/components/starlight/Article.astro",
      },
      customCss: ["./src/styles/fonts.css", "./src/styles/theme.css"],
      expressiveCode: {
        themes: [THEMES.dark, THEMES.light],
        plugins: [readableLines()],
        shiki: { langs: LANGS },
      },
      sidebar: [
        { label: "Start", items: ["getting-started"] },
        { label: "Guides", items: [{ autogenerate: { directory: "guides" } }] },
        {
          label: "Examples",
          items: [{ autogenerate: { directory: "examples" } }],
        },
        {
          label: "Reference",
          items: [{ autogenerate: { directory: "reference" } }],
        },
        { label: "Changelog", link: "/changelog/" },
      ],
    }),
  ],
});
