/**
 * Routing controls of `groups-buses` for the mixer strips: track-to-track input (source +
 * tap point) and VCA assignment. Kit components and tokens only.
 */

import { useShallow } from "zustand/react/shallow";
import type { InputTap, Track, TrackInput } from "@/generated";
import { Select } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, useTransport } from "@/transport";
import { assignVca, sendGroupsEdit } from "./actions";
import { assignedTo, INPUT_TAPS, inputSources, trackInput, vcaTargets } from "./model";

/** `Select` value of the input source: `keep` (a hardware/MIDI input), `none` or `track:<id>`. */
function sourceValue(input: TrackInput): string {
  if (input.type === "Track") return `track:${input.track}`;
  if (input.type === "None") return "none";
  return "keep";
}

/**
 * "In" picker: the track's hardware input (kept as is), no input, or another track's
 * signal (resampling / bus-to-track), plus the tap point when a track is chosen. Choosing
 * a track also turns monitoring to In, so it is heard (Ableton's "Audio From" does the same
 * when armed).
 */
export function TrackInputSelect({ track, className }: { track: Track; className?: string }) {
  const transport = useTransport();
  const sources = useProjectStore(useShallow((s) => (s.project ? inputSources(s.project.tracks, track) : [])));
  const input = track.input;
  const setInput = (next: TrackInput, monitorIn: boolean) => {
    const commands = [cmd("Recording", { type: "SetInput", track: track.id, input: next })];
    if (monitorIn && track.monitor !== "In") {
      commands.push(cmd("Recording", { type: "SetMonitor", track: track.id, monitor: "In" }));
    }
    void sendGroupsEdit(
      transport,
      commands.length === 1 ? commands[0]! : cmd("Edit", { type: "Batch", label: "Set Input", commands }),
    );
  };
  const hardware = input.type === "Audio" || input.type === "Midi";
  const options = [
    ...(hardware ? [{ value: "keep", label: input.type === "Audio" ? `In ${input.first + 1}${input.count > 1 ? `/${input.first + 2}` : ""}` : "MIDI" }] : []),
    { value: "none", label: "No input" },
    ...sources.map((s) => ({ value: `track:${s.id}`, label: `From ${s.name}` })),
  ];
  const current = sourceValue(input);
  if (!options.some((o) => o.value === current)) options.push({ value: current, label: "From (missing)" });
  return (
    <div className={className} data-testid="track-input">
      <Select
        size="sm"
        aria-label={`${track.name} input`}
        value={current}
        onChange={(v) => {
          if (v === "keep" || v === current) return;
          if (v === "none") setInput({ type: "None" }, false);
          else setInput(trackInput(v.slice("track:".length), input.type === "Track" ? input.tap : "PostFader"), true);
        }}
        options={options}
      />
      {input.type === "Track" && (
        <Select<InputTap>
          size="sm"
          aria-label={`${track.name} input tap`}
          value={input.tap}
          onChange={(tap) => setInput({ ...input, tap }, false)}
          options={INPUT_TAPS.map((t) => ({ value: t.tap, label: t.label }))}
        />
      )}
    </div>
  );
}

/** VCA assignment picker (hidden when the project has no VCA to assign to). */
export function VcaSelect({ track, className }: { track: Track; className?: string }) {
  const transport = useTransport();
  const targets = useProjectStore(useShallow((s) => (s.project ? vcaTargets(s.project.tracks, track) : [])));
  if (targets.length === 0 && !track.vca) return null;
  return (
    <Select
      size="sm"
      className={className}
      aria-label={`${track.name} VCA`}
      value={track.vca ?? "none"}
      onChange={(v) => void assignVca(transport, [track.id], v === "none" ? null : v)}
      options={[{ value: "none", label: "No VCA" }, ...targets.map((v) => ({ value: v.id, label: `VCA ${v.name}` }))]}
    />
  );
}

/** "Controls N tracks" line of a VCA strip / lane. */
export function VcaSummary({ vca, className }: { vca: Track; className?: string }) {
  const names = useProjectStore(useShallow((s) => (s.project ? assignedTo(s.project.tracks, vca.id).map((t) => t.name) : [])));
  const text = names.length === 0 ? "No tracks assigned" : names.join(", ");
  return (
    <span className={className} title={text} data-testid="vca-summary">
      {text}
    </span>
  );
}
