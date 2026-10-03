/**
 * Kit gallery: every kit component, variant and design token on one page, with a theme
 * switch. Open `just dev-ui` with `?kit` (e.g. http://localhost:<port>/?kit). For iterating
 * on ui/src/theme/tokens.ts visually; see docs/DESIGN-SYSTEM.md.
 */
import { useState, type ReactNode } from "react";
import {
  Badge,
  Button,
  Dialog,
  Fader,
  IconButton,
  Knob,
  Menu,
  Meter,
  NumberField,
  Panel,
  Popover,
  Select,
  SIZES,
  STATUS_TONES,
  Tabs,
  TextInput,
  Toggle,
  TONES,
  VirtualList,
  Tooltip,
  useTheme,
  type ThemeName,
} from "@/kit";
import {
  componentTokens,
  duration,
  ease,
  font,
  fontSize,
  fontWeight,
  radius,
  size as sizeTokens,
  sharedGroups,
  space,
  THEME_NAMES,
  themes,
  tokenName,
  TRACK_COLORS,
} from "@/theme";
import "./kit-gallery.css";

/** A long option list (the searchable, windowed Select and VirtualList demos). */
const BIG_OPTIONS = Array.from({ length: 5_000 }, (_, i) => ({ value: `p${i}`, label: `Parameter ${i + 1}`, group: `Page ${Math.floor(i / 100) + 1}` }));

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="eth-gallery__section" aria-label={title}>
      <h2 className="eth-gallery__h2">{title}</h2>
      {children}
    </section>
  );
}

function Row({ label, children }: { label?: string; children: ReactNode }) {
  return (
    <div className="eth-gallery__row">
      {label && <span className="eth-gallery__label">{label}</span>}
      {children}
    </div>
  );
}

function TokenName({ children }: { children: string }) {
  return <code className="eth-gallery__code">{children}</code>;
}

// ---- Tokens ---------------------------------------------------------------------------

function ColorTokens({ theme }: { theme: ThemeName }) {
  const t = themes[theme];
  return (
    <div className="eth-gallery__grid">
      {Object.entries(t.color).map(([key, value]) => {
        const name = tokenName("color", key);
        return (
          <div key={key} className="eth-gallery__swatch">
            <span className="eth-gallery__chip" style={{ background: `var(${name})` }} />
            <TokenName>{name}</TokenName>
            <span className="eth-gallery__value">{value}</span>
          </div>
        );
      })}
    </div>
  );
}

function TrackPalette() {
  return (
    <div className="eth-gallery__row">
      {TRACK_COLORS.map((c, i) => (
        <span key={c} className="eth-gallery__chip" title={`--eth-track-${i}: ${c}`} style={{ background: `var(--eth-track-${i})` }} />
      ))}
    </div>
  );
}

function Typography() {
  return (
    <div className="eth-gallery__stack">
      {Object.keys(fontSize).map((k) => (
        <Row key={k} label={tokenName("fs", k)}>
          <span style={{ fontSize: `var(${tokenName("fs", k)})` }}>The quick brown fox — 120.00 BPM</span>
        </Row>
      ))}
      {Object.keys(fontWeight).map((k) => (
        <Row key={k} label={tokenName("fw", k)}>
          <span style={{ fontWeight: `var(${tokenName("fw", k)})` }}>Weight {k}</span>
        </Row>
      ))}
      {Object.keys(font).map((k) => (
        <Row key={k} label={tokenName("font", k)}>
          <span style={{ fontFamily: `var(${tokenName("font", k)})` }}>1.1.1 · 00:00:00.000 · Ethereal</span>
        </Row>
      ))}
    </div>
  );
}

function Spacing() {
  return (
    <div className="eth-gallery__stack">
      {Object.entries(space).map(([k, v]) => (
        <Row key={k} label={`${tokenName("space", k)} (${v})`}>
          <span className="eth-gallery__bar" style={{ width: `var(${tokenName("space", k)})` }} />
        </Row>
      ))}
    </div>
  );
}

function Shapes({ theme }: { theme: ThemeName }) {
  return (
    <>
      <Row label="radius">
        {Object.entries(radius).map(([k, v]) => (
          <span key={k} className="eth-gallery__shape" title={`${tokenName("radius", k)}: ${v}`} style={{ borderRadius: `var(${tokenName("radius", k)})` }}>
            {k}
          </span>
        ))}
      </Row>
      <Row label="shadow">
        {Object.entries(themes[theme].shadow).map(([k, v]) => (
          <span key={k} className="eth-gallery__shape" title={`${tokenName("shadow", k)}: ${v}`} style={{ boxShadow: `var(${tokenName("shadow", k)})` }}>
            {k}
          </span>
        ))}
      </Row>
    </>
  );
}

function Motion() {
  return (
    <div className="eth-gallery__stack">
      {Object.entries(duration).map(([dk, dv]) => (
        <Row key={dk} label={`${tokenName("duration", dk)} (${dv})`}>
          <span className="eth-gallery__motion" style={{ transitionDuration: `var(${tokenName("duration", dk)})` }} />
        </Row>
      ))}
      {Object.entries(ease).map(([ek, ev]) => (
        <Row key={ek} label={tokenName("ease", ek)}>
          <span className="eth-gallery__value">{ev}</span>
        </Row>
      ))}
      <span className="eth-gallery__hint">Hover a bar to see the duration.</span>
    </div>
  );
}

function TokenTable({ entries }: { entries: Array<[string, string]> }) {
  return (
    <table className="eth-gallery__table">
      <tbody>
        {entries.map(([k, v]) => (
          <tr key={k}>
            <td>
              <TokenName>{k}</TokenName>
            </td>
            <td className="eth-gallery__value">{v}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ---- Components -----------------------------------------------------------------------

function Controls() {
  const [knob, setKnob] = useState(0.6);
  const [pan, setPan] = useState(0.35);
  const [fader, setFader] = useState(0.7);
  return (
    <>
      <Row label="Knob sm / md / lg / bipolar / disabled">
        {SIZES.map((s) => (
          <Knob key={s} size={s} value={knob} onChange={setKnob} label={s} valueText={`${Math.round(knob * 100)}%`} />
        ))}
        <Knob value={pan} onChange={setPan} bipolar label="Pan" />
        <Knob value={0.3} disabled label="Off" />
      </Row>
      <Row label="Fader / Meter">
        <Fader value={fader} onChange={setFader} label="Volume" />
        <Fader value={0.4} disabled label="Disabled" />
        <Meter levels={[fader * 1.2, fader * 0.9]} />
        <Meter levels={[0.1, 0.5, 1.3]} />
      </Row>
    </>
  );
}

function Buttons() {
  return (
    <>
      {SIZES.map((size) => (
        <Row key={size} label={`Button ${size}`}>
          {TONES.map((tone) => (
            <Button key={tone} size={size} tone={tone}>
              {tone}
            </Button>
          ))}
          <Button size={size} active>
            active
          </Button>
          <Button size={size} disabled>
            disabled
          </Button>
          {TONES.map((tone) => (
            <IconButton key={tone} size={size} tone={tone} label={`${tone} icon`} icon="●" />
          ))}
        </Row>
      ))}
    </>
  );
}

function Inputs() {
  const [text, setText] = useState("Audio 1");
  const [sel, setSel] = useState<"a" | "b" | "c" | "d" | "e">("a");
  const [num, setNum] = useState(120);
  const [on, setOn] = useState(true);
  const [tab, setTab] = useState<"one" | "two" | "three">("one");
  const [bigSel, setBigSel] = useState("");
  return (
    <>
      {SIZES.map((size) => (
        <Row key={size} label={`Fields ${size}`}>
          <TextInput size={size} value={text} onChange={(e) => setText(e.target.value)} aria-label={`Text ${size}`} />
          <TextInput size={size} placeholder="Placeholder" aria-label={`Placeholder ${size}`} />
          <TextInput size={size} invalid defaultValue="Invalid" aria-label={`Invalid ${size}`} />
          <Select
            size={size}
            aria-label={`Select ${size}`}
            value={sel}
            onChange={setSel}
            options={[
              { value: "a", label: "Option A", group: "First group" },
              { value: "b", label: "Option B", group: "First group" },
              { value: "c", label: "Disabled option", group: "First group", disabled: true },
              { value: "d", label: "Option D", group: "Second group" },
              { value: "e", label: "Option E", group: "Second group" },
            ]}
          />
          <Select
            size={size}
            aria-label={`Placeholder select ${size}`}
            value=""
            placeholder="+ Add…"
            onChange={() => undefined}
            options={[
              { value: "x", label: "Action X" },
              { value: "y", label: "Action Y" },
            ]}
          />
          <NumberField size={size} aria-label={`Tempo ${size}`} value={num} onChange={setNum} min={20} max={999} precision={2} unit="BPM" />
        </Row>
      ))}
      <Row label="Long lists">
        <Select
          size="sm"
          aria-label="Searchable select"
          value={bigSel}
          onChange={setBigSel}
          placeholder="+ Parameter…"
          options={BIG_OPTIONS}
        />
        <VirtualList
          className="eth-gallery__vlist"
          aria-label="Virtual list"
          count={BIG_OPTIONS.length}
          rowHeight={parseFloat(sizeTokens.rowHeight)}
          renderRow={(i) => <span className="eth-gallery__vrow">{BIG_OPTIONS[i]!.label}</span>}
        />
      </Row>
      <Row label="Toggle">
        <Toggle checked={on} onChange={setOn} label="Metronome" />
        <Toggle checked={!on} onChange={(v) => setOn(!v)} size="sm" label="Small" />
        <Toggle checked={on} onChange={setOn} size="lg" label="Large" />
        <Toggle checked disabled label="Disabled" />
      </Row>
      {SIZES.map((size) => (
        <Row key={size} label={`Tabs ${size}`}>
          <Tabs
            size={size}
            label={`Tabs ${size}`}
            value={tab}
            onChange={setTab}
            items={[
              { id: "one", label: "Devices" },
              { id: "two", label: "Piano Roll" },
              { id: "three", label: "Mixer" },
            ]}
          />
        </Row>
      ))}
      <Row label="Badge">
        {STATUS_TONES.map((t) => (
          <Badge key={t} tone={t}>
            {t}
          </Badge>
        ))}
      </Row>
    </>
  );
}

function Overlays() {
  const [dialog, setDialog] = useState(false);
  const [last, setLast] = useState("—");
  return (
    <Row label="Menu / Popover / Dialog / Tooltip">
      <Menu
        aria-label="File"
        trigger={(p) => <Button {...p}>Menu ▾</Button>}
        items={[
          { id: "new", label: "New set", shortcut: "Ctrl+N", onSelect: () => setLast("New set") },
          { id: "open", label: "Open…", shortcut: "Ctrl+O", onSelect: () => setLast("Open") },
          { id: "sep", separator: true },
          { id: "disabled", label: "Disabled", disabled: true, onSelect: () => undefined },
          { id: "delete", label: "Delete", danger: true, onSelect: () => setLast("Delete") },
        ]}
      />
      <Popover trigger={(p) => <Button {...p}>Popover</Button>} aria-label="Popover demo">
        <div className="eth-gallery__stack">
          <span>Popover content</span>
          <Toggle checked label="Inside" />
        </div>
      </Popover>
      <Button tone="accent" onClick={() => setDialog(true)}>
        Dialog
      </Button>
      <Tooltip content="Tooltip text (delay: --tooltip-delay)">
        <Button>Hover me</Button>
      </Tooltip>
      <span className="eth-gallery__value">last menu pick: {last}</span>
      <Dialog
        open={dialog}
        onClose={() => setDialog(false)}
        title="Dialog title"
        footer={
          <>
            <Button tone="ghost" onClick={() => setDialog(false)}>
              Cancel
            </Button>
            <Button tone="accent" onClick={() => setDialog(false)}>
              OK
            </Button>
          </>
        }
      >
        Dialog body. Escape or a backdrop click closes it.
      </Dialog>
    </Row>
  );
}

// ---- Page -----------------------------------------------------------------------------

export function KitGallery() {
  const [theme, setTheme] = useTheme();
  return (
    <div className="eth-gallery">
      <header className="eth-gallery__top">
        <h1 className="eth-gallery__h1">Ethereal kit</h1>
        <Tabs
          label="Theme"
          size="md"
          value={theme}
          onChange={setTheme}
          items={THEME_NAMES.map((t) => ({ id: t, label: t }))}
        />
        <a className="eth-gallery__link" href="?">
          Open app
        </a>
      </header>
      <div className="eth-gallery__body">
        <Panel title="Components" className="eth-gallery__panel">
          <Section title="Buttons">
            <Buttons />
          </Section>
          <Section title="Knobs, faders, meters">
            <Controls />
          </Section>
          <Section title="Inputs, tabs, badges">
            <Inputs />
          </Section>
          <Section title="Overlays">
            <Overlays />
          </Section>
          <Section title="Panel">
            <Panel title="Panel title" actions={<IconButton size="sm" tone="ghost" label="Close" icon="×" />}>
              <div className="eth-gallery__pad">Panel body</div>
            </Panel>
          </Section>
        </Panel>
        <Panel title="Tokens" className="eth-gallery__panel">
          <Section title={`Colors (${theme})`}>
            <ColorTokens theme={theme} />
          </Section>
          <Section title="Track palette">
            <TrackPalette />
          </Section>
          <Section title="Typography">
            <Typography />
          </Section>
          <Section title="Spacing">
            <Spacing />
          </Section>
          <Section title="Radii and shadows">
            <Shapes theme={theme} />
          </Section>
          <Section title="Motion">
            <Motion />
          </Section>
          {["size", "border", "focus", "opacity", "z", "meter", "lh", "tracking"].map((g) => (
            <Section key={g} title={`--eth-${g}-*`}>
              <TokenTable entries={Object.entries(sharedGroups[g] ?? {}).map(([k, v]) => [tokenName(g, k), v])} />
            </Section>
          ))}
          {Object.entries(componentTokens).map(([c, toks]) => (
            <Section key={c} title={`Component tokens: ${c}`}>
              <TokenTable entries={Object.entries(toks)} />
            </Section>
          ))}
        </Panel>
      </div>
    </div>
  );
}
