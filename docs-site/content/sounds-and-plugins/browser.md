---
title: the browser
order: 10
summary: Find, preview and drag in samples, load presets, and add your own sample folders.
platform: all
audience: beginner
covers:
  - browser/the-browser #finding-sounds
  - browser/preview-samples #previewing-samples
  - browser/drag-samples-into-the-project #dragging-samples-in
  - browser/midi-files-from-the-browser #midi-files
  - browser/presets-in-the-browser #presets
  - browser/add-your-own-folders #adding-your-own-folders
  - browser/folder-depth-limit #what-gets-indexed
  - browser/tags-and-favourites #favourites-and-tags
  - browser/packs #packs
  - browser/file-filter-rules #choosing-which-files-are-listed
  - browser/project-media-and-open-projects #projects
---

The browser is where your sounds live: samples, MIDI files, device presets and projects. This page shows how to find a sound, hear it, and put it in your song. You need an open project. If you don't have one yet, start with [your first beat](/docs/welcome/first-beat).

## opening the browser

Click the **library** icon at the top of the icon rail on the left edge of the window. Clicking it again closes the panel. You can also open it from the command palette ([[action:palette.open]]) with **open library**.

## finding sounds

The chips under the search field choose what you're looking at:

- **samples**: audio files.
- **midi**: MIDI files.
- **presets**: saved device settings, both the factory ones and your own.
- **projects**: your Ethereal projects.

With no search, **samples** and **midi** show your library as a folder tree. Click a folder to open it in place. When you type in the search field, Ethereal searches the whole library at once. Folders whose names match come first, then the files. Click a matching folder to show it in the tree.

Next to the search field you can sort the results by **name**, **recent**, **duration** or **bpm**. While you're typing, results are sorted by **relevance**. The folder tree is always sorted by name. Rows show a sample's tempo, key and length when Ethereal could read them. The status line at the bottom counts the results and shows indexing progress while new folders are being read.

## previewing samples

Click a sample to hear it, and click it again to stop. From the search field, [[key:ArrowUp]] and [[key:ArrowDown]] move through the results and play each sample as you land on it. [[key:Enter]] plays or stops the highlighted one, and [[key:Escape]] clears the search.

Turn on **sync** at the bottom of the browser to preview samples in time with the project's tempo. Ethereal remembers this choice.

## dragging samples in

Drag a sample from the browser onto the arrangement:

- **onto an audio track**: an audio clip is created where you drop it, snapped to the grid. Hold [[key:Alt]] while dropping to place it without snapping.
- **onto the empty space below the tracks**: a new audio track is created with the clip on it.

Either way, the import, the new track and the clip are one undo step. While the file is read, a placeholder shows its progress in the lane. Samples can't be dropped on MIDI tracks.

You can also drag samples straight onto devices that play audio files: a **sampler**, a pad of a [drum rack](/docs/making-music/drum-rack), a key of a **multisampler**, or the impulse response of a **convolution reverb**.

Files from your computer can be dropped on the arrangement too. See [importing audio](/docs/import-export/importing-audio).

## midi files

MIDI files are listed under the **midi** chip, where you can search, star and tag them. Drag one onto a MIDI track, or below the tracks, to turn it into MIDI clips. See [midi files](/docs/import-export/midi-files).

## presets

The **presets** chip lists factory presets and the ones you saved. To load a preset:

1. Select a track that has a device of the preset's type (for example a **synth** for a synth preset).
2. Double-click the preset, or press [[key:Enter]] on it, or right-click it and choose **load on …**.

If the selected track has no matching device, the browser tells you to select a track with one. Presets can't be dragged onto tracks. To save presets, use the device's preset menu (see [devices and presets](/docs/making-music/devices-and-presets)).

## projects

The **projects** chip lists your Ethereal projects. Double-click one, or right-click it and choose **open project**, to open it.

## adding your own folders

The last row of the folder tree adds your own sample folders to the library:

- **add folder…** [[desktop]]: pick a folder on your computer. Ethereal indexes it where it is and doesn't copy the files. If you move or delete the folder, its samples disappear from the browser.
- **import folder…** [[web]]: pick a folder, and Ethereal copies its audio files into the library stored in your browser. Progress shows at the bottom of the browser.

You can also drop a folder from your computer onto the browser. On the desktop it's added in place, and on the web it's copied. Audio files dropped on the browser (not folders) are added to the open project without placing a clip.

Right-click one of your folders in the tree for:

- **rescan**: read the folder again after you add or remove files.
- **rename…**: change the name shown in the browser. Leave the field empty to go back to the folder's own name.
- **remove folder**: take it out of the library. Your files stay on disk.

> [!warning]
> On the web, the library lives in your browser's storage, like your projects. Clearing the site's data removes it. See [where your projects live](/docs/welcome/where-your-projects-live).

### what gets indexed

Ethereal reads sub-folders down to 16 levels deep. Hidden files and folders (names starting with a dot, like `.DS_Store` or `.git`) are skipped.

## packs

A sample pack inside one of your folders shows up as its own folder, under the folder it's in. A pack keeps its name exactly as it's written.

## favourites and tags

Click the star on a row to add the item to your favourites. The star next to the chips shows **favourites only**.

Right-click an item for more:

- **favourite** / **unfavourite**.
- **edit tags…**: type tags separated by commas, for example `drums, punchy, dark`.
- **show in folder** (samples and MIDI): jump to the item's folder in the tree.
- **filter by "…"**: show only items with that tag.

Tags also show on the rows. Click one to filter by it. The active tag filters sit under the chips, and clicking one removes it.

## choosing which files are listed

By default the browser lists only files Ethereal can use, and hides system and sidecar files. To change that, open **settings**, go to **general** › **library** and find **library files**:

- **show**: rules for the files to list. With no rules, every file is listed.
- **hide**: rules for files and folders to leave out. A hide rule wins over a show rule.

Rules are regular expressions matched against file names, ignoring case. For example, `\.wav$` matches WAV files. A rule that isn't valid is marked as you type, and nothing is saved until every rule is valid. The line under the lists says how many of your library's files the rules would show. **reset to defaults** brings back the original rules.

DAW projects (`.als`, `.flp`, `.rpp`, `.dawproject`) aren't opened from the browser.

## where next

- [Plugins](/docs/sounds-and-plugins/plugins): use your own instruments and effects.
- [Importing audio](/docs/import-export/importing-audio): bring audio files from your computer.
- [Clips](/docs/making-music/clips): move, trim and loop what you dragged in.
