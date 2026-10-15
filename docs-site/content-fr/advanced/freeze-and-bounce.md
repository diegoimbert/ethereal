---
title: geler, aplatir et faire un rendu
order: 70
summary: Rendez des pistes et des clips en audio pour économiser le processeur, figer un son ou réunir des clips.
platform: all
audience: advanced
covers:
  - clips/freeze-locks-clips-and-devices #freezing-a-track
  - clips/freezing-a-group-locks-its-tracks #freezing-a-group
  - clips/flatten-is-permanent #flatten
  - clips/bounce-to-new-track-bounce-in-place-consolidate #bounce-and-consolidate
---

Les instruments et les effets consomment du processeur pendant la lecture. Quand une piste est terminée, ou que votre ordinateur peine, vous pouvez la rendre en audio : Ethereal lit alors l’enregistrement au lieu de faire tourner ses appareils. Le gel s’annule à tout moment, l’aplatissement le rend définitif, et le rendu transforme une partie en nouveau clip.

## geler une piste {#freezing-a-track}

Faites un clic droit sur l’en-tête d’une piste et choisissez **geler la piste** (si plusieurs pistes sont sélectionnées, elles sont toutes gelées). Ethereal fait le rendu de la piste, puis lit ce rendu.

Tant qu’une piste est gelée :

- un flocon de neige apparaît dans son en-tête ;
- ses clips, ses appareils et l’automation de ses appareils sont verrouillés : vous ne pouvez pas les modifier ;
- son volume, son panoramique, muet, solo et ses envois fonctionnent toujours.

Pour la modifier à nouveau, cliquez sur le flocon (ou faites un clic droit sur l’en-tête et choisissez **dégeler la piste**). La piste lit de nouveau ses clips et ses appareils.

Les rendus se font un par un, en arrière-plan. L’en-tête de la piste affiche la progression : cliquez dessus pour annuler, ou choisissez **annuler le gel** dans le menu de la piste.

## geler un groupe {#freezing-a-group}

Une piste de groupe peut aussi être gelée : elle lit alors un seul rendu de tout ce qu’elle contient, avec les appareils du groupe lui-même. Tant que le groupe est gelé, les pistes qu’il contient affichent un cadenas. Vous ne pouvez pas les modifier, les déplacer, les mettre en solo ou en muet avant de dégeler le groupe, et leurs envois vers les pistes de retour s’arrêtent.

Un groupe ne peut pas être gelé si l’une de ses pistes envoie sa sortie hors du groupe.

## aplatir {#flatten}

**aplatir la piste** (dans le menu d’une piste gelée) rend le gel définitif : les clips et les appareils de la piste sont remplacés par un seul clip audio du rendu. Une confirmation est demandée, avec le nom des pistes qui seront aplaties.

- Une piste MIDI gelée devient une piste audio.
- Un groupe gelé devient une seule piste audio, et les pistes qu’il contenait sont supprimées.

> [!warning]
> Aplatir supprime les clips et les appareils. Seule l’annulation ([[action:edit.undo]]) les ramène : une fois que vous ne pouvez plus annuler, ils sont perdus. Utilisez le gel si vous risquez de vouloir changer la piste plus tard.

## rendu et consolidation {#bounce-and-consolidate}

Sélectionnez des clips, faites un clic droit sur l’un d’eux et choisissez :

- **rendre sur une nouvelle piste** : fait le rendu des clips sélectionnés, avec les effets de la piste et leur queue (la réverbération et le délai résonnent jusqu’à 30 secondes), sur une nouvelle piste audio juste en dessous. La piste d’origine est mise en muet, pour que vous entendiez le rendu.
- **rendre sur place** (clips audio uniquement) : fait le rendu des clips sans les effets de la piste et les remplace par le résultat.
- **consolider** : réunit en un seul clip les clips sélectionnés de chaque piste.

Avec une sélection temporelle, son menu du clic droit propose aussi **rendre sur une nouvelle piste** : le rendu porte sur cette plage de chaque piste sélectionnée, en une seule étape. Les pistes où rien ne joue dans la plage sont ignorées.

Le rendu et la consolidation comptent chacun pour une étape d’annulation.

## et ensuite {#where-next}

- [les pistes](/docs/making-music/tracks) : les groupes et les menus des pistes.
- [les clips audio](/docs/making-music/audio-clips)
- [exporter l’audio](/docs/import-export/exporting-audio) : faites le rendu du morceau entier dans un fichier.
