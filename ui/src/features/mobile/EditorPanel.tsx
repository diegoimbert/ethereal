/**
 * The clip editor on touch screens (#522, the design's §7): the desktop drawer's editors (piano
 * roll, audio clip editor, automation) in a panel over the arrangement, under the top bar and
 * above the transport, so you can play while editing. Full height on phones, the lower part on
 * tablets. Opened by a double-tap on a clip or the action bar's "edit"; the close button and the
 * system back close it. On phones the piano roll's velocity / expression lanes are hidden until
 * the head's "velocity" toggle shows them (the note grid needs the height).
 */
import { useEffect } from "react";
import { BarChart3, X } from "lucide-react";
import { Button, IconButton } from "@/kit";
import { useClip, useEditedClipId } from "@/state";
import { DrawerTabs, EditorDrawer } from "@/app/shell/EditorDrawer";
import { useBackToClose } from "./backToClose";
import { closeClipEditor, useMobileEditor } from "./editorState";
import { useT } from "@/i18n";

export function EditorPanel() {
  const t = useT();
  const open = useMobileEditor((s) => s.open);
  const lanes = useMobileEditor((s) => s.lanes);
  const clip = useClip(useEditedClipId());
  const shown = open && !!clip;
  useBackToClose(shown, closeClipEditor);
  // The clip went away (deleted, undone): the editor closes.
  useEffect(() => {
    if (open && !clip) closeClipEditor();
  }, [open, clip]);
  if (!shown) return null;
  return (
    <section className="eth-meditor" role="dialog" aria-label={t("mobile.editor.label")} data-testid="mobile-editor" data-lanes={lanes || undefined}>
      <header className="eth-meditor__head">
        <DrawerTabs />
        {clip.content.type === "Midi" && (
          <Button tone="ghost" className="eth-meditor__lanes" active={lanes} aria-pressed={lanes} onClick={() => useMobileEditor.setState({ lanes: !lanes })}>
            <BarChart3 aria-hidden /> {t("mobile.editor.velocity")}
          </Button>
        )}
        <IconButton
          size="lg"
          tone="ghost"
          className="eth-mdock__btn eth-mdock__btn--small"
          label={t("mobile.editor.close")}
          icon={<X />}
          onClick={closeClipEditor}
        />
      </header>
      <div className="eth-meditor__body">
        <EditorDrawer />
      </div>
    </section>
  );
}
