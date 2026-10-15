import type { Command } from "@/generated";

export type CommandDomain = Command["domain"];

/** The payload type of a domain (e.g. `DomainCommand<"Mixer">` = `MixerCommand`). */
export type DomainCommand<D extends CommandDomain> = Extract<Command, { domain: D }>["command"];

/**
 * Build a `Command` with type inference on the payload:
 *
 * ```ts
 * transport.send(cmd("Mixer", { type: "SetVolume", track, volume: -6 }));
 * ```
 */
export function cmd<D extends CommandDomain>(domain: D, command: DomainCommand<D>): Command {
  return { domain, command } as Command;
}
