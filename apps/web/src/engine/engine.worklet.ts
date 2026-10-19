// AudioWorklet module: registers the engine's processor (`./engineProcessor.ts`).
import { EtherEngineProcessor, PROCESSOR_NAME, type EngineProcessorOptions } from "./engineProcessor";

// AudioWorkletGlobalScope (not in lib.dom).
declare function registerProcessor(name: string, ctor: new (options: EngineProcessorOptions) => unknown): void;

export { PROCESSOR_NAME };

registerProcessor(PROCESSOR_NAME, EtherEngineProcessor);
