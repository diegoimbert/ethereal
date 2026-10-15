#!/usr/bin/env node
// Ethereal's CLA bot (see CLA.md and LICENSING.md).
//
// Runs from .github/workflows/cla.yml on `pull_request_target` and `issue_comment`. It checks
// whether the pull request author has signed the CLA (an entry in cla-signatures.json on the
// `cla-signatures` branch) and reports a `CLA` commit status on the pull request head, so
// branch protection can require it. A comment with the signing phrase records a signature.
//
// It is self-contained: it uses only the workflow token (contents/pull-requests/statuses
// write) and the REST API. It never checks out or executes pull-request code, and it stores
// nothing outside this repository.
import { readFileSync } from "node:fs";

const API = "https://api.github.com";
const SERVER = process.env.GITHUB_SERVER_URL || "https://github.com";
const [owner, repo] = (process.env.GITHUB_REPOSITORY || "").split("/");
const token = process.env.GITHUB_TOKEN;
const CLA_VERSION = process.env.CLA_VERSION || "1";
const BRANCH = "cla-signatures";
const FILE = "cla-signatures.json";
const SIGN_PHRASE = "i have read the cla document and i hereby sign the cla";
const RECHECK = "recheck";
const MARKER = "<!-- ethereal-cla -->";
const ALLOWLIST = new Set(
  (process.env.CLA_ALLOWLIST || "")
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean),
);

if (!owner || !repo || !token) {
  console.error("cla: GITHUB_REPOSITORY and GITHUB_TOKEN are required");
  process.exit(1);
}

const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
const defaultBranch = event.repository?.default_branch || "main";

async function gh(path, options = {}) {
  const res = await fetch(`${API}${path}`, {
    ...options,
    headers: {
      authorization: `Bearer ${token}`,
      accept: "application/vnd.github+json",
      "x-github-api-version": "2022-11-28",
      "user-agent": "ethereal-cla-bot",
      ...(options.headers || {}),
    },
  });
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`${options.method || "GET"} ${path} -> ${res.status} ${await res.text()}`);
  return res.status === 204 ? null : res.json();
}

const isExempt = (user) => !user || user.type === "Bot" || /\[bot\]$/.test(user.login) || ALLOWLIST.has(user.login.toLowerCase());

// Tolerate people quoting the request ("&gt; I have read…") or wrapping it in punctuation.
const normalize = (s) =>
  (s || "")
    .toLowerCase()
    .replace(/^[>\s]+/, "")
    .replace(/[`*_"'.\s]+$/, "")
    .replace(/\s+/g, " ")
    .trim();

const isSigned = (data, login) => {
  const entry = data[login?.toLowerCase()];
  return Boolean(entry && String(entry.version) === String(CLA_VERSION));
};

async function readSignatures() {
  const file = await gh(`/repos/${owner}/${repo}/contents/${FILE}?ref=${BRANCH}`);
  if (!file) return { data: {}, sha: undefined };
  return { data: JSON.parse(Buffer.from(file.content.replace(/\n/g, ""), "base64").toString("utf8")), sha: file.sha };
}

async function writeSignatures(data, fileSha) {
  const ref = await gh(`/repos/${owner}/${repo}/git/ref/heads/${BRANCH}`);
  if (!ref) {
    const base = await gh(`/repos/${owner}/${repo}/git/ref/heads/${defaultBranch}`);
    try {
      await gh(`/repos/${owner}/${repo}/git/refs`, { method: "POST", body: JSON.stringify({ ref: `refs/heads/${BRANCH}`, sha: base.object.sha }) });
    } catch (e) {
      if (!/already exists/i.test(String(e.message))) throw e; // lost a creation race; fine
    }
  }
  const content = Buffer.from(`${JSON.stringify(data, null, 2)}\n`, "utf8").toString("base64");
  await gh(`/repos/${owner}/${repo}/contents/${FILE}`, {
    method: "PUT",
    body: JSON.stringify({ message: "CLA: record signature", content, branch: BRANCH, ...(fileSha ? { sha: fileSha } : {}) }),
  });
}

async function setStatus(sha, state, description) {
  const url = `${SERVER}/${owner}/${repo}/blob/${defaultBranch}/CLA.md`;
  await gh(`/repos/${owner}/${repo}/statuses/${sha}`, {
    method: "POST",
    body: JSON.stringify({ state, context: "CLA", description, target_url: url }),
  });
}

async function comments(number) {
  return (await gh(`/repos/${owner}/${repo}/issues/${number}/comments?per_page=100`)) || [];
}

async function commentOnce(number, body) {
  if ((await comments(number)).some((c) => (c.body || "").includes(MARKER))) return;
  await gh(`/repos/${owner}/${repo}/issues/${number}/comments`, { method: "POST", body: JSON.stringify({ body }) });
}

function signRequest(login) {
  const url = `${SERVER}/${owner}/${repo}/blob/${defaultBranch}/CLA.md`;
  return [
    MARKER,
    `Thanks for the pull request, @${login}! Before it can be merged, please sign our`,
    `[Contributor License Agreement](${url}) by posting a comment on this pull request with exactly:`,
    "",
    "> I have read the CLA Document and I hereby sign the CLA",
    "",
    "You only have to do this once. If the `CLA` check does not turn green after signing, comment `recheck`.",
  ].join("\n");
}

// The signing phrase is accepted from any non-bot commenter (a person signs for themselves).
// The pull request's status always reflects its author.
async function recordSignature(commenter, prNumber) {
  const { data, sha } = await readSignatures();
  const key = commenter.login.toLowerCase();
  if (isSigned(data, commenter.login)) return false;
  data[key] = { login: commenter.login, version: CLA_VERSION, signed_at: new Date().toISOString(), pull_request: prNumber };
  await writeSignatures(data, sha);
  return true;
}

async function evaluate(pr) {
  const login = pr.user.login;
  const sha = pr.head.sha;
  if (isExempt(pr.user)) {
    await setStatus(sha, "success", "No CLA required");
    return true;
  }
  const { data } = await readSignatures();
  const signed = isSigned(data, login);
  await setStatus(sha, signed ? "success" : "failure", signed ? "CLA signed" : "CLA not signed");
  return signed;
}

async function main() {
  const name = process.env.GITHUB_EVENT_NAME;

  if (name === "pull_request_target") {
    const pr = event.pull_request;
    const signed = await evaluate(pr);
    if (!signed && (event.action === "opened" || event.action === "reopened")) {
      await commentOnce(pr.number, signRequest(pr.user.login));
    }
    return;
  }

  if (name === "issue_comment") {
    if (!event.issue.pull_request) return; // a plain issue, not a pull request
    const body = normalize(event.comment.body);
    if (body !== SIGN_PHRASE && body !== RECHECK) return;
    const number = event.issue.number;
    const pr = await gh(`/repos/${owner}/${repo}/pulls/${number}`);
    const isAuthor = event.comment.user.login.toLowerCase() === pr.user.login.toLowerCase();
    let justSigned = false;
    if (body === SIGN_PHRASE && !isExempt(event.comment.user)) {
      justSigned = await recordSignature(event.comment.user, number);
    }
    const signed = await evaluate(pr);
    if (justSigned && signed && isAuthor) {
      const url = `${SERVER}/${owner}/${repo}/blob/${defaultBranch}/CLA.md`;
      await gh(`/repos/${owner}/${repo}/issues/${number}/comments`, {
        method: "POST",
        body: JSON.stringify({ body: `Thanks @${pr.user.login}, your signature is recorded ([CLA](${url})). The \`CLA\` check is green.` }),
      });
    }
  }
}

main().catch((err) => {
  console.error(`cla: ${err.message}`);
  process.exit(1);
});
