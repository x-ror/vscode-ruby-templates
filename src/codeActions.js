// Quick fixes that edit files the server reads from disk go wrong when the user has unsaved changes there.

const editedUris = (action) =>
  action.edit?.entries().map(([uri]) => uri.toString()) ?? [];

/**
 * The code actions that leave unsaved files alone: those whose edit touches a URI in `dirtyUris`,
 * other than `ownUri` (the document the actions are for, which the server has in memory), are dropped.
 * @param {Array<{ edit?: { entries(): Array<[{ toString(): string }, unknown]> } }> | null | undefined} actions
 * @param {Set<string>} dirtyUris
 * @param {string} ownUri
 */
export function withoutDirtyEdits(actions, dirtyUris, ownUri) {
  return actions?.filter(
    (action) =>
      !editedUris(action).some((uri) => uri !== ownUri && dirtyUris.has(uri)),
  );
}
