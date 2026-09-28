// collab-social (docs/COLLAB.md §12): chat (left-sidebar section, toasts, Mod+Shift+M),
// notes pinned wherever cursors are tracked (arranger and piano roll), peers' playheads, and
// the "Hide users and notes" preference (`useHideOthers` in the collab store).
export { ArrangerSocialLayer } from "./ArrangerSocial";
export { ChatPanel } from "./ChatPanel";
export { ChatToasts } from "./ChatToasts";
export { focusChat } from "./chatStore";
export { EditorNotes, type EditorNotesProps } from "./EditorSocial";
export { leaveNoteEntries, rulerNoteEntries, withSeparator, type NoteSurface } from "./notes/notesStore";
