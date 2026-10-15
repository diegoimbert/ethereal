/**
 * Build-time side of the catalogs (Node only: never imported by the app). Parses each ICU
 * message once; the app gets plain strings or ASTs and ships no parser.
 */
import { parse, type MessageFormatElement } from "@formatjs/icu-messageformat-parser";
import type { CompiledCatalog, CompiledMessage } from "../types.ts";

/** Parses one ICU message (throws a `SyntaxError`-like error on bad syntax). */
export function parseMessage(source: string): MessageFormatElement[] {
  return parse(source, { requiresOtherClause: true, shouldParseSkeletons: true });
}

/** A message without ICU syntax becomes its text; anything else its AST. */
export function compileMessage(source: string): CompiledMessage {
  const ast = parseMessage(source);
  if (ast.length === 0) return "";
  const [first] = ast;
  if (ast.length === 1 && first && first.type === 0) return first.value;
  return stripLocations(ast);
}

function stripLocations(ast: MessageFormatElement[]): MessageFormatElement[] {
  return JSON.parse(JSON.stringify(ast, (k, v) => (k === "location" ? undefined : v))) as MessageFormatElement[];
}

/** A catalog file's text → its compiled form (keys kept in order). */
export function compileCatalog(json: string): CompiledCatalog {
  const raw = JSON.parse(json) as Record<string, string>;
  const out: Record<string, CompiledMessage> = {};
  for (const [key, msg] of Object.entries(raw)) {
    try {
      out[key] = compileMessage(msg);
    } catch (e) {
      throw new Error(`i18n: bad ICU message "${key}": ${e instanceof Error ? e.message : String(e)}`, { cause: e });
    }
  }
  return out;
}
