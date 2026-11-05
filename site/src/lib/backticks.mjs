/** `text` split at backticks, as Markdown quotes code: the odd parts are code. */
export function backtickParts(text) {
  return text.split("`").map((part, i) => ({ text: part, code: i % 2 === 1 }));
}
