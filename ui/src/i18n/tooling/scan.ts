/**
 * Finds hard-coded UI text in TS/TSX (`pnpm i18n report`, and the ratchet in
 * catalogs.test.ts): JSX text, text attributes (title, aria-label, placeholder, label…),
 * text properties of objects (`{ label: "General" }`) and the message of `notify(…)`.
 * `// i18n-ignore` on the line (or the line above) skips one, for text that isn't
 * translatable (a brand, a note name, a sample string).
 */
import { readFileSync } from "node:fs";
import { relative } from "node:path";
import ts from "typescript";
import { UI_DIR } from "./check.ts";
import { isText } from "./text.ts";

export { isText };

export interface Finding {
  file: string;
  line: number;
  text: string;
}

const TEXT_ATTRS = new Set([
  "title",
  "label",
  "placeholder",
  "alt",
  "aria-label",
  "aria-valuetext",
  "aria-description",
  "aria-roledescription",
  "hint",
  "description",
  "tooltip",
  "message",
  "heading",
  "subtitle",
  "emptyLabel",
  "confirmLabel",
  "cancelLabel",
]);
const TEXT_PROPS = new Set([
  "label",
  "title",
  "description",
  "hint",
  "placeholder",
  "tooltip",
  "message",
  "heading",
  "subtitle",
  "ariaLabel",
  "aria-label",
  "closeLabel",
  "emptyLabel",
]);
/** Calls whose argument at this index is shown to the user. */
const TEXT_CALLS = new Map<string, number[]>([["notify", [1]]]);

function ignored(src: ts.SourceFile, node: ts.Node): boolean {
  const { line } = src.getLineAndCharacterOfPosition(node.getStart(src));
  const lines = src.text.split("\n");
  return /i18n-ignore/.test(lines[line] ?? "") || /i18n-ignore/.test(lines[line - 1] ?? "");
}

function stringOf(node: ts.Node | undefined): string | null {
  if (!node) return null;
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return node.text;
  if (ts.isTemplateExpression(node)) return [node.head.text, ...node.templateSpans.map((s) => s.literal.text)].join(" ");
  return null;
}

/** Strings an expression may evaluate to, through `a ? "x" : "y"`, `a && "x"`, `a ?? "x"` and parens. */
function textLeaves(node: ts.Expression | undefined, out: ts.Node[]): void {
  if (!node) return;
  if (stringOf(node) !== null) out.push(node);
  else if (ts.isParenthesizedExpression(node)) textLeaves(node.expression, out);
  else if (ts.isConditionalExpression(node)) {
    textLeaves(node.whenTrue, out);
    textLeaves(node.whenFalse, out);
  } else if (
    ts.isBinaryExpression(node) &&
    [ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken].includes(node.operatorToken.kind)
  ) {
    textLeaves(node.right, out);
    if (node.operatorToken.kind !== ts.SyntaxKind.AmpersandAmpersandToken) textLeaves(node.left, out);
  }
}

export function scanSource(file: string, code: string): Finding[] {
  const src = ts.createSourceFile(file, code, ts.ScriptTarget.Latest, true, file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const findings: Finding[] = [];
  const add = (node: ts.Node, text: string) => {
    if (!isText(text) || ignored(src, node)) return;
    findings.push({ file, line: src.getLineAndCharacterOfPosition(node.getStart(src)).line + 1, text: text.trim().replace(/\s+/g, " ") });
  };
  const addLeaves = (expr: ts.Expression | undefined) => {
    const leaves: ts.Node[] = [];
    textLeaves(expr, leaves);
    for (const l of leaves) add(l, stringOf(l)!);
  };
  const visit = (node: ts.Node) => {
    if (ts.isJsxText(node)) add(node, node.text);
    else if (ts.isJsxExpression(node) && node.parent && (ts.isJsxElement(node.parent) || ts.isJsxFragment(node.parent))) addLeaves(node.expression);
    else if (ts.isJsxAttribute(node)) {
      const name = node.name.getText(src);
      if (TEXT_ATTRS.has(name) && node.initializer) {
        if (ts.isStringLiteral(node.initializer)) add(node.initializer, node.initializer.text);
        else if (ts.isJsxExpression(node.initializer)) addLeaves(node.initializer.expression);
      }
    } else if (
      ts.isBindingElement(node) &&
      node.initializer &&
      ts.isIdentifier(node.name) &&
      (TEXT_PROPS.has(node.name.text) || TEXT_ATTRS.has(node.name.text))
    ) {
      // A text prop's default value: `function X({ label = "Notifications" })`.
      addLeaves(node.initializer);
    } else if (
      ts.isBinaryExpression(node) &&
      node.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
      ts.isPropertyAccessExpression(node.left) &&
      node.left.name.text === "title"
    ) {
      // `document.title = "…"`
      addLeaves(node.right);
    } else if (ts.isPropertyAssignment(node)) {
      const name = ts.isIdentifier(node.name) || ts.isStringLiteral(node.name) ? node.name.text : "";
      if (TEXT_PROPS.has(name)) addLeaves(node.initializer);
    } else if (ts.isCallExpression(node)) {
      const callee = ts.isIdentifier(node.expression) ? node.expression.text : ts.isPropertyAccessExpression(node.expression) ? node.expression.name.text : "";
      for (const i of TEXT_CALLS.get(callee) ?? []) addLeaves(node.arguments[i]);
    }
    ts.forEachChild(node, visit);
  };
  visit(src);
  return findings;
}

export function scanFiles(files: string[]): Finding[] {
  return files.flatMap((f) => scanSource(relative(UI_DIR, f), readFileSync(f, "utf8")));
}

export function formatFindings(findings: Finding[]): string {
  return findings.map((f) => `${f.file}:${f.line}  ${JSON.stringify(f.text)}`).join("\n");
}
