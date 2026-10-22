import type { ReplyValue } from "@/generated";
import { cmd } from "./cmd";
import type { EngineTransport } from "./EngineTransport";

interface Lists {
  /** The `Project::List` the engine is answering. */
  running: Promise<ReplyValue> | null;
  /** The one sent once it has answered, shared by everyone who asked meanwhile. */
  queued: Promise<ReplyValue> | null;
}

const lists = new WeakMap<EngineTransport, Lists>();

/**
 * `Project::List` on `transport`, coalesced: at most one is in flight and one waits behind it.
 * Asking while one is in flight shares the next one (sent when the first answers), so the reply
 * still reflects the store as of the call. Launch asks from several places at once (the
 * provider, the recovery dialog, the project screen, reopen-last) and each request is a scan of
 * the projects folder on the engine's command thread; the engine also pushes
 * `Project::ListChanged` whenever the list changes.
 */
export function listProjects(transport: EngineTransport): Promise<ReplyValue> {
  let state = lists.get(transport);
  if (!state) {
    state = { running: null, queued: null };
    lists.set(transport, state);
  }
  const s = state;
  const start = (): Promise<ReplyValue> => {
    const reply = transport.send(cmd("Project", { type: "List" }));
    s.running = reply;
    const done = () => {
      if (s.running === reply) s.running = null;
    };
    reply.then(done, done);
    return reply;
  };
  if (!s.running) return start();
  s.queued ??= s.running
    .then(
      () => undefined,
      () => undefined,
    )
    .then(() => {
      s.queued = null;
      return start();
    });
  return s.queued;
}
