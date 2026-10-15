---
title: modifier au toucher
order: 30
summary: Modifiez des notes dans le piano roll, dessinez l’automation et mixez avec des faders dans la bêta mobile.
platform: mobile
audience: everyone
covers:
  - mobile/clip-editor-and-piano-roll-on-touch #the-clip-editor
  - midi-editing/piano-roll-on-touch-select-and-draw-modes #the-piano-roll-on-touch
  - mobile/automation-on-touch #automation-on-touch
  - automation/automation-on-touch #automation-on-touch
  - mobile/the-mixer-view-and-fine-mode #the-mixer
---

Cette page couvre la modification à l’intérieur d’un clip, l’automation et le mixage dans la [bêta mobile](/docs/mobile/mobile-beta). Elle suppose que vous connaissez les gestes de base décrits dans [gestes tactiles](/docs/mobile/gestures) : toucher, toucher deux fois, appui long, et glisser un élément sélectionné.

## l’éditeur de clip {#the-clip-editor}

**Touchez deux fois un clip** (ou sélectionnez-le et touchez **modifier** dans la barre des clips) pour ouvrir son éditeur. Il s’ouvre dans un panneau par-dessus l’arrangement, entre la barre du haut et le transport, pour que vous puissiez lancer et arrêter la lecture pendant que vous modifiez. Sur un téléphone, il occupe toute la hauteur ; sur une tablette, la partie basse de l’écran.

- Un clip MIDI s’ouvre dans le piano roll, un clip audio dans l’éditeur de clip audio. Les onglets du haut passent d’un éditeur du clip à l’autre, comme sur un ordinateur.
- Fermez-le avec le bouton **fermer l’éditeur** (✕) ou le geste retour du système.

## le piano roll au toucher {#the-piano-roll-on-touch}

Le piano roll a deux modes, que l’on bascule avec le bouton **dessin** de sa barre d’outils. Un doigt ne peut pas à la fois sélectionner et dessiner sans touche de modification, alors c’est vous qui choisissez.

**Mode sélection** (par défaut) :

- **Touchez une note** pour la sélectionner. Touchez un espace vide pour désélectionner.
- **Touchez deux fois un espace vide** pour ajouter une note. **Touchez deux fois une note** pour la supprimer.
- **Glissez une note sélectionnée** pour la déplacer. Glissez son bord pour l’allonger ou la raccourcir.
- **Glissez sur un espace vide** pour faire défiler.
- **Appui long, puis glissez** sur un espace vide pour tracer un rectangle de sélection autour de plusieurs notes. Un appui long puis un glissement sur une note la déplace même si elle n’est pas sélectionnée.
- **Appui long sur une note** pour son menu.

**Mode dessin** :

- **Touchez** pour ajouter une note.
- **Glissez sur un espace vide** pour dessiner une note aussi longue que votre glissement.
- **Glissez une note** pour la déplacer.

Dans les deux modes :

- **Pincez avec les doigts côte à côte** pour zoomer dans le temps, et **l’un au-dessus de l’autre** pour agrandir ou réduire la hauteur des touches.
- **Touchez une touche** du clavier à gauche pour entendre cette note.

Sur un téléphone, les couloirs de vélocité et d’expression sont masqués, pour laisser la place aux notes. Touchez **vélocité** dans la barre du haut de l’éditeur pour les afficher. Tout le reste fonctionne comme décrit dans [le piano roll](/docs/making-music/piano-roll).

## l’automation au toucher {#automation-on-touch}

Pour voir l’automation d’une piste, touchez son en-tête pour la sélectionner, touchez-le de nouveau pour ouvrir la [fiche de piste](/docs/mobile/gestures#the-track-sheet), puis touchez **automation**. Les couloirs apparaissent sous la piste dans l’arrangement. Ensuite, dans un couloir :

- **Touchez deux fois** pour ajouter un point. **Touchez deux fois un point** pour le supprimer.
- **Appui long sur un point, puis glissez** pour le déplacer.
- **Appui long sur un espace vide du couloir, puis glissez** pour sélectionner une plage de temps dans le couloir.
- Un simple glissement fait défiler la vue, pour que vous ne déplaciez pas de points par accident.

Le rôle de l’automation et le reste de ses options sont expliqués dans [automation](/docs/making-music/automation).

## le mixeur {#the-mixer}

Le bouton à l’extrémité droite du transport bascule entre les pistes et le mixeur. Le mixeur affiche une tranche de console par piste, côte à côte (faites défiler horizontalement pour en voir plus), avec la tranche master fixée à droite. Chaque tranche comporte :

- le nom de la piste et son bouton de panoramique,
- un grand fader de volume avec l’indicateur de niveau de la piste, et le niveau en dB en dessous,
- **muet**, **solo** et **armer**.

Les pistes VCA n’ont pas de tranche, puisqu’elles ne transportent pas d’audio propre. Touchez de nouveau le même bouton pour revenir aux pistes.

### réglage fin {#fine-control}

Les faders et les boutons ne sautent jamais à l’endroit que vous touchez : ils partent de leur valeur actuelle à mesure que vous glissez. Pour de petits changements, faites glisser votre doigt sur le côté tout en glissant vers le haut ou le bas. Plus vous vous écartez sur le côté, plus la commande suit lentement : environ moitié moins vite à faible distance, puis au quart, puis au dixième.

Le même réglage fin fonctionne pour le fader de volume et le bouton de panoramique de la fiche de piste et des en-têtes de piste sur une tablette. **Touchez deux fois** un fader pour le remettre à 0 dB, ou le bouton de panoramique pour le recentrer.

## et ensuite {#where-next}

- [Le piano roll](/docs/making-music/piano-roll) : tout sur les notes, la quantification et la vélocité.
- [Mixage](/docs/making-music/mixing) : volume, panoramique, envois et groupes.
- [Ethereal sur votre téléphone](/docs/mobile/mobile-beta) : ce que la bêta peut et ne peut pas faire.
