// Finds capitalized words in the string literals the UI shows (#524), for `uiCase.test.ts`.
// Test-only (imports the TypeScript compiler); the rules live in `uiCase.ts`.
//
// What counts as UI text, by syntax:
// - JSX text, and string literals in a JSX child expression (`{busy ? "saving…" : "save"}`);
// - the value of a text-bearing JSX attribute (`title`, `aria-label`, `placeholder`, `label`, `*Label`...);
// - an object property with a text-bearing name (`label: "..."`, `title`, `message`, `hint`...);
// - the message of `notify(level, message)` and `confirm`/`alert`/`prompt`;
// - the values of a variable, or the strings `return`ed by a function, whose name says it holds
//   labels (`KIND_LABEL = {...}`, `text = "..."`, `function trackKindLabel() { return "..." }`, `formatGridStep`).
// - any other literal that reads as words (two words or more), outside `console.*` calls.
// Strings reach these through `?:`, `||`, `??`, `+`, templates, arrays and the literal arguments of a call.
// A literal on a line with `eth-allow-case` (or the line after one) is skipped, and a file
// containing `eth-allow-case-file` is skipped whole.
import ts from "typescript";
import { badWords, lowerUiText } from "./uiCase.ts";

/** Attribute and property names whose string values are shown to people. */
const TEXT_NAME =
  /^(title|label|text|submit|placeholder|alt|tooltip|hint|description|message|caption|heading|subtitle|body|detail|summary|empty|confirm|group|aria|aria-label|aria-description|aria-valuetext|aria-roledescription|aria-placeholder)$|(Label|Title|Text|Hint|Message|Placeholder|Tooltip|Description|Caption|Heading|Summary)$/;

/** Variable or function names that hold or produce labels. */
const LABEL_HOLDER = {
  test: (name: string) =>
    /(label|title|text|hint|message|caption|heading|tooltip|placeholder|description|summary)s?$|^describe/i.test(name) ||
    /^(format[A-Z]|action$|verb$)/.test(name),
};

/** Functions whose string arguments are shown as-is, by argument index. */
const SHOWN_ARGS: Record<string, readonly number[]> = { notify: [1], confirm: [0], alert: [0], prompt: [0, 1] };

const ALLOW_MARK = "eth-allow-case";
/** A file whose strings are not UI text (prompts for an LLM, fixtures of third-party names). */
const FILE_ALLOW_MARK = "eth-allow-case-file";

export interface CaseFinding {
  file: string;
  line: number;
  word: string;
  text: string;
}

type Piece = ts.StringLiteral | ts.NoSubstitutionTemplateLiteral | ts.TemplateLiteralLikeNode | ts.JsxText;

/** Any literal that reads as words ("couldn't reach the server"): UI text unless it is code (CSS, SVG, URLs...). */
function isProse(n: ts.Node): n is Piece {
  let t: string;
  if (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) t = n.text;
  else if (ts.isTemplateHead(n) || ts.isTemplateMiddle(n) || ts.isTemplateTail(n)) {
    // a template's parts read as words together: `couldn't reach ${host}. is it running?`
    const tpl = ts.isTemplateHead(n) ? n.parent : n.parent.parent;
    if (!ts.isTemplateExpression(tpl)) return false;
    t = [tpl.head.text, ...tpl.templateSpans.map((s) => s.literal.text)].join(" ");
    // a word next to a substitution reads as text too: `${verb} ${count} Tracks`
    if (/(^|\s)[A-Za-z][a-z]{2,}(\s|$)/.test(t.trim() === t ? t : ` ${t} `) && /\s[A-Z][a-z]{2,}\b|\b[A-Z][a-z]{2,}\s/.test(t) && !/[{}=<>:;]|https?:/.test(t))
      return true;
  } else return false;
  return /[A-Za-z]{2,}(['’][a-z]+)?[^\sA-Za-z]*\s[^A-Za-z]*[A-Za-z]/.test(t) && !/[{}=<>]|https?:|\b(var|calc|translate[XYZ]?|rgba?)\(|^\s*[Mm]\s?-?\d/.test(t);
}

/** `console.*(...)`: developer output, never shown in the UI. */
function isConsoleCall(n: ts.Node): boolean {
  return (
    ts.isCallExpression(n) &&
    ts.isPropertyAccessExpression(n.expression) &&
    ts.isIdentifier(n.expression.expression) &&
    n.expression.expression.text === "console"
  );
}

function nameOf(n: ts.Node | undefined): string | undefined {
  if (!n) return undefined;
  if (ts.isIdentifier(n) || ts.isStringLiteral(n) || ts.isPrivateIdentifier(n)) return n.text;
  if (ts.isJsxNamespacedName(n)) return `${n.namespace.text}:${n.name.text}`;
  return undefined;
}

/** Literal pieces reachable from `e` without a call. */
function shown(e: ts.Node, out: Piece[]): void {
  if (ts.isStringLiteral(e) || ts.isNoSubstitutionTemplateLiteral(e)) out.push(e);
  else if (ts.isTemplateExpression(e)) {
    out.push(e.head);
    for (const s of e.templateSpans) {
      shown(s.expression, out);
      out.push(s.literal);
    }
  } else if (ts.isParenthesizedExpression(e) || ts.isAsExpression(e) || ts.isSatisfiesExpression(e) || ts.isNonNullExpression(e)) shown(e.expression, out);
  else if (ts.isConditionalExpression(e)) {
    shown(e.whenTrue, out);
    shown(e.whenFalse, out);
  } else if (ts.isBinaryExpression(e)) {
    const op = e.operatorToken.kind;
    if (op === ts.SyntaxKind.PlusToken || op === ts.SyntaxKind.BarBarToken || op === ts.SyntaxKind.QuestionQuestionToken) {
      shown(e.left, out);
      shown(e.right, out);
    } else if (op === ts.SyntaxKind.AmpersandAmpersandToken) shown(e.right, out);
  } else if (ts.isArrayLiteralExpression(e)) for (const x of e.elements) shown(x, out);
  else if (ts.isCallExpression(e)) {
    // a text helper's literal arguments: `plural(n, "Delete clip", "Delete # clips")`
    for (const a of e.arguments) if (ts.isStringLiteral(a) || ts.isNoSubstitutionTemplateLiteral(a) || ts.isTemplateExpression(a)) shown(a, out);
  } else if (ts.isJsxExpression(e) && e.expression) shown(e.expression, out);
}

function enclosingFunctionName(n: ts.Node): string | undefined {
  for (let p: ts.Node | undefined = n.parent; p; p = p.parent) {
    if (ts.isFunctionDeclaration(p) || ts.isMethodDeclaration(p)) return nameOf(p.name);
    if (ts.isArrowFunction(p) || ts.isFunctionExpression(p)) {
      const d = p.parent;
      return d && (ts.isVariableDeclaration(d) || ts.isPropertyAssignment(d)) ? nameOf(d.name) : undefined;
    }
  }
  return undefined;
}

/** The literal pieces of UI text in one source file. */
/** This file's functions by name, with the indices of their parameters that hold text (`(label: string, ...)`). */
function textParams(sf: ts.SourceFile): Map<string, number[]> {
  const out = new Map<string, number[]>();
  const add = (name: string | undefined, params: ts.NodeArray<ts.ParameterDeclaration>) => {
    const idx = params.flatMap((p, i) => (TEXT_NAME.test(nameOf(p.name) ?? "") || LABEL_HOLDER.test(nameOf(p.name) ?? "") ? [i] : []));
    if (name && idx.length) out.set(name, idx);
  };
  const visit = (n: ts.Node) => {
    if (ts.isFunctionDeclaration(n)) add(nameOf(n.name), n.parameters);
    else if (ts.isVariableDeclaration(n) && n.initializer && (ts.isArrowFunction(n.initializer) || ts.isFunctionExpression(n.initializer)))
      add(nameOf(n.name), n.initializer.parameters);
    ts.forEachChild(n, visit);
  };
  visit(sf);
  return out;
}

function uiPieces(sf: ts.SourceFile): Piece[] {
  const out: Piece[] = [];
  const local = textParams(sf);
  const visit = (n: ts.Node) => {
    if (ts.isImportDeclaration(n) || ts.isExportDeclaration(n) || ts.isLiteralTypeNode(n) || isConsoleCall(n)) return;
    if (isProse(n)) out.push(n);
    if (ts.isJsxText(n)) {
      if (n.text.trim()) out.push(n);
    } else if (ts.isJsxExpression(n) && n.expression && (ts.isJsxElement(n.parent) || ts.isJsxFragment(n.parent))) {
      shown(n.expression, out);
    } else if ((ts.isJsxAttribute(n) || ts.isPropertyAssignment(n)) && TEXT_NAME.test(nameOf(n.name) ?? "")) {
      if (n.initializer) shown(n.initializer, out);
    } else if (ts.isCallExpression(n)) {
      const callee = ts.isIdentifier(n.expression) ? n.expression.text : ts.isPropertyAccessExpression(n.expression) ? n.expression.name.text : "";
      // `notify(level, message)`, or a local helper whose parameter is named for text (`edit("cut", ...)`)
      const shownArgs = (Object.hasOwn(SHOWN_ARGS, callee) && SHOWN_ARGS[callee]) || (ts.isIdentifier(n.expression) && local.get(callee)) || [];
      for (const i of shownArgs) {
        const arg = n.arguments[i];
        if (arg) shown(arg, out);
      }
    } else if (ts.isVariableDeclaration(n) && n.initializer && LABEL_HOLDER.test(nameOf(n.name) ?? "")) {
      let init: ts.Expression = n.initializer;
      while (ts.isAsExpression(init) || ts.isSatisfiesExpression(init) || ts.isParenthesizedExpression(init)) init = init.expression;
      if (ts.isObjectLiteralExpression(init)) {
        for (const p of init.properties) if (ts.isPropertyAssignment(p)) shown(p.initializer, out);
      } else shown(init, out);
    } else if (
      (ts.isBindingElement(n) || ts.isParameter(n)) &&
      n.initializer &&
      (TEXT_NAME.test(nameOf(n.name) ?? "") || LABEL_HOLDER.test(nameOf(n.name) ?? ""))
    ) {
      // a default prop value: `{ label = "notifications" }`
      shown(n.initializer, out);
    } else if (ts.isBinaryExpression(n) && n.operatorToken.kind === ts.SyntaxKind.EqualsToken && LABEL_HOLDER.test(nameOf(n.left) ?? "")) {
      shown(n.right, out);
    } else if (ts.isReturnStatement(n) && n.expression) {
      if (LABEL_HOLDER.test(enclosingFunctionName(n) ?? "")) shown(n.expression, out);
    } else if (ts.isArrowFunction(n) && !ts.isBlock(n.body)) {
      if (LABEL_HOLDER.test(enclosingFunctionName(n.body) ?? "")) shown(n.body, out);
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
  return [...new Set(out)];
}

function parse(file: string, src: string): ts.SourceFile {
  return ts.createSourceFile(file, src, ts.ScriptTarget.Latest, true, file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
}

function checkedPieces(file: string, src: string): Array<{ piece: Piece; line: number; sf: ts.SourceFile }> {
  const sf = parse(file, src);
  const lines = src.split("\n");
  if (src.includes(FILE_ALLOW_MARK)) return [];
  return uiPieces(sf)
    .map((piece) => ({ piece, line: sf.getLineAndCharacterOfPosition(piece.getStart(sf)).line, sf }))
    .filter(({ line }) => !lines[line]?.includes(ALLOW_MARK) && !lines[line - 1]?.includes(ALLOW_MARK));
}

/** Capitalized words in the UI text of one file. */
export function scanCase(file: string, src: string): CaseFinding[] {
  return checkedPieces(file, src).flatMap(({ piece, line, sf }) => {
    const text = piece.getText(sf);
    return badWords(text).map((word) => ({ file, line: line + 1, word, text: text.trim().slice(0, 100) }));
  });
}

/** `src` with its UI text lowercased (the fixer behind the sweep; also handy for new files). */
export function fixCase(file: string, src: string): string {
  const edits = checkedPieces(file, src)
    .map(({ piece, sf }) => ({ start: piece.getStart(sf), end: piece.getEnd(), text: lowerUiText(piece.getText(sf)) }))
    .sort((a, b) => b.start - a.start);
  let out = src;
  for (const e of edits) out = out.slice(0, e.start) + e.text + out.slice(e.end);
  return out;
}

/** The UI texts of one file (cooked: escapes resolved). */
export function uiTexts(file: string, src: string): string[] {
  return checkedPieces(file, src).map(({ piece }) => piece.text);
}

export function formatCaseFindings(findings: CaseFinding[]): string {
  return findings.map((f) => `${f.file}:${f.line}  ${f.word}  in ${f.text}`).join("\n");
}
