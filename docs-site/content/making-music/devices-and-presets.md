---
title: devices and presets
order: 50
summary: Add instruments and effects to a track, open their windows, turn their knobs and save your sounds as presets.
platform: all
audience: beginner
covers:
  - devices/adding-a-device #adding-a-device
  - devices/instruments-only-on-midi-tracks-one-per-chain #what-goes-where
  - devices/chain-order-is-fixed-by-section #the-order-of-a-chain
  - devices/device-window-vs-parameter-list #the-device-window-and-the-parameter-list
  - devices/bypass-rename-copy-paste-devices #bypass-rename-copy-and-paste
  - devices/hover-only-header-buttons #the-device-header
  - devices/knob-gestures #turning-knobs
  - devices/touch-knobs-fine-mode-by-moving-sideways #turning-knobs
  - devices/no-typing-values-into-knobs #exact-values
  - devices/parameter-context-menu #the-parameter-menu
  - devices/show-all-parameters-and-pinning #plugins-with-many-parameters
  - devices/presets-load-save-factory-vs-user #presets
  - devices/presets-are-files-in-your-user-library #where-presets-are-saved
  - devices/current-preset-name-is-per-session #the-preset-name-on-the-button
  - devices/device-library-favourites-sort-and-format-toggles #favourites-sorting-and-plugin-formats
---

Devices make and shape the sound of a track. An **instrument** turns MIDI notes into audio, an **audio effect** changes the audio passing through it, and a **MIDI effect** changes notes before they reach the instrument. This page shows how to add them, how to use their controls, and how presets work. You need a project with at least one track: see [tracks](/docs/making-music/tracks) if you have none yet.

## where devices live

Select a track: its devices appear in the **inspector**, the panel on the right, under the track's name and mixer. A track's devices form its **chain**: the signal goes through them from top to bottom. Each device is a card with a header (its power button, its name and a few buttons), and the device's own controls open in a window.

## adding a device

Each section of the chain has its own add button: the **+** next to the section's label, or the dashed **add instrument**, **add audio effect** or **add MIDI effect** card at the end of the section. Either one opens the device library:

- type in the search field to filter, then press [[key:Enter]] or click a row to add it;
- the **all**, **factory** and **plugins** chips choose between the built-in devices and your plugins (plugins are desktop only);
- the star filters your favourites, and the sort menu orders the list by name, most used or most recent;
- drag a row onto the chain to put the device at a place in its section: before a device, or at the section's end.

A device you add opens its window right away, so you can start playing with it. You can also add a device from the command palette ([[action:palette.open]]).

### what goes where

- Instruments and MIDI effects need notes, so they only go on **MIDI tracks**. Audio tracks take audio effects only.
- A chain has **one instrument**. Adding another instrument replaces the current one, as one undo step.
- VCA tracks take no devices at all.

If a device can't go on the selected track, a message says why (for example "select a MIDI track").

### the order of a chain

The sections always come in the same order: MIDI effects first, then the instrument, then audio effects. You can reorder devices inside a section by dragging a card's header up or down, but a MIDI effect always stays before the instrument and an audio effect after it.

## the device header

From left to right, a device header has:

- the **window** button, which opens the device's own window;
- the **power** button, which turns the device on or bypasses it;
- the device's **name**;
- the **presets** button and the **+** button that adds a modulator. These two only appear while the pointer is over the card, to keep the chain tidy;
- a **sidechain** selector, on devices that can be keyed by another track (see [modulation and sidechain](/docs/making-music/modulation-and-sidechain#sidechain));
- the **parameters** button (sliders icon), shown like presets while the pointer is over the card (or while the list is open), and **×** to remove the device.

### the device window and the parameter list

The card itself only shows the header. There are two ways to see a device's controls, and they are independent:

- The **window** button opens the device's own interface in a floating window: the factory device's layout, or a plugin's own editor. You can move and resize it, and it comes back where you left it, on this computer, the next time you open it.
- The **parameters** button (sliders icon) shows a plain list of every parameter on the card itself, with a search field when the list is long. It works the same for every device, built-in or plugin.

Racks are the exception: a rack has no window, because its chains and macros are on its card (see [racks and macros](/docs/making-music/racks-and-macros)).

### bypass, rename, copy and paste

- Click the **power** button to bypass a device (the sound passes through unchanged). Click it again to turn the device back on.
- Double-click the name to rename the device. An empty name brings back its original name.
- Click a card to select it, then press [[action:edit.copy]] to copy it and [[action:edit.paste]] to paste the copy after the selected device. The copy keeps everything: settings, a plugin's state, a rack's chains or a drum rack's pads.
- [[key:Mod]]-click adds a card to the selection or takes it out, and [[key:Shift]]-click selects the cards in between: MIDI effects, the instrument and audio effects together. A paste puts each copy in its own section, in one undo step. A pasted instrument replaces the chain's instrument, after asking.
- Right-click the header for the same actions and more: **copy**, **paste after**, **rename…**, **bypass**, **group into … rack**, one **add …** entry per modulator type, and **delete device**.

## turning knobs

- **Drag** a knob up or down to change it. Hold [[key:Shift]] while dragging for fine control (ten times slower).
- **Double-click** a knob to reset it to its default value.
- A focused knob also answers the arrow keys, [[key:PageUp]], [[key:PageDown]], [[key:Home]] and [[key:End]].
- Horizontal sliders: click the track to jump there, or drag the thumb. [[key:Shift]] makes it finer.

On a touchscreen [[mobile]], drag your finger up or down on the knob. The further your finger moves sideways away from the knob while you drag, the finer the change, so you can make big moves and then precise ones without lifting your finger (see [editing on touch](/docs/mobile/editing-on-touch)).

### exact values

You can't type a value into a knob. To set a precise value, drag with [[key:Shift]] held, or focus the knob and use the arrow keys. A few controls do take typed numbers, such as number fields and the multisampler's zone settings. For automation points, right-click a point and choose **set value…** (see [automation](/docs/making-music/automation#set-an-exact-value)).

### the parameter menu

Right-click any device parameter for:

- **show automation lane**: opens that parameter's automation lane under the track;
- **reset to default**.

A parameter that is automated shows a small marker, and its knob follows the automation (see [automation](/docs/making-music/automation#automated-knobs-follow-their-lane)).

### plugins with many parameters

A plugin can have hundreds of parameters. Its parameter list on the card is capped in height and scrolls. **show all n parameters…** opens a searchable list of every parameter, where you can **pin** the ones you use. Pinned parameters come first on the card.

Pins belong to the plugin, not to one instance: they apply to every copy of that plugin in every project. They are kept on this computer only.

## presets

A preset is a saved sound for one kind of device. The **presets** button in the device header (it appears when you hover the card, and shows the current preset's name, or **presets**) opens the list:

- **factory** presets come with Ethereal, **user** presets are the ones you saved;
- search filters by name and tags;
- click a preset to load it. Loading is one undo step.

To save the current sound, choose **save preset…**, give it a name and, if you like, a few tags separated by commas. If a preset of yours already has that name, the button changes to **replace**: click it again to overwrite. Right-click one of your presets (or use its **…** button) to rename or delete it. Factory presets can't be changed.

Plugins have presets too: a plugin preset stores the plugin's complete state.

### where presets are saved

User presets are files in your **user library**, so they're available in every project and also show up in the [browser](/docs/sounds-and-plugins/browser)'s presets:

- on the desktop app, in `Documents/Ethereal/User Library/Presets/<device>/` as `.etherpreset` files (plugin presets under `Presets/plugins/<format>/<plugin id>/`);
- in the web app, in the browser's storage, like your projects.

A sampler or multisampler preset also copies its samples into the user library's `Samples` folder, so the preset still works in other projects.

### the preset name on the button

The presets button shows the last preset you loaded on that device **during this session**. After you reopen a project it no longer shows where a device's sound came from, and turning a knob after loading a preset doesn't mark it as modified.

## favourites, sorting and plugin formats

The device library remembers a few choices on this computer:

- click the star on a row to make it a favourite, and the star at the top to list only favourites;
- the sort menu: **name**, **most used** or **most recent**;
- when your plugins come in several formats, the **CLAP**, **VST3** and **AU** chips choose which formats are listed. A plugin installed in several formats appears once, in your preferred format, unless you turn off **show each plugin once** in **settings › plugins › plugin list** (see [the same plugin in several formats](/docs/sounds-and-plugins/plugins#the-same-plugin-in-several-formats)). At least one format stays on.

## where next

- [instruments](/docs/making-music/instruments): what each built-in instrument does.
- [effects](/docs/making-music/effects): the built-in audio and MIDI effects.
- [automation](/docs/making-music/automation): make parameters move over time.
