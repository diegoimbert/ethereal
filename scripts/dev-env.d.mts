export declare const PORT_OFFSETS: { dev: 0; preview: 1; playwright: 2; collab: 3 };
export declare function sanitizeInstance(name: string): string;
export declare function instanceId(env?: Record<string, string | undefined>): string;
export declare function basePort(env?: Record<string, string | undefined>): number;
export declare function port(
  kind?: keyof typeof PORT_OFFSETS,
  env?: Record<string, string | undefined>,
): number;
