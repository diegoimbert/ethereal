/**
 * The mobile app shell (design: the "Mobile UX design" comment on the issue): the same
 * arrangement and feature code as the desktop editor, laid out for touch screens.
 *
 *   phone portrait / tablet          phone landscape
 *   ┌ top: project · undo redo · share ┐   ┌ arrangement ──────────────────────┐
 *   │ arrangement                      │   │                                    │
 *   │                                  │   ├ top (left) │ transport (right)     ┤
 *   ├ status: position · tempo · follow┤   └────────────────────────────────────┘
 *   └ transport: stop play rec loop met┘
 *
 * The top bar and the transport keep their place in the React tree in every layout (the
 * grid moves them), so a rotation never remounts the project menu (launch, autosave, the
 * project URL) or the arrangement. No rail, floating panes or device windows: panels arrive
 * as sheets in later steps of.
 */
import { ExtensionRoots } from "@/app/ExtensionRoots";
import "./mobile.css";
import { memo, useEffect } from "react";
import { Redo2, Settings, Undo2 } from "lucide-react";
import { Button, ConfirmDialog, ContextMenuHost, IconButton, TitleTooltipHost } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { tText, useT } from "@/i18n";
import { ArrangementView } from "@/features/arrangement";
import { AudioSettingsDialog, openSettings } from "@/features/audio-settings";
import { MarkerLane } from "@/features/clip-editing";
import { ImportRoot } from "@/features/import";
import { ImportResolverRoot } from "@/features/import-resolver";
import { MediaRefsRoot } from "@/features/media-refs";
import { ProjectMenu } from "@/features/project";
import { ExportDialog } from "@/features/export";
import { bindSelectionHistory } from "@/features/time-edits/selectionHistory";
import { VersionsRoot } from "@/features/versions";
import { NewsRoot } from "@/features/news";
import { bindBackgroundStop } from "./background";
import { useKeepRecordingSetup } from "@/features/recording";
import { keepCenterOnRotation } from "./rotation";
import { isPhone, PHONE_TRACK_HEIGHT, TABLET_TRACK_HEIGHT, useFormFactor } from "./mode";
import { TrackSheetRoot } from "./TrackSheet";
import { ClipSheetRoot } from "./ClipSheet";
import { ClipActionBar, DragBadge } from "./ClipActions";
import { EditorPanel } from "./EditorPanel";
import { MobileMixer } from "./MobileMixer";
import { useMobileView } from "./viewState";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { TransportDock } from "./TransportDock";

const Arrangement = memo(ArrangementView);
const Markers = memo(MarkerLane);

/** The layout and the touch arrangement's setup, shared by the editor and the viewer shells. */
function useMobileLayout() {
  const layout = useFormFactor();
  const transport = useOptionalTransport();
  useEffect(() => (transport ? bindBackgroundStop(transport) : undefined), [transport]);
  // The view follows the playhead by default on touch screens (a pan detaches it).
  useEffect(() => arrangementView.getState().setFollowPlayhead(true), []);
  // Turning the device keeps the beat at the centre of the lanes there.
  useEffect(() => keepCenterOnRotation(arrangementView), []);
  // Phones: the narrowest track header column, so the lanes get the room, and rows tall enough
  // for the header's two lines (name, then mute / solo / arm). Not remembered: the desktop keeps its own.
  // Tablets: the desktop's header column, its rows tall enough for the two lines (mobile.css).
  const phone = isPhone(layout);
  useEffect(() => {
    useArrangementUi.getState().setPhoneHeader(phone);
    useArrangementUi.setState((s) => ({
      defaultHeight: Math.max(s.defaultHeight, phone ? PHONE_TRACK_HEIGHT : TABLET_TRACK_HEIGHT),
    }));
  }, [phone]);
  return layout;
}

export function MobileShell() {
  const t = useT();
  const layout = useMobileLayout();
  // Arms and punch come back when a project opens again.
  useKeepRecordingSetup();
  // Undo / redo bring back the time selections of their step (as on desktop).
  useEffect(() => bindSelectionHistory(), []);

  return (
    <div className="eth-shell eth-mshell" data-layout={layout} data-testid="mobile-shell">
      <header className="eth-mshell__top" data-slot="top">
        <div className="eth-mshell__project" data-slot="project">
          <ProjectMenu />
        </div>
        <UndoRedo />
        <div className="eth-mshell__spacer" />
        {/* The project menu's "export audio…" opens it; while rendering with the
            dialog closed, its progress chip sits here. */}
        <div data-slot="export">
          <ExportDialog />
        </div>
        <ExtensionRoots shell="mobile" slot="top-bar" />
        <IconButton
          size="lg"
          tone="ghost"
          className="eth-mdock__btn eth-mdock__btn--small"
          label={t("mobile.shell.settings")}
          icon={<Settings />}
          onClick={() => openSettings()}
        />
      </header>
      <main className="eth-mshell__main" data-slot="main">
        <div className="eth-mshell__stage">
          <div data-slot="markers">
            <Markers />
          </div>
          <Arrangement />
          <ClipActionBar />
          <MixerView />
          <EditorPanel />
        </div>
      </main>
      <footer className="eth-mshell__dock" data-slot="transport-bar">
        <TransportDock />
      </footer>
      <AudioSettingsDialog />
      <ImportRoot />
      <MediaRefsRoot />
      <ImportResolverRoot />
      <VersionsRoot />
      <NewsRoot />
      <ExtensionRoots shell="mobile" slot="overlay" />
      <ConfirmDialog />
      <TrackSheetRoot />
      <ClipSheetRoot />
      <DragBadge />
      <ContextMenuHost />
      <TitleTooltipHost />
    </div>
  );
}

/** A read-only view on a touch screen (`state/viewer.ts`): the band of whatever shows it is its top. */
export function MobileViewerShell() {
  const layout = useMobileLayout();
  return (
    <div className="eth-shell eth-mshell eth-mshell--read-only" data-layout={layout} data-testid="mobile-shell" data-read-only>
      <main className="eth-mshell__main" data-slot="main">
        <div className="eth-mshell__stage">
          <div data-slot="markers">
            <Markers />
          </div>
          <Arrangement />
          <MixerView />
          <EditorPanel />
        </div>
      </main>
      <footer className="eth-mshell__dock" data-slot="transport-bar">
        <TransportDock />
      </footer>
      <TitleTooltipHost />
    </div>
  );
}

function UndoRedo() {
  const transport = useOptionalTransport();
  const history = useProjectStore((s) => s.history);
  const t = useT();
  const hasProject = useProjectStore((s) => s.project !== null);
  const disabled = !transport || !hasProject;
  return (
    <div className="eth-mshell__history" role="group" aria-label={t("mobile.shell.history")}>
      <Button
        tone="ghost"
        aria-label={t("mobile.shell.undo")}
        title={history.undo_label ? t("history.undo_named", { what: tText(history.undo_key, history.undo_label) }) : t("history.nothing_to_undo")}
        className="eth-mdock__btn eth-mdock__btn--small"
        disabled={disabled || !history.can_undo}
        onClick={() => void transport?.send(cmd("Edit", { type: "Undo" }))}
      >
        <Undo2 aria-hidden />
      </Button>
      <Button
        tone="ghost"
        aria-label={t("mobile.shell.redo")}
        title={history.redo_label ? t("history.redo_named", { what: tText(history.redo_key, history.redo_label) }) : t("history.nothing_to_redo")}
        className="eth-mdock__btn eth-mdock__btn--small"
        disabled={disabled || !history.can_redo}
        onClick={() => void transport?.send(cmd("Edit", { type: "Redo" }))}
      >
        <Redo2 aria-hidden />
      </Button>
    </div>
  );
}

function MixerView() {
  return useMobileView((s) => s.view) === "mixer" ? <MobileMixer /> : null;
}
