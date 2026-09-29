/**
 * The widget dispatcher: one case per catalog entry (`ether_protocol::layout::Widget`,
 * append-only; a new widget = a BCR + a case here).
 */

import type { Widget, WidgetSize } from "@/generated";
import { useParam } from "./context";
import { EqCurveWidget } from "./eq";
import { MacrosWidget, MeterWidget, RackChainsWidget, SampleWaveformWidget, SpectrumWidget, TunerWidget, ZoneMapWidget } from "./widgets/data";
import { ChoiceWidget, KnobWidget, NumberWidget, SliderWidget, ToggleWidget } from "./widgets/params";
import {
  CrossoverWidget,
  EnvelopeWidget,
  FilterCurveWidget,
  LfoWidget,
  Missing,
  OscillatorWidget,
  StepEditorWidget,
  TransferCurveWidget,
  XyPadWidget,
} from "./widgets/typed";

export interface WidgetViewProps {
  widget: Widget;
  size: WidgetSize;
  /** `LayoutItem.label`: `null` = the param's name (or the widget's default caption). */
  label: string | null;
}

type ParamWidgetType = "Knob" | "Slider" | "Toggle" | "Choice" | "Number";

function ParamWidgetView({ widget, size, label }: WidgetViewProps & { widget: Extract<Widget, { type: ParamWidgetType }> }) {
  const b = useParam(widget.param);
  if (!b) return <Missing type={widget.type} />;
  const props = { binding: b, size, label: label ?? b.info.name };
  switch (widget.type) {
    case "Knob":
      return <KnobWidget {...props} />;
    case "Slider":
      return <SliderWidget {...props} vertical={widget.vertical} />;
    case "Toggle":
      return <ToggleWidget {...props} />;
    case "Choice":
      return <ChoiceWidget {...props} />;
    case "Number":
      return <NumberWidget {...props} />;
  }
}

/** Render one catalog widget inside a `LayoutContext`. */
export function WidgetView({ widget, size, label }: WidgetViewProps) {
  switch (widget.type) {
    case "Knob":
    case "Slider":
    case "Toggle":
    case "Choice":
    case "Number":
      return <ParamWidgetView widget={widget} size={size} label={label} />;
    case "Envelope":
      return <EnvelopeWidget widget={widget} size={size} label={label} />;
    case "FilterCurve":
      return <FilterCurveWidget widget={widget} size={size} label={label} />;
    case "TransferCurve":
      return <TransferCurveWidget widget={widget} size={size} label={label} />;
    case "Oscillator":
      return <OscillatorWidget widget={widget} size={size} label={label} />;
    case "Lfo":
      return <LfoWidget widget={widget} size={size} label={label} />;
    case "StepEditor":
      return <StepEditorWidget widget={widget} size={size} label={label} />;
    case "XyPad":
      return <XyPadWidget widget={widget} size={size} label={label} />;
    case "Crossover":
      return <CrossoverWidget widget={widget} size={size} label={label} />;
    case "SampleWaveform":
      return <SampleWaveformWidget widget={widget} size={size} label={label} />;
    case "ZoneMap":
      return <ZoneMapWidget widget={widget} size={size} label={label} />;
    case "Spectrum":
      return <SpectrumWidget widget={widget} size={size} label={label} />;
    case "Tuner":
      return <TunerWidget widget={widget} size={size} label={label} />;
    case "Meter":
      return <MeterWidget widget={widget} size={size} label={label} />;
    case "RackChains":
      return <RackChainsWidget widget={widget} size={size} label={label} />;
    case "Macros":
      return <MacrosWidget widget={widget} size={size} label={label} />;
    case "EqCurve":
      return <EqCurveWidget widget={widget} size={size} label={label} />;
  }
}
