---
title: exporter vers d’autres daw
order: 40
summary: Enregistrez votre projet en DAWproject, en set Ableton Live ou en projet FL Studio, et sachez ce qui ne fera pas le voyage.
platform: all
audience: everyone
covers:
  - export/export-to-another-daw #exporting-a-project
  - export/als-and-flp-exports-are-untested-in-live-fl #which-format-to-choose
  - export/what-other-daws-can-t-receive #what-other-daws-cant-receive
  - export/fl-studio-may-ask-to-locate-samples #fl-studio-and-its-samples
---

Ethereal peut écrire votre projet sous forme de fichier de projet d’une autre DAW, pour terminer un morceau ailleurs ou le confier à quelqu’un qui n’utilise pas Ethereal. Cette page présente les formats, lequel choisir, et ce qui se perd en route. Pour partager seulement le son, [exportez l’audio](/docs/import-export/exporting-audio) à la place.

## exporter un projet {#exporting-a-project}

Ouvrez le menu du projet (en haut à gauche) et choisissez **exporter vers une autre DAW**, puis l’un de ces formats :

- **DAWproject (.dawproject)** : s’ouvre dans Bitwig Studio, Studio One, Cubase, Nuendo, Cubasis, VST Live, n-Track Studio et Waveform.
- **Ableton Live Set (.als)** : un set Live 11.
- **FL Studio Project (.flp)** : un projet FL Studio 20.

Les mêmes choix se trouvent dans la palette de commandes sous **exporter vers …**.

- **Bureau** [[desktop]] : Ethereal vous demande où enregistrer le fichier. Pour Live et FL Studio, les fichiers audio sont écrits dans un dossier `Samples` à côté.
- **Web** [[web]] : le fichier se télécharge. Les sets Live et les projets FL Studio se téléchargent sous forme de `.zip` qui contient le projet et son dossier `Samples` : décompressez-le avant de l’ouvrir.

À la fin, un rapport indique ce qui a été exporté, puis les éléments **exportés approximativement** et les éléments **non exportés**.

## quel format choisir {#which-format-to-choose}

> [!warning]
> Les exports Ableton Live et FL Studio n’ont pas encore été testés dans Live ou FL Studio. Ethereal les vérifie en les réimportant, mais un fichier peut encore refuser de s’ouvrir. DAWproject est la voie la plus fiable.

Si votre DAW lit le DAWproject, utilisez-le. Ableton Live et FL Studio ne savent pas lire le DAWproject : pour eux, utilisez leurs propres formats.

## ce que les autres daw ne peuvent pas recevoir {#what-other-daws-cant-receive}

Certaines choses n’existent que dans Ethereal : elles sont donc laissées de côté ou simplifiées. Le rapport les liste une par une.

- **Les appareils intégrés d’Ethereal** (delay, réverbération, sampler, racks…) ne sont pas exportés vers Live ni FL Studio. En DAWproject, seuls l’EQ, le compresseur, le gate et le limiteur survivent, sous forme d’appareils génériques du format.
- **Les sidechains et les modulateurs** sont supprimés dans tous les formats.
- **L’audio inversé** est lu à l’endroit en DAWproject et dans Live.
- **Les couloirs de prises** n’arrivent pas en DAWproject : seuls les clips qui jouent sont exportés. Live les reçoit.
- **FL Studio** ne reçoit que les plug-ins VST2 (pas de VST3, CLAP ni AU), et ni automation, ni changements de tempo, ni fondus.
- **Les plug-ins CLAP et AU** ne sont pas exportés vers Live.

## fl studio et ses échantillons {#fl-studio-and-its-samples}

L’export FL Studio copie votre audio dans un dossier `Samples` à côté du `.flp`. FL Studio peut quand même vous demander où se trouvent les fichiers : indiquez-lui ce dossier. Gardez le dossier `Samples` à côté du projet chaque fois que vous le déplacez.

## et ensuite {#where-next}

- [Les projets d’autres DAW](/docs/import-export/other-daws) : ouvrez leurs projets dans Ethereal.
- [Exporter l’audio](/docs/import-export/exporting-audio) : un mix ou des stems.
- [Déplacer des projets](/docs/basics/moving-projects) : emportez un projet Ethereal sur un autre ordinateur.
