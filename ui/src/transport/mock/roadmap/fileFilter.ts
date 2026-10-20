/**
 * Mock of the library file filters (`Browser::{Get,Set,Reset,Check}FileFilter`,
 * CONTRACTS.md §12.8; engine: `crates/ether-controller/src/browser/filter.rs`): regex
 * whitelist/blacklist rules on file names, case-insensitive, blacklist wins, an empty
 * whitelist allows every file, folders are hidden only by the blacklist. JavaScript regexes
 * stand in for Rust's `regex` (the same for the usual rules). In memory; `Set`/`Reset` emit
 * `FileFilterChanged`. `MockTransport` filters `Media::ListDirectory` through `allowsFile` /
 * `allowsDir`; the library counts are over the mock library's files.
 */

import type { FileFilter, FileFilterCheck, FileFilterRuleError, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";
import { LIBRARY_FILES } from "../library";

export const MAX_FILE_FILTER_RULES = 64;
export const MAX_FILE_FILTER_RULE_LEN = 256;

/** The engine's defaults (`default_filter`). */
export function defaultFileFilter(): FileFilter {
  return {
    whitelist: [String.raw`\.(wav|wave|aif|aiff|aifc|flac|mp3|ogg|oga|m4a|aac)$`, String.raw`\.(mid|midi)$`, String.raw`\.(etherpreset|ethertemplate)$`],
    blacklist: [String.raw`^\.`, String.raw`^Thumbs\.db$`, String.raw`^desktop\.ini$`, String.raw`\.asd$`, String.raw`\.reapeaks$`],
  };
}

interface Compiled {
  whitelist: RegExp[];
  blacklist: RegExp[];
}

function compileList(list: "Whitelist" | "Blacklist", rules: string[], errors: FileFilterRuleError[]): RegExp[] {
  const out: RegExp[] = [];
  rules.forEach((rule, index) => {
    const error = (message: string) => errors.push({ list, index, message });
    if (index >= MAX_FILE_FILTER_RULES) return error(`at most ${MAX_FILE_FILTER_RULES} rules per list`);
    if (!rule.trim()) return error("empty rule");
    if (rule.length > MAX_FILE_FILTER_RULE_LEN) return error(`rules are at most ${MAX_FILE_FILTER_RULE_LEN} characters`);
    try {
      out.push(new RegExp(rule, "i"));
    } catch (e) {
      error(e instanceof Error ? e.message.replace(/^Invalid regular expression: /, "") : "invalid regular expression");
    }
  });
  return out;
}

export function compileFileFilter(f: FileFilter): { compiled: Compiled; errors: FileFilterRuleError[] } {
  const errors: FileFilterRuleError[] = [];
  const whitelist = compileList("Whitelist", f.whitelist, errors);
  const blacklist = compileList("Blacklist", f.blacklist, errors);
  return { compiled: { whitelist, blacklist }, errors };
}

const allowsDir = (c: Compiled, name: string) => !c.blacklist.some((r) => r.test(name));
const allowsFile = (c: Compiled, name: string) => (c.whitelist.length === 0 || c.whitelist.some((r) => r.test(name))) && allowsDir(c, name);
const allowsPath = (c: Compiled, path: string) => {
  const segs = path.split("/").filter(Boolean);
  const name = segs.pop();
  return name !== undefined && segs.every((d) => allowsDir(c, d)) && allowsFile(c, name);
};

export class MockFileFilter {
  private filter = defaultFileFilter();
  private compiled = compileFileFilter(this.filter).compiled;

  constructor(private readonly emit: (filter: FileFilter) => void) {}

  allowsFile(name: string): boolean {
    return allowsFile(this.compiled, name);
  }

  allowsDir(name: string): boolean {
    return allowsDir(this.compiled, name);
  }

  allowsPath(path: string): boolean {
    return allowsPath(this.compiled, path);
  }

  private reply(): ReplyValue {
    return { type: "BrowserFileFilter", filter: structuredClone(this.filter), defaults: defaultFileFilter() };
  }

  get(): ReplyValue {
    return this.reply();
  }

  set(f: FileFilter): ReplyValue {
    const { compiled, errors } = compileFileFilter(f);
    const e = errors[0];
    if (e) fail("InvalidArgument", `${e.list.toLowerCase()} rule ${e.index + 1}: ${e.message}`);
    this.filter = structuredClone(f);
    this.compiled = compiled;
    this.emit(structuredClone(f));
    return this.reply();
  }

  reset(): ReplyValue {
    return this.set(defaultFileFilter());
  }

  check(f: FileFilter | null, paths: string[]): ReplyValue {
    const { compiled, errors } = f ? compileFileFilter(f) : { compiled: this.compiled, errors: [] };
    const names = LIBRARY_FILES.map((x) => x.path.split("/").pop()!);
    const check: FileFilterCheck = {
      errors,
      visible: paths.map((p) => allowsPath(compiled, p)),
      library: { shown: names.filter((n) => allowsFile(compiled, n)).length, total: names.length },
    };
    return { type: "BrowserFileFilterCheck", check };
  }
}
