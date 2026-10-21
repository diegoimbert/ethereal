---
title: tempo, signature rythmique et gamme
order: 30
summary: Réglez le tempo et la signature rythmique, changez-les au cours du morceau, et choisissez la gamme du projet.
platform: all
audience: beginner
covers:
  - transport-tempo/tempo-field-type-drag-arrows-shift-for-fine #setting-the-tempo
  - transport-tempo/the-tempo-field-edits-the-tempo-at-the-playhead-not-the-whol #songs-with-tempo-changes
  - transport-tempo/tap-tempo #tap-tempo
  - transport-tempo/time-signature-field-is-per-position #the-time-signature
  - transport-tempo/tempo-and-time-signature-changes-on-the-ruler-marker-lane #changes-during-the-song
  - transport-tempo/tempo-automation-lane-on-the-master-track #the-tempo-lane
  - transport-tempo/a-time-signature-change-mid-bar-cuts-that-bar-short #a-change-in-the-middle-of-a-bar
  - transport-tempo/project-scale-is-a-visual-guide-only #the-project-scale
---

Cette page montre comment régler le tempo et la signature rythmique de votre morceau, comment les changer en cours de route, et à quoi sert la gamme du projet. Les champs se trouvent au milieu de la barre du haut, à côté de l’affichage de la position.

## régler le tempo {#setting-the-tempo}

Le champ **bpm** affiche le tempo en battements par minute. Vous pouvez le changer de trois façons :

- **Tapez** une valeur et appuyez sur [[key:Enter]].
- **Faites glisser** vers le haut ou le bas sur le champ : un bpm par pas. Maintenez [[key:Shift]] pour des pas fins de 0,01 bpm. Un glissement entier est une seule étape d’annulation.
- Appuyez sur [[key:ArrowUp]] ou [[key:ArrowDown]] quand le champ a le focus : un bpm par appui, ou 0,1 avec [[key:Shift]].

Le tempo va de 20 à 999 bpm.

### tap tempo {#tap-tempo}

Cliquez sur **tap** en rythme, au moins deux fois, et Ethereal règle le tempo d’après le temps moyen entre vos clics. Arrêtez-vous plus de 2 secondes et le clic suivant commence un nouveau comptage. Tous les clics d’un même comptage forment une seule étape d’annulation.

### morceaux avec des changements de tempo {#songs-with-tempo-changes}

Si votre morceau contient des changements de tempo (voir [plus bas](#changes-during-the-song)), le champ bpm affiche et modifie le tempo **à la tête de lecture**, et non le premier tempo du morceau. Placez la tête de lecture dans la partie à modifier avant de taper.

## la signature rythmique {#the-time-signature}

Le champ de signature rythmique, à côté du tempo, fonctionne de la même façon : il affiche et modifie la signature **à la tête de lecture**. Tapez une valeur comme `7/8` et appuyez sur [[key:Enter]].

Pour changer de mesure en cours de morceau, ajoutez plutôt un changement de signature rythmique sur la règle.

## changements au cours du morceau {#changes-during-the-song}

Faites un clic droit sur la règle (ses numéros ou la bande des marqueurs au-dessus : elles partagent un menu, avec aussi **ajouter un marqueur ici**) à l’endroit du changement et choisissez **ajouter un changement de tempo ici** ou **ajouter un changement de signature rythmique ici**. Les changements apparaissent sous forme de petites pastilles le long du bord inférieur de la règle : celles de tempo se lisent comme ♩120, celles de signature comme 3/4.

- **Faites glisser** une pastille pour la déplacer. Elle s’aligne sur la grille ; maintenez [[key:Alt]] pour la déplacer librement.
- **Faites un clic droit** sur une pastille de tempo pour basculer entre **rampe jusqu’au point suivant** (le tempo glisse jusqu’au changement suivant) et **tempo constant (sans rampe)** (il saute), ou pour la supprimer.
- **Faites un clic droit** sur une pastille de signature pour choisir une signature courante ou pour la supprimer.

Le tempo et la signature de la mesure 1 sont toujours présents : vous pouvez changer leurs valeurs, mais pas les déplacer ni les supprimer.

### un changement au milieu d’une mesure {#a-change-in-the-middle-of-a-bar}

Un changement de signature rythmique peut se placer sur n’importe quel temps, pas seulement sur une barre de mesure. Si vous en mettez un à l’intérieur d’une mesure, cette mesure se termine plus tôt et la mesure 1 de la nouvelle signature commence au changement.

## le couloir de tempo {#the-tempo-lane}

Pour un travail plus fin sur le tempo, ouvrez le couloir **tempo** de la piste master (c’est le premier couloir d’automation de la piste master ; voir [l’automation](/docs/making-music/automation)). Il dessine le tempo sous forme de courbe, avec une poignée pour chaque point de tempo :

- **Double-cliquez** sur le couloir pour ajouter un point, et double-cliquez sur un point pour le retirer.
- **Faites glisser** un point pour changer sa position et son bpm. Maintenez [[key:Shift]] pour des pas fins de 0,01 bpm, et [[key:Alt]] pour un placement précis : hors de la grille, dix fois moins que la souris. Le point du début ne change que son bpm. [[key:Escape]] pendant le glissement remet les points en place et ne laisse rien dans l’historique d’annulation.
- **Faites un clic droit** sur un point pour activer ou désactiver **rampe jusqu’au point suivant**, ou pour le supprimer.
- Sélectionnez un point et appuyez sur [[key:Delete]] ou [[key:Backspace]] pour le retirer.

## la gamme du projet {#the-project-scale}

Chaque projet a une **gamme** (une tonique et un type, comme do mineur). Vous la réglez dans le menu du projet (☰), sous **gamme du projet**. Le [piano roll](/docs/making-music/piano-roll) de chaque piste MIDI l’utilise par défaut pour mettre en évidence les notes qui font partie de la gamme.

La gamme n’est qu’un repère visuel. Elle ne vous empêche pas de jouer ou de dessiner d’autres notes, et ne corrige rien.

## et ensuite {#where-next}

- [Lire et se déplacer](/docs/basics/playing-and-navigating)
- [Le piano roll](/docs/making-music/piano-roll)
- [L’automation](/docs/making-music/automation)
