// Which jobs the devbox runner pool may run (docs/CI-RUNNERS.md). The job-started hook calls this
// before the first step of every job, with the event GitHub sent; a refusal fails the job before
// any of its code runs. The workflows already send untrusted jobs to GitHub-hosted runners: this
// is the backstop for a workflow edited to target the pool (a fork's pull request runs the fork's
// own workflow files).
//
// Trusted: pushes, manual and scheduled runs (they need write access or run the default branch),
// and pull requests whose head branch lives in this repository and that no bot opened. Everything
// else is refused: pull requests from forks, Dependabot (its pull requests bump third-party code),
// and the events that run in a pull request's context with the base's privileges.

/**
 * @param {{ eventName: string, repository: string, actor: string, event: any }} job
 * @returns {{ ok: boolean, why: string }}
 */
export function verdict({ eventName, repository, actor, event }) {
  if (!repository) return { ok: false, why: "no GITHUB_REPOSITORY" };
  const bot = (login) => typeof login === "string" && login.endsWith("[bot]");
  if (bot(actor)) return { ok: false, why: `actor ${actor} is a bot` };
  switch (eventName) {
    case "push":
    case "workflow_dispatch":
    case "schedule":
    case "merge_group":
      return { ok: true, why: eventName };
    case "pull_request": {
      const pr = event?.pull_request;
      const head = pr?.head?.repo?.full_name;
      if (!pr || !head) return { ok: false, why: "pull_request without a head repository" };
      if (head.toLowerCase() !== repository.toLowerCase()) return { ok: false, why: `pull request from ${head}` };
      if (bot(pr.user?.login)) return { ok: false, why: `pull request opened by ${pr.user.login}` };
      return { ok: true, why: "pull request from a branch of this repository" };
    }
    default:
      return { ok: false, why: `event ${eventName} is not allowed on the pool` };
  }
}
