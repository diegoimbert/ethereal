import { test } from "node:test";
import assert from "node:assert/strict";
import { verdict } from "./runner/trust.mjs";

const repository = "diegoimbert/ethereal";
const pr = (head, user = "diegoimbert") => ({ pull_request: { head: { repo: { full_name: head } }, user: { login: user } } });

test("pushes, manual and scheduled runs are trusted", () => {
  for (const eventName of ["push", "workflow_dispatch", "schedule"]) {
    assert.equal(verdict({ eventName, repository, actor: "diegoimbert", event: {} }).ok, true, eventName);
  }
});

test("a pull request from a branch of this repository is trusted", () => {
  assert.equal(verdict({ eventName: "pull_request", repository, actor: "diegoimbert", event: pr("diegoimbert/ethereal") }).ok, true);
  assert.equal(verdict({ eventName: "pull_request", repository, actor: "diegoimbert", event: pr("DiegoImbert/Ethereal") }).ok, true);
});

test("a pull request from a fork is refused", () => {
  const v = verdict({ eventName: "pull_request", repository, actor: "someone", event: pr("someone/ethereal", "someone") });
  assert.equal(v.ok, false);
  assert.match(v.why, /someone\/ethereal/);
});

test("Dependabot and other bots are refused", () => {
  assert.equal(verdict({ eventName: "pull_request", repository, actor: "dependabot[bot]", event: pr(repository, "dependabot[bot]") }).ok, false);
  // A maintainer re-running a bot's pull request: still the bot's code.
  assert.equal(verdict({ eventName: "pull_request", repository, actor: "diegoimbert", event: pr(repository, "dependabot[bot]") }).ok, false);
  assert.equal(verdict({ eventName: "push", repository, actor: "github-actions[bot]", event: {} }).ok, false);
});

test("pull_request_target, issue_comment and unknown events are refused", () => {
  for (const eventName of ["pull_request_target", "issue_comment", "workflow_run", "release", undefined]) {
    assert.equal(verdict({ eventName, repository, actor: "diegoimbert", event: pr(repository) }).ok, false, String(eventName));
  }
});

test("a malformed pull_request event is refused", () => {
  assert.equal(verdict({ eventName: "pull_request", repository, actor: "diegoimbert", event: {} }).ok, false);
  assert.equal(verdict({ eventName: "push", repository: "", actor: "diegoimbert", event: {} }).ok, false);
});
