import { execFileSync } from "node:child_process";
import path from "node:path";

const parseEntry = (line) => {
  const [date, author] = line.split("\t");
  return { date: new Date(date), author };
};

/** When a file was first committed, and when and by whom it last changed; undefined outside git or for an uncommitted file. */
export function fileHistory(file) {
  let log;
  try {
    log = execFileSync(
      "git",
      ["log", "--follow", "--format=%aI%x09%an", "--", path.basename(file)],
      {
        cwd: path.dirname(file),
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      },
    );
  } catch {
    // No git, or not a repository (a source archive): the page simply has no dates.
    return undefined;
  }
  const entries = log.trim().split("\n").filter(Boolean).map(parseEntry);
  if (!entries.length) return undefined;
  return { published: entries.at(-1).date, updated: entries[0] };
}
