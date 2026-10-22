// The extensions' choices in a project's delete confirmation (`@/ext` "project.deleteChoices",
// e.g. "also delete it from the cloud"): switches shown with the question, run once the
// project is deleted.
import { useReducer } from "react";
import { contributions, type ProjectDeleteChoice } from "@/ext";
import { Toggle } from "@/kit";

/** The choices for one confirmation, with what the user switched. */
export class DeleteChoices {
  readonly items: { choice: ProjectDeleteChoice; checked: boolean }[];

  constructor(project: { id: string; name: string }) {
    this.items = contributions("project.deleteChoices")
      .flatMap((f) => [...f(project)])
      .map((choice) => ({ choice, checked: choice.checked ?? false }));
  }

  /** Run the switched-on choices (after the delete); each failure goes to `onError`. */
  async runChecked(onError: (e: unknown) => void): Promise<void> {
    for (const it of this.items) {
      if (!it.checked) continue;
      try {
        await it.choice.run();
      } catch (e) {
        onError(e);
      }
    }
  }
}

/** The switches of `choices` (nothing when there are none). */
export function DeleteChoiceSwitches({ choices }: { choices: DeleteChoices }) {
  const [, redraw] = useReducer((n: number) => n + 1, 0);
  if (choices.items.length === 0) return null;
  return (
    <span className="eth-project-delete-choices">
      {choices.items.map((it) => (
        <Toggle
          key={it.choice.id}
          size="sm"
          checked={it.checked}
          label={it.choice.label}
          onChange={(on) => {
            it.checked = on;
            redraw();
          }}
        />
      ))}
    </span>
  );
}
