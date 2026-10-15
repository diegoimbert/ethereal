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
const BIG_OPTIONS = Array.from({ length: 5_000 }, (_, i) => ({ value: `p${i}`, label: `parameter ${i + 1}`, group: `page ${Math.floor(i / 100) + 1}` }));

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
          <span style={{ fontSize: `var(${tokenName("fs", k)})` }}>the quick brown fox — 120.00 bpm</span>
        </Row>
      ))}
      {Object.keys(fontWeight).map((k) => (
        <Row key={k} label={tokenName("fw", k)}>
          <span style={{ fontWeight: `var(${tokenName("fw", k)})` }}>weight {k}</span>
        </Row>
      ))}
      {Object.keys(font).map((k) => (
        <Row key={k} label={tokenName("font", k)}>
          <span style={{ fontFamily: `var(${tokenName("font", k)})` }}>1.1.1 · 00:00:00.000 · ethereal</span>
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
      <span className="eth-gallery__hint">hover a bar to see the duration.</span>
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
      <Row label="knob sm / md / lg / bipolar / disabled">
        {SIZES.map((s) => (
          <Knob key={s} size={s} value={knob} onChange={setKnob} label={s} valueText={`${Math.round(knob * 100)}%`} />
        ))}
        <Knob value={pan} onChange={setPan} bipolar label="pan" />
        <Knob value={0.3} disabled label="off" />
      </Row>
      <Row label="fader / meter">
        <Fader value={fader} onChange={setFader} label="volume" />
        <Fader value={0.4} disabled label="disabled" />
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
        <Row key={size} label={`button ${size}`}>
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
        <Row key={size} label={`fields ${size}`}>
          <TextInput size={size} value={text} onChange={(e) => setText(e.target.value)} aria-label={`text ${size}`} />
          <TextInput size={size} placeholder="placeholder" aria-label={`placeholder ${size}`} />
          <TextInput size={size} invalid defaultValue="Invalid" aria-label={`invalid ${size}`} />
          <Select
            size={size}
            aria-label={`select ${size}`}
            value={sel}
            onChange={setSel}
            options={[
              { value: "a", label: "option a", group: "first group" },
              { value: "b", label: "option b", group: "first group" },
              { value: "c", label: "disabled option", group: "first group", disabled: true },
              { value: "d", label: "option d", group: "second group" },
              { value: "e", label: "option e", group: "second group" },
            ]}
          />
          <Select
            size={size}
            aria-label={`placeholder select ${size}`}
            value=""
            placeholder="+ add…"
            onChange={() => undefined}
            options={[
              { value: "x", label: "action x" },
              { value: "y", label: "action y" },
            ]}
          />
          <NumberField size={size} aria-label={`tempo ${size}`} value={num} onChange={setNum} min={20} max={999} precision={2} unit="bpm" />
        </Row>
      ))}
      <Row label="long lists">
        <Select size="sm" aria-label="searchable select" value={bigSel} onChange={setBigSel} placeholder="+ parameter…" options={BIG_OPTIONS} />
        <VirtualList
          className="eth-gallery__vlist"
          aria-label="virtual list"
          count={BIG_OPTIONS.length}
          rowHeight={parseFloat(sizeTokens.rowHeight)}
          renderRow={(i) => <span className="eth-gallery__vrow">{BIG_OPTIONS[i]!.label}</span>}
        />
      </Row>
      <Row label="toggle">
        <Toggle checked={on} onChange={setOn} label="metronome" />
        <Toggle checked={!on} onChange={(v) => setOn(!v)} size="sm" label="small" />
        <Toggle checked={on} onChange={setOn} size="lg" label="large" />
        <Toggle checked disabled label="disabled" />
      </Row>
      {SIZES.map((size) => (
        <Row key={size} label={`tabs ${size}`}>
          <Tabs
            size={size}
            label={`tabs ${size}`}
            value={tab}
            onChange={setTab}
            items={[
              { id: "one", label: "devices" },
              { id: "two", label: "piano roll" },
              { id: "three", label: "mixer" },
            ]}
          />
        </Row>
      ))}
      <Row label="badge">
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
    <Row label="menu / popover / dialog / tooltip">
      <Menu
        aria-label="file"
        trigger={(p) => <Button {...p}>menu ▾</Button>}
        items={[
          { id: "new", label: "new set", shortcut: "Ctrl+N", onSelect: () => setLast("new set") },
          { id: "open", label: "open…", shortcut: "Ctrl+O", onSelect: () => setLast("Open") },
          { id: "sep", separator: true },
          { id: "disabled", label: "disabled", disabled: true, onSelect: () => undefined },
          { id: "delete", label: "delete", danger: true, onSelect: () => setLast("Delete") },
        ]}
      />
      <Popover trigger={(p) => <Button {...p}>popover</Button>} aria-label="popover demo">
        <div className="eth-gallery__stack">
          <span>popover content</span>
          <Toggle checked label="inside" />
        </div>
      </Popover>
      <Button tone="accent" onClick={() => setDialog(true)}>
        dialog
      </Button>
      <Tooltip content="tooltip text (delay: --tooltip-delay)">
        <Button>hover me</Button>
      </Tooltip>
      <span className="eth-gallery__value">last menu pick: {last}</span>
      <Dialog
        open={dialog}
        onClose={() => setDialog(false)}
        title="dialog title"
        footer={
          <>
            <Button tone="ghost" onClick={() => setDialog(false)}>
              cancel
            </Button>
            <Button tone="accent" onClick={() => setDialog(false)}>
              ok
            </Button>
          </>
        }
      >
        dialog body. escape or a backdrop click closes it.
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
        <h1 className="eth-gallery__h1">ethereal kit</h1>
        <Tabs label="theme" size="md" value={theme} onChange={setTheme} items={THEME_NAMES.map((t) => ({ id: t, label: t }))} />
        <a className="eth-gallery__link" href="?">
          open app
        </a>
      </header>
      <div className="eth-gallery__body">
        <Panel title="components" className="eth-gallery__panel">
          <Section title="buttons">
            <Buttons />
          </Section>
          <Section title="knobs, faders, meters">
            <Controls />
          </Section>
          <Section title="inputs, tabs, badges">
            <Inputs />
          </Section>
          <Section title="overlays">
            <Overlays />
          </Section>
          <Section title="panel">
            <Panel title="panel title" actions={<IconButton size="sm" tone="ghost" label="close" icon="×" />}>
              <div className="eth-gallery__pad">panel body</div>
            </Panel>
          </Section>
        </Panel>
        <Panel title="tokens" className="eth-gallery__panel">
          <Section title={`colors (${theme})`}>
            <ColorTokens theme={theme} />
          </Section>
          <Section title="track palette">
            <TrackPalette />
          </Section>
          <Section title="typography">
            <Typography />
          </Section>
          <Section title="spacing">
            <Spacing />
          </Section>
          <Section title="radii and shadows">
            <Shapes theme={theme} />
          </Section>
          <Section title="motion">
            <Motion />
          </Section>
          {["size", "border", "focus", "opacity", "z", "meter", "lh", "tracking"].map((g) => (
            <Section key={g} title={`--eth-${g}-*`}>
              <TokenTable entries={Object.entries(sharedGroups[g] ?? {}).map(([k, v]) => [tokenName(g, k), v])} />
            </Section>
          ))}
          {Object.entries(componentTokens).map(([c, toks]) => (
            <Section key={c} title={`component tokens: ${c}`}>
              <TokenTable entries={Object.entries(toks)} />
            </Section>
          ))}
        </Panel>
      </div>
    </div>
  );
}
