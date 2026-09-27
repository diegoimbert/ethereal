// Regenerates tokens.css from tokens.ts. Run: `node ui/src/theme/gen-css.ts` (Node >= 23.6).
import { writeFileSync } from "node:fs";
import { renderTokensCss } from "./css.ts";

const out = new URL("./tokens.css", import.meta.url);
writeFileSync(out, renderTokensCss());
console.log(`wrote ${out.pathname}`);
