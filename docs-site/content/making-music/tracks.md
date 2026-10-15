---
title: tracks
order: 10
summary: Add, select, solo, rename, reorder, resize and group tracks, and learn what each kind of track is for.
platform: all
audience: beginner
covers:
  - arrangement/adding-tracks-menu-drag-or-empty-space-right-click #adding-a-track
  - arrangement/new-midi-track-opens-the-instrument-picker #choosing-an-instrument
  - arrangement/track-types #kinds-of-tracks
  - arrangement/selecting-tracks-click-shift-cmd-ctrl #selecting-tracks
  - arrangement/solo-is-exclusive-cmd-ctrl-click-adds #mute-and-solo
  - arrangement/arm-is-exclusive-cmd-ctrl-click-to-arm-several #arming-for-recording
  - arrangement/header-volume-and-pan #volume-and-pan
  - arrangement/rename-tracks-empty-name-restores-the-default #renaming-a-track
  - arrangement/reorder-tracks-by-dragging-headers #moving-tracks
  - arrangement/track-height-header-width #track-height-and-header-width
  - arrangement/group-and-ungroup-tracks #groups
  - arrangement/vca-tracks #vca-tracks
  - arrangement/track-menu-cut-copy-paste-duplicate-delete-tracks #cut-copy-paste-duplicate-and-delete
  - arrangement/cut-tracks-can-only-be-pasted-right-away #cut-copy-paste-duplicate-and-delete
  - arrangement/track-input-from-another-track #recording-from-another-track
---

Tracks are the rows of the arrangement. Each one holds clips (or other tracks, for a group) and has its own chain of devices. This page covers how to add and organize them. You only need an open project.

## adding a track

There are three ways to add a track:

- Click **add track** in the corner above the track headers, or the **add track** row under the last track. The menu offers a **midi track**, an **audio track** or a **return track**. A click adds it after the last track (a return track right after the selected return, or after the other returns, above the master).
- Drag an entry out of that menu and drop it between two tracks, or onto a group's header, to create it right there. A return track can only be dropped between the other returns.
- Right-click the empty space below the tracks and choose **add audio track**, **add midi track** or **add return track**. The same menu has **import audio…**. A track's own right-click menu also has **add return track**.

The command palette ([[action:palette.open]]) also has commands to add an audio, a MIDI or a return track.

### choosing an instrument

A new MIDI track opens an instrument library under its header. Pick an instrument to replace the track's default synth, or click away to keep the synth. You can change the instrument later in the track's devices (see [devices and presets](/docs/making-music/devices-and-presets)).

## kinds of tracks

| kind | what it holds |
|---|---|
| **midi** | MIDI clips (notes), played by the instrument at the start of its device chain. |
| **audio** | Audio clips (recordings and samples). |
| **group** | Other tracks. Their sound is summed through the group, so one fader and one device chain can process them all. |
| **vca** | Nothing to play. It turns the volume of the tracks assigned to it up or down. |
| **return** | Effects that tracks reach through sends. Returns sit at the bottom, above the master. |
| **master** | The final mix. It is pinned at the bottom of the arrangement. |

A group can hold audio, MIDI and group tracks, but not returns or VCAs.

> [!note]
> The **add track** menu creates MIDI, audio and return tracks. Groups and VCAs come from the commands below. See [sends and return tracks](/docs/making-music/mixing#sends-and-return-tracks) for what a return track is for.

## selecting tracks

Click a track header to select the track. The inspector then shows its settings. [[key:Shift]]-click selects a range of tracks, and [[key:Mod]]-click adds a track to the selection or removes it. Clicking empty space in a track's lane also selects that track.

## mute and solo

Each header has a mute button (the speaker) and a solo button (the headphones).

Solo is exclusive. A plain click solos only that track and unsolos the others. Clicking the only soloed track unsolos it. Hold [[key:Mod]] while clicking to add a track to the soloed tracks, or to remove it, without touching the others.

## arming for recording

The round arm button readies a track for recording. Arming is exclusive as well: a plain click arms only that track. [[key:Mod]]-click arms several. If you arm an audio track while no input device is open, Ethereal offers to choose one. See [recording](/docs/making-music/recording).

## volume and pan

At the right end of each header are a small horizontal volume fader and a pan knob.

- **Volume**: drag left or right. Hold [[key:Shift]] for fine steps. Double-click to reset it to 0 dB. The value in dB shows while you hover or drag.
- **Pan**: drag up or down. Double-click to center it.

With several tracks selected, dragging one of their faders moves all of them by the same number of dB, so their balance stays the same.

## renaming a track

Double-click a track's name, type, then press [[key:Enter]] or click away. [[key:Escape]] cancels. If you clear the name, the track goes back to its default name. An unnamed MIDI track shows its instrument's name.

## moving tracks

Drag a header up or down. A line shows where the track will go: release between two rows to place it there. Release over the middle of a group's header to put the track inside the group. A group moves together with the tracks inside it. Near the top or bottom edge of the arrangement, the view scrolls. Press [[key:Escape]] to cancel the drag. Each move is one undo step.

Return tracks stay together at the bottom, above the master, and you can reorder them the same way: drag a return's header between two other returns. Over the regular tracks, they turn red and the pointer shows that the return can't go there. Sends keep going to the same return, wherever it moves. The master never moves.

## track height and header width

- Drag the bottom edge of a track's header to make the track taller or shorter. It moves in steps. Hold [[key:Alt]] to resize freely. Double-click the edge to reset it.
- Drag the edge between the headers and the lanes to make the header column wider or narrower. Double-click it to reset. When the column gets narrow, the **grid** picker moves into a **…** menu in the corner.

## groups

Select some tracks and press [[action:track.group]] (or right-click a header and choose **group track** / **group 3 tracks**). If the tracks come from different groups, they are first moved next to each other, then wrapped in a new group.

- Click a group's round badge (the chevron) to fold or unfold it. A folded group's lane shows a summary of where its tracks have clips.
- [[action:track.ungroup]] (or **ungroup** in the header menu) removes the group and leaves its tracks in place. If the group has its own devices, automation, sends or routings, Ethereal asks first, because those are removed with it. You can undo it.

## vca tracks

A VCA controls the volume of other tracks without carrying any audio. Select the tracks, right-click one of their headers and choose **new vca for track** (or **new vca for 3 tracks**). The same menu can **assign to vca** or **remove from vca**. The inspector also has a **vca** row for each track, and a VCA's inspector lists the tracks it controls.

## cut, copy, paste, duplicate and delete

Right-click a header to act on every selected track. The labels show how many (**delete 3 tracks**). The usual keys work too when tracks are selected: [[action:edit.cut]], [[action:edit.copy]], [[action:edit.paste]], [[action:edit.duplicate]] and [[action:edit.delete]]. The master track has no menu.

- **Duplicate** copies everything: devices, clips, automation and sends. The copies appear right after the selected tracks.
- **Paste** puts copies of the copied tracks after the selected track (or at the end).
- **Cut** deletes the tracks. Pasting restores them only while the cut is still your latest undo step. After any other edit the cut tracks can no longer be pasted, but [[action:edit.undo]] still brings them back.

Copies of return tracks and VCAs stay right after their source, because a group can't hold them.

## colors

Each new track gets a color from the palette. To change it, select the track and pick a color in the inspector, next to its name. Clips use their track's color unless you give them their own (see [clips](/docs/making-music/clips#clip-colors)).

## recording from another track

An audio track's input can be another track instead of an audio interface, to record (resample) what that track plays. Select the audio track and open the **input** row in the inspector: choose **from** and the source track, then where to take its signal:

- **pre fx**: before the source track's devices.
- **post fx**: after its devices.
- **post mixer**: after its volume and pan.

Choosing a track also sets the track's monitoring to **in**, so you hear the source through it. See [recording](/docs/making-music/recording).

## where next

- [clips](/docs/making-music/clips): make, move and edit what's on the tracks.
- [devices and presets](/docs/making-music/devices-and-presets): instruments and effects on a track.
- [arranging](/docs/making-music/arranging): edit whole sections of the song at once.
