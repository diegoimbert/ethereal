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

- **rendre sur une nouvelle piste** ([[action:clip.bounce]]) : fait le rendu des clips sélectionnés, avec les effets de la piste et leur queue (la réverbération et le délai résonnent jusqu’à 30 secondes), sur une nouvelle piste audio juste en dessous. La piste d’origine est mise en muet, pour que vous entendiez le rendu.
- **rendre sur place** (clips audio uniquement) : fait le rendu des clips sans les effets de la piste et les remplace par le résultat.
- **consolider** ([[action:clip.consolidate]]) : réunit en un seul clip les clips sélectionnés de chaque piste.

Avec une sélection temporelle, les mêmes raccourcis et son menu du clic droit agissent sur la plage :

- **rendre sur une nouvelle piste** fait le rendu de cette plage de chaque piste sélectionnée, en une seule étape.
- **consolider** réunit cette plage de chaque piste sélectionnée en un seul clip par piste, audio ou MIDI. Les parties des clips hors de la sélection restent telles quelles.

Les pistes où rien ne joue dans la plage sont ignorées.

Le rendu et la consolidation comptent chacun pour une étape d’annulation.

## convertir un instrument en échantillonneur {#converting-an-instrument-to-a-sampler}

Vous pouvez jouer un instrument terminé sans le faire tourner : faites un clic droit sur lui dans la chaîne d’appareils d’une piste (un rack d’instruments, un synthé), ou à l’intérieur d’un rack d’instruments, et choisissez **convertir en échantillonneur…**. Ethereal enregistre l’instrument note par note, puis le remplace à sa place par un multi-échantillonneur qui joue ces enregistrements. Les enregistrements sont l’instrument seul : les effets MIDI placés avant lui, les autres chaînes du rack et le volume de sa chaîne restent tels quels autour de l’échantillonneur, et les macros du rack qui règlent ses paramètres sont enregistrées telles qu’elles sont. L’enregistrement consomme beaucoup moins de processeur, mais c’est un instantané : les modifications du son d’origine ne l’atteignent pas, et une note tenue s’arrête à la fin de son échantillon.

La boîte de dialogue choisit ce qui est enregistré :

- **notes** : les touches à enregistrer. Choisissez un préréglage (chaque note, tous les quelques demi-tons, chaque octave), ou cliquez sur les touches du clavier pour les ajouter ou les retirer ; glissez sur plusieurs touches pour les ajouter ou les retirer d’un coup, ou glissez avec le bouton droit pour les retirer. Un préréglage s’étend de votre touche la plus grave à la plus aiguë (de do1 à do6 au départ), et modifier les touches fait passer le préréglage à **personnalisé**. Les notes entre deux enregistrements jouent le plus proche, transposé ; les notes au-delà utilisent l’enregistrement le plus proche du bord. Chaque note donne la meilleure qualité ; toutes les trois ou quatre notes va beaucoup plus vite.
- **vélocités** : la force avec laquelle chaque note est jouée à l’enregistrement. Un préréglage les répartit régulièrement jusqu’à 127 ; **configurer…** les liste pour en changer, en ajouter ou en retirer une (jusqu’à 16), ce qui fait passer le préréglage à **personnalisé**. Chaque vélocité jouée utilise la plus proche enregistrée. Une seule vélocité est la plus rapide et garde un jeu doux plus faible ; davantage conservent la réponse de l’instrument quand vous jouez doucement ou fort.
- **durée de la note** et **queue de relâchement** : combien de temps chaque note est tenue et quelle part de sa décroissance est gardée.

La boîte de dialogue indique combien d’échantillons ces réglages produisent. La conversion se fait comme un gel, un rendu à la fois, et affiche sa progression dans l’en-tête de la piste. Le résultat compte pour une seule étape d’annulation.

## et ensuite {#where-next}

- [les pistes](/docs/making-music/tracks) : les groupes et les menus des pistes.
- [les clips audio](/docs/making-music/audio-clips)
- [exporter l’audio](/docs/import-export/exporting-audio) : faites le rendu du morceau entier dans un fichier.
