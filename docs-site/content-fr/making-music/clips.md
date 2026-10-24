---
title: clips
order: 20
summary: Créez, sélectionnez, déplacez, copiez, raccourcissez, scindez, bouclez et renommez des clips dans l’arrangement.
platform: all
audience: beginner
covers:
  - clips/grab-clips-by-their-title-bar #moving-a-clip
  - clips/cmd-ctrl-drag-copies-not-alt #copying-while-dragging
  - clips/clip-selection-modifiers #selecting-clips
  - clips/click-opens-the-clip-editor #opening-a-clip
  - clips/double-click-expands-the-editor-and-inspector #double-clicking-a-clip
  - clips/moving-onto-other-clips-trims-or-deletes-them #dropping-onto-other-clips
  - clips/moving-clips-between-tracks-and-to-new-tracks #moving-to-another-track
  - clips/trimming-limits-and-leading-gaps #trimming
  - clips/stretch-edge-cmd-ctrl-shift-drag #stretching
  - clips/slip-edit-cmd-ctrl-shift-drag #slip-editing
  - clips/create-midi-clips-with-double-click #making-a-midi-clip
  - clips/drag-and-drop-audio #adding-audio
  - clips/split-duplicate-loop-reverse-shortcuts #split-duplicate-loop-and-reverse
  - clips/paste-goes-to-the-insert-marker-and-tiles #copy-and-paste
  - clips/rename-clips #renaming-a-clip
  - clips/clip-and-track-colors #clip-colors
  - clips/editing-several-clips-in-the-inspector #editing-several-clips-at-once
---

Les clips sont les blocs de musique posés sur une piste : les clips MIDI contiennent des notes, les clips audio jouent un morceau d’un fichier audio. Cette page couvre les modifications communes à tous les clips. Pour les fondus, le warp et les autres modifications propres à l’audio, voir [clips audio](/docs/making-music/audio-clips).

## créer un clip midi {#making-a-midi-clip}

Double-cliquez sur un endroit vide du couloir d’une piste MIDI pour y créer un clip d’une mesure. Pour choisir sa longueur, double-cliquez en gardant le bouton enfoncé au second clic, puis faites glisser : le clip couvre les cases de la grille que vous survolez. Maintenez [[key:Alt]] pour ignorer la grille.

## ajouter de l’audio {#adding-audio}

Faites glisser des fichiers audio depuis votre ordinateur, ou des échantillons depuis le navigateur, sur le couloir d’une piste audio. Les clips commencent là où vous les déposez. Plusieurs fichiers se placent les uns après les autres. Déposez sous la dernière piste pour obtenir une nouvelle piste audio par fichier. Les pistes MIDI n’acceptent pas l’audio. Pendant l’import d’un fichier, un emplacement **import… %** montre sa progression. Maintenez [[key:Alt]] en déposant pour ignorer la grille.

Vous pouvez aussi utiliser **importer de l’audio…** ([[action:file.import]]). Voir [importer de l’audio](/docs/import-export/importing-audio).

## sélectionner des clips {#selecting-clips}

- Cliquez sur un clip pour le sélectionner (sa piste est sélectionnée aussi).
- Un [[key:Shift]]-clic ajoute un clip à la sélection.
- Un [[key:Mod]]-clic ajoute ou retire un clip.
- Un simple clic sur un clip déjà sélectionné ne garde que celui-ci.
- Faites glisser sur l’espace vide d’un couloir pour tracer un cadre : tous les clips qu’il touche sont sélectionnés.

Cliquer sur un clip place aussi le marqueur d’insertion à son début. Quand la lecture est arrêtée, la tête de lecture s’y déplace aussi, si bien que **lecture** démarre depuis le clip.

## ouvrir un clip {#opening-a-clip}

Cliquer sur un clip MIDI l’ouvre dans le piano roll en bas de la fenêtre. Cliquer sur un clip audio (sans glisser et sans touche de modification) ouvre son éditeur, avec les réglages de warp et les enveloppes. Cliquer ailleurs referme l’éditeur, sauf si vous l’avez épinglé. Double-cliquer sur le corps d’un clip l’ouvre aussi.

### double-cliquer sur un clip {#double-clicking-a-clip}

Double-cliquer sur un clip (sa barre de titre ou son corps, ou le clip d’une prise dans un couloir de prises) l’ouvre dans son éditeur, et déplie aussi le tiroir d’édition et l’inspecteur si vous les aviez réduits à une barre. Chacun revient à la taille que vous lui aviez donnée. Un panneau déjà ouvert garde sa taille. Vous pouvez les réduire de nouveau avec leur **×** : un simple clic sur un clip les laisse réduits.

C’est une bascule : quand le tiroir d’édition et l’inspecteur sont tous deux dépliés, double-cliquer sur le clip déjà ouvert les réduit tous les deux. Chacun garde sa taille pour la prochaine fois, et le clip reste sélectionné et ouvert : un nouveau double-clic dessus les fait revenir. Double-cliquer sur un autre clip affiche plutôt ce clip dans l’éditeur et laisse les deux panneaux ouverts.

Dans une petite fenêtre, un panneau déplié est rétréci si besoin pour que l’arrangement garde sa taille minimale, la même limite que quand vous faites glisser le bord d’un panneau.

## déplacer un clip {#moving-a-clip}

Saisissez un clip par sa barre de titre, la bande colorée en haut, et faites-le glisser. Le reste du clip (son corps) laisse passer les clics vers le couloir : faire glisser le corps trace donc un cadre de sélection au lieu de déplacer le clip. C’est ce qui vous permet de placer le marqueur d’insertion ou d’atteindre les poignées de fondu à l’intérieur d’un clip.

Les déplacements se calent sur la grille. Maintenez [[key:Alt]] pendant le glissement pour déplacer librement. [[key:Escape]] annule un glissement en cours. Chaque glissement est une seule étape d’annulation.

### copier en faisant glisser {#copying-while-dragging}

Maintenez [[key:Mod]] au moment de relâcher pour déposer une copie et laisser l’original en place. Vous pouvez appuyer sur [[key:Mod]] ou le relâcher pendant le glissement pour passer du déplacement à la copie. [[key:Alt]] ne copie pas dans Ethereal : il désactive le magnétisme.

### déplacer vers une autre piste {#moving-to-another-track}

Faites glisser vers le haut ou le bas pour déplacer des clips vers une autre piste du même type : les clips audio vers des pistes audio, les clips MIDI vers des pistes MIDI. Si l’un des clips sélectionnés ne peut pas aller là où vous pointez, ils restent tous sur leurs pistes.

Faites glisser sous la dernière piste pour créer de nouvelles pistes pour les clips. Des couloirs fantômes montrent les pistes qui seront créées. Les nouvelles pistes et le déplacement forment une seule étape d’annulation.

### déposer sur d’autres clips {#dropping-onto-other-clips}

Un clip qui arrive sur d’autres clips de la même piste se fait de la place. Cela se produit que vous le déposiez, le colliez ou le redimensionniez. Les clips qu’il recouvre entièrement sont supprimés. Les clips qu’il recouvre en partie sont raccourcis. Un clip à l’intérieur duquel il arrive est scindé autour de lui. Les fondus enchaînés sont conservés.

## raccourcir et rallonger {#trimming}

Faites glisser le bord gauche ou droit d’un clip pour le raccourcir ou le rallonger. Ces modifications se calent sur la grille. Maintenez [[key:Alt]] pour les faire librement.

- Un clip audio qui ne boucle pas ne peut pas dépasser la fin de son fichier. Avec la boucle activée, il se répète aussi loin que vous faites glisser.
- Faire glisser le début vers la gauche au-delà du début du contenu laisse un espace vide (du silence, ou aucune note) avant lui. Faites glisser de nouveau vers la droite pour supprimer cet espace.
- Le clip le plus court dure un seizième de temps.

### étirer {#stretching}

Maintenez [[key:Mod+Shift]] en faisant glisser un bord pour étirer le clip à la place : son contenu est joué plus vite ou plus lentement pour remplir la nouvelle longueur. Les notes, l’expression et la boucle d’un clip MIDI suivent. Un clip audio est warpé pour jouer son audio sur la nouvelle longueur (un clip non warpé est d’abord warpé), et ses fondus et enveloppes suivent aussi. En mode repitch, la hauteur suit la vitesse. Avec plusieurs clips sélectionnés, tous sont étirés, en une seule étape d’annulation.

## édition par glissement du contenu {#slip-editing}

Cette édition déplace ce qui joue à l’intérieur d’un clip sans déplacer le clip lui-même. Maintenez [[key:Mod+Shift]] et faites glisser le corps du clip. Le contenu se cale sur la grille ([[key:Alt]] : librement), et [[key:Escape]] annule. Si plusieurs clips sont sélectionnés, le contenu de chacun glisse.

Le contenu s’arrête aux extrémités du fichier audio (pour le MIDI, à la dernière note). Un clip qui montre déjà tout le fichier n’a donc rien à faire glisser.

## scinder, dupliquer, boucler et inverser {#split-duplicate-loop-and-reverse}

Ces raccourcis agissent sur les clips sélectionnés quand l’arrangement a le focus :

| action | raccourci |
|---|---|
| scinder au marqueur d’insertion (ou aux bords d’une sélection temporelle) | [[action:edit.split]] |
| dupliquer juste après le clip | [[action:edit.duplicate]] |
| activer / désactiver la boucle | [[action:clip.toggleLoop]] |
| inverser | [[action:clip.reverse]] |
| supprimer | [[action:edit.delete]] |

Le menu contextuel du clip propose les mêmes actions, plus **muet**, **renommer** et, pour les clips MIDI, **ouvrir dans le piano roll**. L’inversion est expliquée dans [clips audio](/docs/making-music/audio-clips#reverse).

Un clip en boucle affiche une icône de boucle dans sa barre de titre. Étirez-le vers la droite pour répéter son contenu.

## copier et coller {#copy-and-paste}

[[action:edit.copy]], [[action:edit.cut]] et [[action:edit.paste]] agissent sur les clips sélectionnés. Les clips collés commencent au marqueur d’insertion (ou là où vous avez fait un clic droit) et gardent leurs écarts. Le marqueur se déplace ensuite à leur fin, si bien que coller de nouveau prolonge le motif.

Les clips sont collés sur leurs propres pistes. S’ils viennent tous d’une même piste et que vous sélectionnez une autre piste du même type, ils y vont à la place.

Si vous avez fait une sélection temporelle, ces raccourcis agissent sur cette section plutôt que sur des clips entiers. Voir [arrangement](/docs/making-music/arranging#edit-keys-and-the-time-selection).

## renommer un clip {#renaming-a-clip}

Faites un clic droit sur un clip et choisissez **renommer**. Tapez dans sa barre de titre, puis appuyez sur [[key:Enter]] ou cliquez ailleurs. [[key:Escape]] annule. Si vous effacez le nom, le clip reprend son nom par défaut (un clip audio affiche le nom de son échantillon). Un nouveau clip MIDI n’a pas de nom à lui : il affiche le nom de sa piste, et le suit quand vous renommez la piste. **renommer** n’est proposé que quand un seul clip est sélectionné. L’inspecteur a aussi un champ de nom pour le clip sélectionné.

## couleurs des clips {#clip-colors}

Sélectionnez un ou plusieurs clips et choisissez une couleur dans l’inspecteur. La première pastille, **utiliser la couleur de la piste**, fait de nouveau suivre aux clips la couleur de leur piste. Pour changer la couleur d’une piste, voir [pistes](/docs/making-music/tracks#colors).

## modifier plusieurs clips à la fois {#editing-several-clips-at-once}

Sélectionnez plusieurs clips pour les modifier ensemble dans l’inspecteur : les rendre muets, leur donner une couleur et, pour les clips audio, régler le gain, la transposition, l’inversion et le warp.

- Taper une valeur l’applique à chaque clip.
- Faire glisser un champ ou le modifier avec les flèches déplace chaque clip de la même quantité, si bien que leurs écarts sont conservés.
- Un champ dont les clips ont des valeurs différentes les affiche comme mixtes.

## et ensuite {#where-next}

- [clips audio](/docs/making-music/audio-clips) : fondus, fondus enchaînés, gain, inversion et warp.
- [le piano roll](/docs/making-music/piano-roll) : modifiez les notes d’un clip MIDI.
- [arrangement](/docs/making-music/arranging) : modifiez des sections sur plusieurs pistes.
