/**
 * The few Cloudflare Workers runtime types the service uses (instead of a
 * `@cloudflare/workers-types` dependency: the surface is small and stable).
 */

interface DurableObjectId {
  toString(): string;
}

interface DurableObjectStub {
  fetch(request: Request): Promise<Response>;
}

interface DurableObjectNamespace {
  idFromName(name: string): DurableObjectId;
  get(id: DurableObjectId): DurableObjectStub;
}

interface DurableObjectStorage {
  get<T>(key: string): Promise<T | undefined>;
  put<T>(key: string, value: T): Promise<void>;
  delete(key: string): Promise<boolean>;
  deleteAll(): Promise<void>;
  setAlarm(scheduledTime: number): Promise<void>;
  deleteAlarm(): Promise<void>;
}

interface CfWebSocket {
  send(message: string): void;
  close(code?: number, reason?: string): void;
  serializeAttachment(value: unknown): void;
  deserializeAttachment(): unknown;
}

interface DurableObjectState {
  readonly storage: DurableObjectStorage;
  acceptWebSocket(ws: CfWebSocket, tags?: string[]): void;
  getWebSockets(tag?: string): CfWebSocket[];
}

declare const WebSocketPair: { new (): { 0: CfWebSocket; 1: CfWebSocket } };

interface CfResponseInit extends ResponseInit {
  webSocket?: CfWebSocket;
}

interface Env {
  ROOMS: DurableObjectNamespace;
  /** `IpBudget` objects (per-IP claims and bad doors); absent = no per-IP limits. */
  BUDGETS?: DurableObjectNamespace;
  ALLOWED_ORIGINS: string;
  STUN_URLS: string;
  ROOM_TTL_DAYS: string;
  TURN_KEY_ID?: string;
  TURN_KEY_API_TOKEN?: string;
}
