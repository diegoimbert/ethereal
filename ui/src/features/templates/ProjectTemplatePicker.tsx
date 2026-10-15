import "./templates.css";
import { useState } from "react";
import { useEngineCommands } from "@/features/transport-bar/engine";
import { cmd } from "@/transport";
import { templateMenu, type ProjectTemplateChoice } from "./model";
import { TemplateList } from "./TemplateDialogs";
import { useTemplates } from "./useTemplates";
import { useT } from "@/i18n";

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
  const t = useT();
  const { send } = useEngineCommands();
  const { templates } = useTemplates("Project", true);
  const [error, setError] = useState<string | null>(null);
  if (!templates || templates.length === 0) return null;
  const current = value === undefined ? (templates.find((tpl) => tpl.default)?.id ?? null) : value;
  const setDefault = (id: string | null) => {
    setError(null);
    void send(cmd("Template", { type: "SetDefault", template: id })).then((r) => {
      if (!r) setError(t("templates.picker.default_failed"));
    });
  };
  return (
    <div className="eth-template-picker">
      <span className="eth-template-picker__label">{t("templates.picker.start_from")}</span>
      <TemplateList
        label={t("templates.picker.list")}
        templates={templates}
        selected={current}
        onPick={(tpl) => onChange(tpl.id)}
        onMenu={(e, tpl) => templateMenu(e, tpl, setDefault)}
        leading={
          <li className="eth-templates__row">
            <button type="button" className="eth-templates__item" aria-current={current === null || undefined} onClick={() => onChange(null)}>
              <span className="eth-templates__name">{t("templates.picker.empty")}</span>
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
