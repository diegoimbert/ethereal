// Small drawings for the record tour's bubbles (themed through tutorials.css).
import { useT, type MessageKey } from "@/i18n";

/** Microphone and guitar → audio interface → computer → headphones and speakers. */
export function InterfaceDiagram() {
  const t = useT();
  return (
    <svg viewBox="0 0 320 110" role="img" aria-label={t("tutorials.diagram.interface_label")}>
      <rect className="eth-tour-diagram__box" x="6" y="14" width="62" height="28" rx="6" />
      <text className="eth-tour-diagram__label" x="37" y="32">
        {t("tutorials.diagram.microphone")}
      </text>
      <rect className="eth-tour-diagram__box" x="6" y="62" width="62" height="28" rx="6" />
      <text className="eth-tour-diagram__label" x="37" y="80">
        {t("tutorials.diagram.guitar")}
      </text>
      <path className="eth-tour-diagram__wire" d="M68 28 C 90 28, 90 48, 112 48 M68 76 C 90 76, 90 60, 112 60" />
      <rect className="eth-tour-diagram__accent" x="112" y="30" width="86" height="48" rx="8" />
      <text className="eth-tour-diagram__label" x="155" y="52">
        {t("tutorials.diagram.audio_interface")}
      </text>
      <text className="eth-tour-diagram__small" x="155" y="66">
        {t("tutorials.diagram.sound_numbers")}
      </text>
      <path className="eth-tour-diagram__wire" d="M198 44 L 246 28 M198 64 L 246 80" />
      <rect className="eth-tour-diagram__box" x="246" y="14" width="68" height="28" rx="6" />
      <text className="eth-tour-diagram__label" x="280" y="32">
        {t("tutorials.diagram.computer")}
      </text>
      <rect className="eth-tour-diagram__box" x="246" y="66" width="68" height="28" rx="6" />
      <text className="eth-tour-diagram__label" x="280" y="84">
        {t("tutorials.diagram.headphones")}
      </text>
      <text className="eth-tour-diagram__small" x="90" y="104">
        {t("tutorials.diagram.inputs")}
      </text>
      <text className="eth-tour-diagram__small" x="222" y="104">
        {t("tutorials.diagram.outputs")}
      </text>
    </svg>
  );
}

/** An interface's numbered inputs: input 1 a mic (mono), input 2 a guitar (mono), 3/4 a stereo keyboard. */
export function InputsDiagram() {
  const t = useT();
  const inputs: ReadonlyArray<{ n: string; what: MessageKey }> = [
    { n: "1", what: "tutorials.diagram.mic" },
    { n: "2", what: "tutorials.diagram.guitar" },
    { n: "3", what: "tutorials.diagram.keys_left" },
    { n: "4", what: "tutorials.diagram.keys_right" },
  ];
  return (
    <svg viewBox="0 0 320 96" role="img" aria-label={t("tutorials.diagram.inputs_label")}>
      <rect className="eth-tour-diagram__accent" x="8" y="20" width="304" height="48" rx="8" />
      {inputs.map((x, i) => (
        <g key={x.n}>
          <circle className="eth-tour-diagram__box" cx={48 + i * 74} cy="44" r="14" />
          <text className="eth-tour-diagram__label" x={48 + i * 74} y="48">
            {x.n}
          </text>
          <text className="eth-tour-diagram__small" x={48 + i * 74} y="86">
            {t(x.what)}
          </text>
        </g>
      ))}
      <text className="eth-tour-diagram__small" x="85" y="14">
        {t("tutorials.diagram.mono_mono")}
      </text>
      <path className="eth-tour-diagram__wire" d="M196 76 H 270" />
      <text className="eth-tour-diagram__small" x="233" y="14">
        {t("tutorials.diagram.stereo_pair")}
      </text>
    </svg>
  );
}

/** A level meter: keep the loudest peaks in the green-yellow, never in the red. */
export function LevelDiagram() {
  const t = useT();
  return (
    <svg viewBox="0 0 320 64" role="img" aria-label={t("tutorials.diagram.level_label")}>
      <rect className="eth-tour-diagram__box" x="10" y="20" width="300" height="16" rx="4" />
      <rect className="eth-tour-diagram__accent" x="150" y="20" width="110" height="16" rx="4" />
      <rect x="276" y="20" width="34" height="16" rx="4" fill="var(--eth-color-danger)" />
      <text className="eth-tour-diagram__small" x="70" y="54">
        {t("tutorials.diagram.too_quiet")}
      </text>
      <text className="eth-tour-diagram__small" x="205" y="54">
        {t("tutorials.diagram.loudest")}
      </text>
      <text className="eth-tour-diagram__small" x="293" y="54">
        {t("tutorials.diagram.clipping")}
      </text>
    </svg>
  );
}
