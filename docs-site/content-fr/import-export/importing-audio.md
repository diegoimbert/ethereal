---
title: importer de l’audio
order: 10
summary: Ajoutez des fichiers audio de votre ordinateur à un projet, avec le raccourci d’import ou en les déposant.
platform: all
audience: beginner
covers:
  - import-daw/import-audio-files-i-ctrl-i #import-audio
  - import-daw/dropping-audio-vs-project-files #dropping-files-on-the-window
  - import-daw/audio-import-limits-and-errors #limits-and-errors
---

Cette page montre comment faire entrer des fichiers audio de votre ordinateur dans le projet ouvert : des boucles, des enregistrements, les stems d’un ami. Pour utiliser les sons de votre bibliothèque d’échantillons, faites-les plutôt glisser depuis [le navigateur](/docs/sounds-and-plugins/browser).

## importer de l’audio… {#import-audio}

Appuyez sur [[action:file.import]], ou choisissez **importer de l’audio…** dans la palette de commandes ou dans le menu du clic droit de l’arrangement. Choisissez un ou plusieurs fichiers. Ethereal lit les fichiers WAV, AIFF, FLAC, MP3, OGG et M4A/AAC.

Où arrivent les fichiers :

- **si une piste audio est sélectionnée** : sur cette piste, à partir du marqueur d’insertion (là où vous avez cliqué en dernier dans l’arrangement, pas la tête de lecture), l’un après l’autre.
- **sinon** : chaque fichier va sur une nouvelle piste audio, tous à partir du marqueur d’insertion.

Tout l’import compte comme une seule étape d’annulation : [[action:edit.undo]] retire donc tous les fichiers qu’il a ajoutés.

> [!note]
> Sur le bureau, un fichier importé est utilisé là où il se trouve sur votre disque, il n’est pas copié dans le projet. Si vous le déplacez ou le supprimez, le clip apparaît comme manquant. Pour emporter un projet sur un autre ordinateur avec son audio, utilisez **exporter le projet…** en y incluant les médias. Voir [déplacer des projets](/docs/basics/moving-projects).

## déposer des fichiers sur la fenêtre {#dropping-files-on-the-window}

Vous pouvez aussi faire glisser des fichiers depuis le gestionnaire de fichiers de votre ordinateur :

- **des fichiers audio sur une piste audio** : les clips sont placés là où vous les déposez, l’un après l’autre.
- **des fichiers audio sous les pistes** : chacun reçoit une nouvelle piste audio.
- **des fichiers audio sur le navigateur** : ils sont ajoutés au projet sans placer de clip.
- **des fichiers MIDI** (`.mid`, `.midi`) sur une piste MIDI ou sous les pistes : ils deviennent des clips MIDI. Voir [les fichiers MIDI](/docs/import-export/midi-files).
- **un projet Ethereal** (`.ether`), déposé n’importe où : il est importé et ouvert comme nouveau projet.

## limites et erreurs {#limits-and-errors}

Ethereal vérifie chaque fichier avant de le lire. Les fichiers de plus de 1 Go, les fichiers vides et les fichiers d’autres types sont refusés, avec la raison (« use WAV, AIFF, FLAC, MP3 or OGG », « too large », « the file is empty »).

Chaque fichier en cours d’import affiche une ligne avec sa progression en bas de la fenêtre. Si un fichier échoue, les autres sont quand même importés, et celui qui a échoué reste dans la liste avec son erreur jusqu’à ce que vous le fermiez. Vous pouvez annuler un fichier tant qu’il est encore en cours de lecture. Une fois son audio dans le projet, l’import ne peut plus être annulé : utilisez plutôt l’annulation.

## et ensuite {#where-next}

- [Les clips](/docs/making-music/clips) : déplacez, rognez et bouclez votre audio.
- [Les clips audio](/docs/making-music/audio-clips) : fondus, warp et inversion.
