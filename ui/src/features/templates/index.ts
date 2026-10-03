// OWNERSHIP: the `templates` node owns `ui/src/features/templates/**` (v0.3, CONTRACTS.md §13.10).
export { TemplateDialogs, TemplateList } from "./TemplateDialogs";
export { ProjectTemplatePicker } from "./ProjectTemplatePicker";
export {
  insertTemplate,
  newProjectCommand,
  placementAfter,
  templateMenu,
  templateTrackEntries,
  tracksToSave,
  useTemplateDialog,
  type Placement,
  type ProjectTemplateChoice,
  type TemplateDialog,
} from "./model";
