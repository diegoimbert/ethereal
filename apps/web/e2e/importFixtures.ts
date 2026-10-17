// Project files of other DAWs for the import e2e (import-dawproject), built from the
// importer's test fixtures (crates/ether-import/tests/fixtures): no bundled third-party samples.

/** CRC-32 (ZIP). */
function crc32(b: Uint8Array): number {
  let c = ~0;
  for (const x of b) {
    c ^= x;
    for (let k = 0; k < 8; k++) c = c & 1 ? (c >>> 1) ^ 0xedb88320 : c >>> 1;
  }
  return ~c >>> 0;
}

/** A ZIP archive with stored (uncompressed) entries. */
export function zip(entries: Array<[string, Uint8Array]>): Buffer {
  const parts: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  for (const [name, data] of entries) {
    const n = Buffer.from(name, "utf8");
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(n.length, 26);
    parts.push(local, n, Buffer.from(data));
    const c = Buffer.alloc(46);
    c.writeUInt32LE(0x02014b50, 0);
    c.writeUInt16LE(20, 4);
    c.writeUInt16LE(20, 6);
    c.writeUInt16LE(0x0800, 8);
    c.writeUInt32LE(crc, 16);
    c.writeUInt32LE(data.length, 20);
    c.writeUInt32LE(data.length, 24);
    c.writeUInt16LE(n.length, 28);
    c.writeUInt32LE(offset, 42);
    central.push(c, n);
    offset += 30 + n.length + data.length;
  }
  const dir = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(dir.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, dir, end]);
}

/** A mono 16-bit WAV: `seconds` of a 220 Hz sine at `rate`. */
export function sineWav(seconds: number, rate = 44100): Buffer {
  const frames = Math.round(rate * seconds);
  const b = Buffer.alloc(44 + frames * 2);
  b.write("RIFF", 0);
  b.writeUInt32LE(36 + frames * 2, 4);
  b.write("WAVEfmt ", 8);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24);
  b.writeUInt32LE(rate * 2, 28);
  b.writeUInt16LE(2, 32);
  b.writeUInt16LE(16, 34);
  b.write("data", 36);
  b.writeUInt32LE(frames * 2, 40);
  for (let i = 0; i < frames; i++) b.writeInt16LE(Math.round(8000 * Math.sin((2 * Math.PI * 220 * i) / rate)), 44 + i * 2);
  return b;
}

/** A small REAPER project: a folder with a MIDI track (CLAP synth), an audio track, a return. */
export const RPP = `<REAPER_PROJECT 0.1 "7.27/linux-x86_64" 1727000000
  TEMPO 96 4 4
  MARKER 1 0 "Intro" 0 0
  MARKER 2 5 "Drop" 0 0
  <TRACK
    NAME Band
    ISBUS 1 1
  >
  <TRACK
    NAME Keys
    ISBUS 2 -1
    <FXCHAIN
      BYPASS 0 0 0
      <CLAP "CLAPi: Surge XT (Surge Synth Team)" org.surge-synth-team.surge-xt ""
        <STATE
          c3VyZ2Utc3RhdGU=
        >
      >
    >
    <ITEM
      POSITION 0
      LENGTH 2.5
      NAME Chords
      <SOURCE MIDI
        HASDATA 1 960 QN
        E 0 90 3c 64
        E 960 80 3c 40
        E 0 90 40 64
        E 960 80 40 40
      >
    >
  >
  <TRACK
    NAME Guitar
    VOLPAN 0.5 0.2 -1 -1 1
    <ITEM
      POSITION 2.5
      LENGTH 2
      FADEIN 1 0.1 0 1 0 0 0
      <SOURCE WAVE
        FILE "Audio/guitar.wav"
      >
    >
  >
  <TRACK
    NAME Verb
    AUXRECV 2 0 0.5 0 0 0 0 0 0 -1:U 0 -1 ''
  >
>
`;

/** The importer's fixtures (repo-relative): only in a build that has the importer. */
export const IMPORT_FIXTURES_DIR = "crates/ether-import/tests/fixtures/dawproject";

export function importFixtures(read: (repoPath: string) => Buffer) {
  const dir = IMPORT_FIXTURES_DIR;
  const canon = zip([
    ["project.xml", read(`${dir}/canon-project.xml`)],
    ["metadata.xml", read(`${dir}/canon-metadata.xml`)],
    ["audio/canon_guitar_150bpm.wav", sineWav(1)],
  ]);
  const full = zip([
    ["project.xml", read(`${dir}/full-project.xml`)],
    ["metadata.xml", Buffer.from("<MetaData><Title>Night Drive</Title></MetaData>")],
    ["audio/loop.wav", sineWav(2, 48000)],
    ["plugins/sat.clap-preset", Buffer.from("raw-clap-state")],
  ]);
  return { canon, full, rpp: Buffer.from(RPP, "utf8") };
}
