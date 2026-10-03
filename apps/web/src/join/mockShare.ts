// Test hook: answer `Share::*` with the UI's `MockShare` instead of the engine, so the e2e
// can walk the join flow (landing → Continue → Ready → Join) before the engine side of
// sharing exists. Enabled only when a test sets `window.__etherMockShare = true` before the
// page loads (Playwright `addInitScript`); loaded on demand, never in normal use.
import type { Command, Event, ReplyValue } from "@/generated";
import { useProjectStore } from "@/state";
import { Emitter, type EngineTransport, type SendOptions } from "@/transport/EngineTransport";
import type { MockHost } from "@/transport/mock/roadmap/host";
import { MockShare } from "@/transport/mock/roadmap/share";

export function withMockShare<T extends EngineTransport>(inner: T): T {
  const events = new Emitter<Event>();
  const unsupported = (): never => {
    throw new Error("mock share host: document commands are not available");
  };
  const host: MockHost = {
    project: () => {
      const p = useProjectStore.getState().project;
      if (!p) throw new Error("no open project");
      return p;
    },
    emit: (e) => events.emit(e),
    newId: unsupported,
    applyDocument: unsupported,
    execute: unsupported,
  };
  const share = new MockShare(host);
  (window as { __etherShare?: MockShare }).__etherShare = share;
  return new Proxy(inner, {
    get(target, prop, receiver) {
      if (prop === "send") {
        return (command: Command, opts?: SendOptions): Promise<ReplyValue> => {
          if (command.domain !== "Share") return target.send(command, opts);
          try {
            return Promise.resolve(share.command(command.command));
          } catch (e) {
            return Promise.reject(e);
          }
        };
      }
      if (prop === "onEvent") {
        return (listener: (e: Event) => void) => {
          const offInner = target.onEvent(listener);
          const offMock = events.on(listener);
          return () => {
            offInner();
            offMock();
          };
        };
      }
      const v = Reflect.get(target, prop, receiver) as unknown;
      return typeof v === "function" ? (v as (...a: unknown[]) => unknown).bind(target) : v;
    },
  });
}
