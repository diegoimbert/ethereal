/**
 * WsTransport: the UI talking to a remote engine (`ether-server` or a desktop app) over a
 * WebSocket. Owned by the `remote-engine` node; stub until then (see
 * `crates/ether-protocol/src/remote.rs` and docs/ROADMAP.md).
 *
 * Protocol:
 * 1. Open `ws(s)://host:port/` (never put the token in the URL).
 * 2. Send one text frame: `ClientHello { protocol_version, token, client }`.
 * 3. Receive one text frame: `ServerHello` — `Welcome { server, session }` or
 *    `Rejected { reason, message }` (then the server closes: 4001 auth, 4002 version).
 * 4. Afterwards every text frame is one JSON `ClientMessage` (sent) / `ServerMessage`
 *    (received), with the usual ordering (patches before the reply); `connect()` then sends
 *    `Project::Get` and resolves with the project.
 * 5. Binary frames carry bulk bytes (`./binaryFrame.ts`): `[kind u8][header_len u32 LE]
 *    [header JSON][payload]`, kinds `Bytes` = 1 (upload chunks, export downloads) and
 *    `Peaks` = 2. Receivers accept both the binary and the plain JSON form.
 */
import type { Command, Event, MeterFrame, PlayheadFrame, Project, ReplyValue } from "@/generated";
import type { EngineTransport, SendOptions, Unsubscribe } from "../EngineTransport";

const NOT_IMPLEMENTED = "WsTransport: not implemented yet (remote-engine node)";

export interface WsTransportOptions {
  /** Shared secret of the server (`ClientHello::token`). */
  token?: string | null;
}

export class WsTransport implements EngineTransport {
  readonly kind = "remote" as const;
  readonly url: string;
  readonly options: WsTransportOptions;

  constructor(url: string, opts: WsTransportOptions = {}) {
    this.url = url;
    this.options = opts;
  }

  connect(): Promise<Project> {
    return Promise.reject(new Error(NOT_IMPLEMENTED));
  }

  send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    void command;
    void opts;
    return Promise.reject(new Error(NOT_IMPLEMENTED));
  }

  onEvent(listener: (event: Event) => void): Unsubscribe {
    void listener;
    throw new Error(NOT_IMPLEMENTED);
  }

  subscribePlayhead(listener: (frame: PlayheadFrame) => void): Unsubscribe {
    void listener;
    throw new Error(NOT_IMPLEMENTED);
  }

  subscribeMeters(listener: (frame: MeterFrame) => void): Unsubscribe {
    void listener;
    throw new Error(NOT_IMPLEMENTED);
  }

  dispose(): void {
    throw new Error(NOT_IMPLEMENTED);
  }
}
