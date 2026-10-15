# Debug panels

Floating windows for looking inside a feature (a pipeline, a cache, …). Each one is a kit
`FloatingWindow`: it moves by its title bar, resizes from any edge or corner, stays inside the
viewport and comes back where it was (geometry per panel, open state per panel, both in
localStorage).

## Adding one

```tsx
// features/<feature>/debug/register.tsx
import { registerDebugPanel } from "@/features/debug";
import { MyDebugPanel } from "./MyDebugPanel";

registerDebugPanel({
  id: "my-feature", // stable: the open state and window geometry are remembered under it
  title: "My feature", // title bar, palette ("Debug: My feature"), Settings › Advanced
  render: () => <MyDebugPanel />,
  defaultSize: { width: 720, height: 480 }, // optional (px)
  keywords: "words the palette also matches", // optional
  onOpen: () => {}, // optional: e.g. turn on whatever feeds the panel
});
```

Import the registration once from the feature's `index.ts` (`import "./debug/register";`).
That's all: the palette entry, the Settings › Advanced switch and the window come from the
registry. Open it from code with `openDebugPanel("my-feature")`.

## Content in a resizable window

The panel fills the window body (a flex column) and the user can make it small:

- the root: `display: flex; flex-direction: column;` (it gets `flex: 1; min-height: 0;
  min-width: 0` from the window);
- a toolbar that wraps (`flex-wrap: wrap`), then a scrolling region
  (`flex: 1; min-height: 0; overflow: auto`);
- nothing wider than the window: `min-width: 0` on flex/grid children holding text,
  `overflow-wrap: anywhere` for ids and JSON, `white-space: pre-wrap` in `<pre>`, labels that
  truncate with an ellipsis (and a `title` with the full value);
- kit components and design tokens only (both themes).
