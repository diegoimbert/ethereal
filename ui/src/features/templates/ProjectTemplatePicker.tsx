import "./templates.css";
import { useState } from "react";
import { useEngineCommands } from "@/features/transport-bar/engine";
import { cmd } from "@/transport";
import { templateMenu, type ProjectTemplateChoice } from "./model";
import { TemplateList } from "./TemplateDialogs";
import { useTemplates } from "./useTemplates";

export interface ProjectTemplatePickerProps {
  /** `undefined` until the user picks: the default template is shown as picked. */
  value: ProjectTemplateChoice | undefined;
  onChange(value: ProjectTemplateChoice): void;
}

/**
 * "Start from": an empty project or one of the project templates (the default one is
 * picked until the user chooses). User templates have a menu: use for new projects,
 * rename, delete. Hidden when there are no project templates.
 */
export function ProjectTemplatePicker({ value, onChange }: ProjectTemplatePickerProps) {
  const { send } = useEngineCommands();
  const { templates } = useTemplates("Project", true);
  const [error, setError] = useState<string | null>(null);
  if (!templates || templates.length === 0) return null;
  const current = value === undefined ? (templates.find((t) => t.default)?.id ?? null) : value;
  const setDefault = (id: string | null) => {
    setError(null);
    void send(cmd("Template", { type: "SetDefault", template: id })).then((r) => {
      if (!r) setError("Couldn’t change the default template.");
    });
  };
  return (
    <div className="eth-template-picker">
      <span className="eth-template-picker__label">Start from</span>
      <TemplateList
        label="Project templates"
        templates={templates}
        selected={current}
        onPick={(t) => onChange(t.id)}
        onMenu={(e, t) => templateMenu(e, t, setDefault)}
        leading={
          <li className="eth-templates__row">
            <button type="button" className="eth-templates__item" aria-current={current === null || undefined} onClick={() => onChange(null)}>
              <span className="eth-templates__name">Empty project</span>
            </button>
          </li>
        }
      />
      {error && (
        <p className="eth-template-dialog__error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

