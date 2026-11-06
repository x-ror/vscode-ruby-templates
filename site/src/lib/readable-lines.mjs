// Expressive Code renders each line as a <div>, which reader views turn into paragraphs: indentation goes and blank
// lines appear. Lines become spans ending in a newline, like Listing's; the grid display keeps them looking the same.
const isElement = (node) => node.type === "element";
const isLine = (node) =>
  isElement(node) && node.properties?.className?.includes("ec-line");

function spanLines(node) {
  for (const child of node.children ?? []) {
    if (!isLine(child)) {
      if (isElement(child)) spanLines(child);
      continue;
    }
    child.tagName = "span";
    for (const part of child.children)
      if (isElement(part) && part.tagName === "div") part.tagName = "span";
    child.children.push({ type: "text", value: "\n" });
  }
}

export const readableLines = () => ({
  name: "readable-lines",
  hooks: {
    postprocessRenderedBlock: ({ renderData }) =>
      spanLines(renderData.blockAst),
  },
});
