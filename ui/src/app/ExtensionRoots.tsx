// The extensions' components in a shell (`@/ext` "shell.root"), each in its own error
// boundary, and their start-up hooks ("app.init"). Nothing renders without extensions.
import { useEffect } from "react";
import { contributions, useContributions, type ShellRoot } from "@/ext";
import { PanelBoundary } from "./PanelBoundary";

type Shell = NonNullable<ShellRoot["shells"]>[number];
type Slot = NonNullable<ShellRoot["slot"]>;

/** The roots contributed to `slot` of `shell`. `bare`: no wrapper per root (inside a boundary). */
export function ExtensionRoots({ shell, slot, bare }: { shell: Shell; slot: Slot; bare?: boolean }) {
  const roots = useContributions("shell.root").filter((r) => (r.shells ?? ["editor"]).includes(shell) && (r.slot ?? "overlay") === slot);
  return (
    <>
      {roots.map(({ name, label, Component, dataSlot }) => {
        if (bare) return <Component key={name} />;
        const boundary = (
          <PanelBoundary key={name} name={name} label={label} variant={slot === "overlay" || slot === "device-windows" ? "overlay" : undefined}>
            <Component />
          </PanelBoundary>
        );
        return slot === "top-bar" ? (
          <div key={name} data-slot={dataSlot ?? name}>
            {boundary}
          </div>
        ) : (
          boundary
        );
      })}
    </>
  );
}

/** Run the extensions' "app.init" once (the shell's lifetime). */
export function useExtensionInit(): void {
  useEffect(() => {
    const offs = contributions("app.init").map((init) => init());
    return () => {
      for (const off of offs) if (typeof off === "function") off();
    };
  }, []);
}
