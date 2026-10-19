#!/usr/bin/env node
// ACTIONS_RUNNER_HOOK_JOB_STARTED of the devbox pool's runners: refuses an untrusted job (trust.mjs)
// before any of its steps run. A non-zero exit fails the job.
import { readFileSync } from "node:fs";
import { verdict } from "./trust.mjs";

const env = process.env;
let event = {};
try {
  event = JSON.parse(readFileSync(env.GITHUB_EVENT_PATH, "utf8"));
} catch (e) {
  console.error(`::error::devbox pool: cannot read the event (${e.message}); refusing the job`);
  process.exit(1);
}
const v = verdict({ eventName: env.GITHUB_EVENT_NAME, repository: env.GITHUB_REPOSITORY, actor: env.GITHUB_ACTOR, event });
if (!v.ok) {
  console.error(`::error::devbox pool refuses this job: ${v.why}. Untrusted jobs run on GitHub-hosted runners (docs/CI-RUNNERS.md).`);
  process.exit(1);
}
console.log(`devbox pool: ${v.why}`);
