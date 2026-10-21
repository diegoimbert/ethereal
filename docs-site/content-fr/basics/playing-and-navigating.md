---
title: lire et se déplacer
order: 20
summary: Lancez et arrêtez la lecture de votre morceau, comprenez la différence entre la tête de lecture et le marqueur d’insertion, bouclez une section, et zoomez et faites défiler l’arrangement.
platform: all
audience: beginner
covers:
  - transport-tempo/play-stop-buttons-and-what-stop-does-twice #the-play-and-stop-buttons
  - transport-tempo/space-returns-to-where-you-started-shift-space-stops-in-plac #space-and-shiftspace
  - transport-tempo/space-works-everywhere-except-text-fields #space-works-everywhere
  - transport-tempo/record-loop-and-metronome-have-no-default-shortcut #record-loop-and-metronome-keys
  - transport-tempo/ruler-click-to-locate-and-scrub #the-ruler-moves-the-playhead
  - arrangement/click-on-a-lane-places-the-insert-marker-not-the-playhead #the-insert-marker
  - transport-tempo/follow-playhead-is-off-by-default #follow-the-playhead
  - transport-tempo/loop-brace-move-resize-toggle-draw-a-new-one #looping-a-section
  - arrangement/zoom-and-scroll-with-wheel-trackpad-middle-button #zoom-and-scroll
  - arrangement/zoom-out-stops-past-the-song-end #how-far-you-can-zoom-out
  - transport-tempo/position-display-before-zero-count-in #the-position-display
  - transport-tempo/metronome-settings-and-count-in-click #the-metronome
  - transport-tempo/count-in-and-punch-in-out #count-in-and-punch
---

Cette page présente le transport (lecture, arrêt, boucle, métronome) et la façon de se déplacer dans l’arrangement. Mieux vaut avoir un projet ouvert avec un clip ou deux. Si vous n’en avez pas encore, commencez par [votre première boucle](/docs/welcome/first-beat).

## les boutons lecture et arrêt {#the-play-and-stop-buttons}

Le transport se trouve au milieu de la barre du haut.

- **Lecture** (▶) lance la lecture. Pendant la lecture, il devient un bouton pause : appuyez dessus pour vous arrêter là où vous êtes.
- **Arrêt** (■) arrête la lecture. Appuyez de nouveau dessus à l’arrêt et la tête de lecture revient à votre [marqueur d’insertion](#the-insert-marker) ou à votre sélection. Un troisième appui la ramène au début du morceau.

## espace et maj+espace {#space-and-shiftspace}

- [[action:transport.play]] lance la lecture. Appuyé de nouveau, il arrête et revient là où vous étiez : au marqueur d’insertion ou au début de la sélection temporelle, sinon à l’endroit où la lecture a commencé. Il est ainsi facile d’écouter le même passage encore et encore.
- [[action:transport.stopToSelection]] lance aussi la lecture, mais appuyé pendant la lecture, il arrête **sur place**, en laissant la tête de lecture où elle est.

Les deux peuvent être modifiés dans les raccourcis clavier (voir [raccourcis clavier](/docs/reference/keyboard-shortcuts)).

### espace fonctionne partout {#space-works-everywhere}

Espace lance et arrête toujours la lecture, où que soit le focus : sur un bouton, une case à cocher, un menu, un curseur, un bouton rotatif, une boîte de dialogue ou la fenêtre d’un plug-in. Il n’« appuie » jamais sur le bouton qui a le focus. Seule exception : un champ de texte, où espace tape une espace.

## touches enregistrer, boucle et métronome {#record-loop-and-metronome-keys}

Dans le preset de touches par défaut **Ethereal**, **enregistrer**, **boucle** et **métronome** n’ont pas de touche : utilisez leurs boutons dans la barre du haut, ou la [palette de commandes](/docs/basics/command-palette-and-settings). Le preset **ableton-like** ajoute des touches pour deux d’entre eux : enregistrer [[action:transport.record]] et boucle [[action:transport.loop]]. Vous pouvez attribuer à chacun votre propre touche dans **raccourcis clavier…** (dans la palette de commandes, ou dans **réglages › souris**).

## la tête de lecture et le marqueur d’insertion {#the-insert-marker}

Ethereal a deux curseurs différents, comme Ableton Live :

- La **tête de lecture** indique où en est la lecture, ou d’où elle partira. C’est la ligne qui avance pendant que le morceau joue.
- Le **marqueur d’insertion** indique où vont vos modifications. Un simple clic sur une partie vide d’une piste, ou sur le corps d’un clip, y place un marqueur d’insertion clignotant, sur cette piste. Il s’aligne sur la grille ; maintenez [[key:Alt]] en cliquant pour le placer librement.

Coller, couper, insérer et les autres modifications s’appliquent au marqueur d’insertion (ou dans votre sélection temporelle), **jamais à la tête de lecture**.

À l’arrêt, placer le marqueur d’insertion y déplace aussi la tête de lecture, pour que la lecture parte de là où vous avez cliqué. Pendant la lecture, un clic ne déplace jamais la tête de lecture : il ne prend effet qu’au prochain arrêt.

### la règle déplace la tête de lecture {#the-ruler-moves-the-playhead}

Pour déplacer la tête de lecture elle-même, utilisez la **règle** en haut de l’arrangement. Appuyez dessus pour y placer la tête de lecture, et faites glisser pour parcourir le morceau. Elle s’aligne sur la grille ([[key:Alt]] : librement). Cliquer sur la règle ne déplace pas votre marqueur d’insertion.

## suivre la tête de lecture {#follow-the-playhead}

Par défaut, la vue reste immobile pendant la lecture, et la tête de lecture peut sortir de l’écran. Pour que la vue la suive, activez le bouton **suivre** (le réticule) dans la barre d’outils du coin de l’arrangement. Avec le suivi activé, la tête de lecture traverse la vue jusqu’à environ 40 % de sa largeur, puis la vue défile en douceur pour l’y maintenir.

Sur un écran tactile, faire glisser la vue désactive le suivi.

## boucler une section {#looping-a-section}

L’**accolade de boucle** est la barre le long de la règle qui délimite la zone de boucle. Quand la boucle est activée, la lecture répète cette zone.

- Activez ou désactivez la boucle avec le bouton **boucle** de la barre du haut, ou double-cliquez sur l’accolade.
- Faites glisser l’accolade pour la déplacer, et ses bords pour la redimensionner.
- Faites glisser avec [[key:Shift]] sur la règle pour dessiner une nouvelle zone de boucle.

Tout cela s’aligne sur la grille ; maintenez [[key:Alt]] pour déplacer librement. Chaque modification de la boucle est une étape d’annulation.

## zoom et défilement {#zoom-and-scroll}

Avec une souris ou un pavé tactile, dans l’arrangement :

| pour | faites |
|---|---|
| faire défiler les pistes vers le haut et le bas | la molette |
| faire défiler le temps sur le côté | [[key:Shift]] + molette, ou un balayage latéral sur le pavé tactile |
| zoomer et dézoomer dans le temps | [[key:Mod]] + molette, ou un pincement sur le pavé tactile |
| agrandir ou réduire la hauteur de toutes les pistes | [[key:Mod+Shift]] + molette |
| se déplacer dans toutes les directions | faire glisser avec le bouton du milieu |

Un pincement zoome toujours. Sous Windows, le sens du zoom est inversé par défaut, pour que pousser la molette vers l’avant zoome comme sur un Mac.

Tout cela se règle dans **réglages › souris et clavier** : la sensibilité du zoom et du défilement, le modificateur qui zoome (ou si la molette seule zoome), l’inversion de chaque sens, le rôle du bouton du milieu et celui des boutons précédent et suivant de la souris. La palette de commandes propose **réglages de la souris et de la molette…** pour y aller vite, et **ajuster le zoom** pour afficher tout le morceau.

### jusqu’où vous pouvez dézoomer {#how-far-you-can-zoom-out}

Le dézoom s’arrête un peu après la fin de votre morceau (le dernier clip ou marqueur), et jamais en deçà de 32 mesures en 4/4. Dans un projet vide, vous ne pouvez donc pas dézoomer beaucoup : il n’y a rien à voir.

## l’affichage de la position {#the-position-display}

Entre les boutons du transport et le tempo, la barre du haut indique deux fois où se trouve la tête de lecture :

- en **mesures.temps.double-croches**, selon la signature rythmique (par exemple 5.3.1) ;
- en **minutes:secondes**.

Pendant le décompte d’un enregistrement, les mesures sont comptées à rebours depuis avant le début : une mesure avant 1.1.1, c’est -1.1.1 (il n’y a pas de mesure 0).

## le métronome {#the-metronome}

Le bouton **métronome** de la barre du haut active et désactive le clic. Le petit bouton à côté du transport ouvre les **réglages du métronome** : le volume du clic, l’accentuation ou non du premier temps de chaque mesure, et son son (**classique**, **bois** ou **bip**). Chaque modification est une étape d’annulation, et les réglages sont enregistrés avec le projet.

Quand vous enregistrez avec un décompte, le clic joue pendant le décompte même si le métronome est désactivé.

## décompte et punch {#count-in-and-punch}

Les commandes d’enregistrement de la barre du haut proposent un **décompte** de 0, 1, 2 ou 4 mesures avant le début de l’enregistrement, et un bouton **punch in/out** qui n’enregistre qu’à l’intérieur de la zone de boucle. Elles sont expliquées dans [l’enregistrement](/docs/making-music/recording).

## et ensuite {#where-next}

- [Tempo, signature rythmique et gamme](/docs/basics/tempo-and-time-signature)
- [Projets et enregistrement](/docs/basics/projects)
- [Raccourcis clavier](/docs/reference/keyboard-shortcuts)
