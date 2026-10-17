// @vitest-environment node
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { isGroupId } from "@/features/keymap/groups";
import { badWords, lowerUiText } from "./uiCase";
import { fixCase, formatCaseFindings, scanCase } from "./uiCaseScan";

const repo = fileURLToPath(new URL("../../..", import.meta.url));

/** The UI's source files: ui/src (but generated types, the mock engine and tests), and the web host (the static-share viewer is in ui/src). */
function sources(): string[] {
  const out: string[] = [];
  const walk = (d: string) => {
    for (const n of readdirSync(d)) {
      const p = join(d, n);
      if (statSync(p).isDirectory()) {
        if (!/^(generated|mock|node_modules|e2e|tooling)$/.test(n)) walk(p);
      } else if (/\.tsx?$/.test(n) && !/\.(test|spec)\.tsx?$|\.d\.ts$/.test(n)) out.push(p);
    }
  };
  for (const d of ["ui/src", "apps/web/src"]) walk(join(repo, d));
  return out;
}

describe("UI text is lowercase (#524)", () => {
  it("every string the UI shows is lowercase, acronyms, units and names included", () => {
    const findings = sources().flatMap((f) => {
      const src = readFileSync(f, "utf8");
      const lines = src.split("\n");
      // Command palette and shortcut groups are English ids, shown through their messages (#667).
      return scanCase(relative(repo, f), src).filter((x) => {
        const id = /\bgroup: "([^"]+)"/.exec(lines[x.line - 1] ?? "")?.[1];
        return !(id !== undefined && isGroupId(id));
      });
    });
    expect(
      formatCaseFindings(findings),
      "lowercase these: app text has no capitalized words, acronyms and units included (eth-allow-case only for what is not UI text)",
    ).toBe("");
  });

  it("no stylesheet changes the case of text: the strings themselves are lowercase", () => {
    const css: string[] = [];
    const walk = (d: string) => {
      for (const n of readdirSync(d)) {
        const p = join(d, n);
        if (statSync(p).isDirectory()) {
          if (n !== "node_modules") walk(p);
        } else if (/\.(css|tsx?)$/.test(n) && !/\.test\.tsx?$/.test(n)) css.push(p);
      }
    };
    for (const d of ["ui/src", "apps/web/src"]) walk(join(repo, d));
    const found = css.filter((f) =>
      /text-transform:\s*(uppercase|lowercase|capitalize)|textTransform:\s*["'](uppercase|lowercase|capitalize)/.test(readFileSync(f, "utf8")),
    );
    expect(found.map((f) => relative(repo, f))).toEqual([]);
  });

  it('lowercases every word: acronyms, formats, units and names too (owner: "lowercase everything")', () => {
    expect(lowerUiText("Add MIDI Track")).toBe("add midi track");
    expect(lowerUiText("Open VST3 Plugin at 440 Hz, 1.2 kHz, -6 dB")).toBe("open vst3 plugin at 440 hz, 1.2 khz, -6 db");
    expect(lowerUiText("Import from Ableton Live or FL Studio on macOS")).toBe("import from ableton live or fl studio on macos");
    expect(lowerUiText("LFO, FM Synth, DC Filter, GR, PWM, RMS, Sidechain HPF, MPE, CLAP, AU, DAW, IR, OSC")).toBe(
      "lfo, fm synth, dc filter, gr, pwm, rms, sidechain hpf, mpe, clap, au, daw, ir, osc",
    );
    expect(lowerUiText("Tempo 120 BPM, 37% CPU, EQ")).toBe("tempo 120 bpm, 37% cpu, eq");
    expect(lowerUiText("A Track")).toBe("a track");
    expect(lowerUiText("Welcome to Ethereal")).toBe("welcome to ethereal");
    expect(lowerUiText("Line\\nBreak")).toBe("line\\nbreak");
    for (const w of ["MIDI", "dB", "Hz", "kHz", "VST3", "LFO", "OpenRouter", "DAWproject", "CCs", "VST3-only"]) expect(badWords(`x ${w} y`)).toEqual([w]);
    // a code identifier in a message isn't a word
    expect(badWords("WasmTransport disposed, SharedArrayBuffer")).toEqual([]);
  });

  it("keeps what isn't a word: key caps in a shortcut, note names, pan and grid notation", () => {
    expect(lowerUiText("Press ⌘K or Ctrl+Shift+Enter")).toBe("press ⌘K or Ctrl+Shift+Enter");
    expect(lowerUiText("Hold Shift")).toBe("hold shift");
    expect(lowerUiText("Transpose to C#4 in A minor")).toBe("transpose to C#4 in A minor");
    expect(badWords("25L, 100R, C, 1/8T")).toEqual([]);
  });

  it("flags the old uppercase terms in any literal, not only the ones read as UI text", () => {
    const src = [
      'const unit = { Decibels: "dB", Hertz: "Hz" };',
      "const s = `${v} kHz`;",
      'const ok = "db"; // lowercase',
      'const keep = "MIDI"; // eth-allow-case: the reason',
    ].join("\n");
    expect(scanCase("x.ts", src).map((f) => f.word)).toEqual(["dB", "Hz", "kHz"]);
  });

  it("finds UI text by syntax and leaves code alone", () => {
    const src = [
      'const a = <button title="Save Project" aria-label={busy ? "Saving…" : "Save"}>Open File</button>;',
      'const b = { label: "Delete Track", id: "Delete", type: "Audio" };',
      'notify("Error", `Couldn\'t reach ${host}. Check your connection.`);',
      'send(cmd("Transport", { type: "Play" }));',
      'const path = "M1.5 12V8.5a6.5 6.5 0 0 1 13 0";',
      'console.warn("Ethereal: Something failed");',
      'const c = { label: plural(n, "Freeze Track", "Freeze # Tracks") };',
      'text = "Connected";',
      'function Stack({ label = "Notifications" }) {}',
      'const edit = (label: string, action: string) => ({ label, action }); edit("Cut", "edit.cut");',
      'function formatStep() { return "1 Bar"; }',
      'const keep = "Not Shown Here"; // eth-allow-case: the reason',
    ].join("\n");
    expect(scanCase("x.tsx", src).map((f) => f.word)).toEqual([
      ...["Save", "Project", "Saving", "Save", "Open", "File", "Delete", "Track", "Couldn", "Check"],
      ...["Freeze", "Track", "Freeze", "Tracks", "Connected", "Notifications", "Cut", "Bar"],
    ]);
    expect(fixCase("x.tsx", src).split("\n").slice(0, 3)).toEqual([
      'const a = <button title="save project" aria-label={busy ? "saving…" : "save"}>open file</button>;',
      'const b = { label: "delete track", id: "Delete", type: "Audio" };',
      'notify("Error", `couldn\'t reach ${host}. check your connection.`);',
    ]);
  });
});
