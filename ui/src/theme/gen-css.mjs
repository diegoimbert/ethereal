// Regenerates tokens.css from tokens.ts (`just gen-tokens`). Works on Node >= 20: the TS
// sources are loaded through Vite's module runner, not Node type-stripping.
import { runnerImport } from "vite";

const { module } = await runnerImport(new URL("./tokens-plugin.ts", import.meta.url).pathname);
const changed = await module.writeTokensCss();
console.log(changed ? "tokens.css regenerated" : "tokens.css already up to date");
