/** The property a synthetic keydown from `runAction` (run.ts) carries: the action id. */
export const ACTION_TAG = "ethAction";

/** The action a synthetic event from `runAction` carries (DOM or React event), if any. */
export function taggedAction(e: object): string | undefined {
  const own = (e as { [ACTION_TAG]?: string })[ACTION_TAG];
  if (own !== undefined) return own;
  return (e as { nativeEvent?: { [ACTION_TAG]?: string } }).nativeEvent?.[ACTION_TAG];
}
