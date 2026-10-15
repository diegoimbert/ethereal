// The fetch every provider adapter uses. On the web it's the browser's fetch (CORS applies,
// see `networkError`). On the desktop it's tauri-plugin-http: the request is made in Rust,
// so providers that refuse web origins work. The plugin only fetches allowed URLs
// (apps/desktop/src-tauri/src/ai_http.rs): the preset providers, plus the origin of the
// base URL being called, which `ai_http_allow` validates and adds first.
import { invoke } from "@tauri-apps/api/core";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { isTauri } from "@/transport";

/** The desktop side (replaced in tests). */
export const desktopHttp = {
  invoke: invoke as (cmd: string, args?: Record<string, unknown>) => Promise<unknown>,
  fetch: tauriFetch as typeof fetch,
};

/** A desktop request the HTTP plugin refused or couldn't send (no response). */
export class DesktopFetchError extends TypeError {
  /** The URL isn't in the AI chat's scope (an unsupported base URL). */
  readonly notAllowed: boolean;
  constructor(message: string) {
    super(message);
    this.name = "DesktopFetchError";
    this.notAllowed = /not allowed|base URL/i.test(message);
  }
}

const abortError = () => Object.assign(new Error("aborted"), { name: "AbortError" });

/** Origins already allowed this session (one `ai_http_allow` each). */
const allowed = new Map<string, Promise<void>>();

function allowOrigin(origin: string): Promise<void> {
  let p = allowed.get(origin);
  if (!p) {
    p = desktopHttp.invoke("ai_http_allow", { baseUrl: origin }).then(() => undefined);
    // A refusal isn't remembered: the next request asks again (and fails the same way).
    p.catch(() => allowed.delete(origin));
    allowed.set(origin, p);
  }
  return p;
}

/** Forget the allowed origins (tests). */
export function resetDesktopFetch(): void {
  allowed.clear();
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * The plugin reports a cancelled request or body as "Request cancelled"; the adapters expect
 * an `AbortError` (a Stop isn't an error), so the body is passed through with that mapping.
 */
function abortAware(res: Response, signal: AbortSignal | null | undefined): Response {
  if (!res.body || !signal) return res;
  const reader = res.body.getReader();
  const body = new ReadableStream<Uint8Array>({
    async pull(controller) {
      try {
        const { done, value } = await reader.read();
        if (done) controller.close();
        else controller.enqueue(value);
      } catch (e) {
        controller.error(signal.aborted ? abortError() : e);
      }
    },
    cancel(reason) {
      return reader.cancel(reason);
    },
  });
  return new Response(body, { status: res.status, statusText: res.statusText, headers: res.headers });
}

async function desktopFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = input instanceof Request ? input.url : String(input);
  const signal = init?.signal ?? (input instanceof Request ? input.signal : undefined);
  try {
    await allowOrigin(new URL(url).origin);
    const headers = new Headers(init?.headers ?? (input instanceof Request ? input.headers : undefined));
    // No `Origin`: like any other API client. Local servers (Ollama) refuse unknown origins.
    headers.set("origin", "");
    const res = await desktopHttp.fetch(input, { ...init, headers });
    return abortAware(res, signal);
  } catch (e) {
    if (signal?.aborted) throw abortError();
    // Never the request's headers (the key): the plugin's message names the URL at most.
    throw new DesktopFetchError(message(e));
  }
}

/** `fetch` for provider requests: the desktop's Rust-side fetch, or the browser's. */
export const providerFetch: typeof fetch = (input, init) => (isTauri() ? desktopFetch(input, init) : globalThis.fetch(input, init));
