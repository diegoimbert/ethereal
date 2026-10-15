---
title: le piano roll
order: 30
summary: Dessinez, sélectionnez, déplacez et quantifiez des notes, modifiez la vélocité, et travaillez avec les gammes, les sections et plusieurs clips à la fois.
platform: all
audience: beginner
covers:
  - midi-editing/draw-a-note-double-click-empty-space-or-turn-on-draw-mode #drawing-notes
  - midi-editing/new-notes-are-one-grid-step-long #note-length-and-the-grid
  - midi-editing/delete-a-note-by-double-clicking-it #deleting-notes
  - midi-editing/move-resize-and-transpose-by-dragging #moving-resizing-and-transposing
  - midi-editing/pressing-a-note-plays-it-the-headphones-toggle #hearing-notes-as-you-edit
  - midi-editing/selecting-notes-click-shift-cmd-ctrl-marquee-select-all #selecting-notes
  - midi-editing/click-a-piano-key-to-select-all-notes-of-that-pitch #selecting-every-note-of-one-pitch
  - midi-editing/alt-option-drag-a-note-to-change-its-velocity #velocity
  - midi-editing/cmd-ctrl-drag-duplicates-notes-and-you-can-switch-mid-drag #duplicating-notes
  - midi-editing/turning-snapping-off-shift-when-moving-alt-for-everything-el #turning-snapping-off
  - midi-editing/shift-drag-on-empty-space-makes-a-time-section #sections
  - midi-editing/with-a-section-copy-cut-paste-duplicate-use-its-exact-length #editing-with-a-section
  - midi-editing/arrow-keys-nudge-notes-shift-octave #nudging-with-the-arrow-keys
  - midi-editing/click-empty-grid-to-set-where-playback-starts #setting-where-playback-starts
  - midi-editing/velocity-lane #the-velocity-lane
  - midi-editing/quantize-cmd-ctrl-u-uses-the-quantize-popover-settings #quantize
  - midi-editing/scales-are-a-visual-guide-only #scales
  - midi-editing/scale-notes-only-hides-notes-that-are-still-there #scale-notes-only
  - midi-editing/editing-the-scale-in-follow-project-mode-changes-every-track #project-scale-or-track-scale
  - midi-editing/stretch-bar-2-2-and-reverse #stretching-and-reversing-notes
  - midi-editing/key-height-grip-and-triplet-grid #key-height-and-triplets
  - midi-editing/edit-several-midi-clips-at-once #editing-several-clips-at-once
  - midi-editing/multi-clip-notes-can-t-leave-their-clip-and-clips-never-grow #limits-when-editing-several-clips
  - midi-editing/looping-clips-only-the-first-pass-is-real #looping-clips
  - midi-editing/drum-rack-tracks-open-the-beat-editor #drum-rack-tracks-open-the-beat-editor
---

Le piano roll est l’endroit où vous écrivez et modifiez les notes d’un clip MIDI. Cliquez sur un clip MIDI dans l’arrangement : il s’ouvre dans l’éditeur en bas de la fenêtre. [[action:view.editor]] affiche ou masque cet éditeur. Les hauteurs vont de haut en bas le long du clavier à gauche, et le temps va de gauche à droite.

Si vous n’avez pas encore de clip MIDI, voir [clips](/docs/making-music/clips) pour en créer un.

## dessiner des notes {#drawing-notes}

Un simple clic sur une partie vide de la grille ne dessine pas de note : il déplace seulement le marqueur d’insertion (voir [choisir où démarre la lecture](#setting-where-playback-starts)). Pour ajouter une note :

![Un clip MIDI ouvert dans le piano roll, avec sa barre d’outils, ses notes et le couloir de vélocité, et l’inspecteur à droite](images/piano-roll.jpg "Un clip dans le piano roll, avec le couloir de vélocité sous les notes.")

- **Double-cliquez** sur un endroit vide. Si vous gardez le bouton de la souris enfoncé après le second clic et faites glisser vers la droite, vous réglez la longueur de la note en la dessinant.
- Ou activez **dessin** dans la barre d’outils ([[action:pianoRoll.drawMode]]). En mode dessin, chaque appui ajoute une note, et le glissement règle sa longueur. Appuyez de nouveau pour revenir à l’édition normale.

Les nouvelles notes ont une vélocité de 100. Sur une piste avec un instrument, vous entendez chaque note en la dessinant.

### longueur des notes et grille {#note-length-and-the-grid}

Une nouvelle note dure exactement un pas de grille. Le pas est affiché à côté du sélecteur **grille** dans la barre d’outils (par exemple `1/16`). Ce n’est pas la longueur de la dernière note dessinée. Pour dessiner des notes plus longues, choisissez une grille plus large, ou faites glisser en dessinant.

Le sélecteur **grille** propose des grilles adaptatives, dont le pas change avec le zoom, des pas fixes de **1 mesure** à **1/32**, ou **désactivé**.

## supprimer des notes {#deleting-notes}

- Double-cliquez sur une note pour la supprimer. Les deux clics doivent tomber sur la même note.
- Ou sélectionnez des notes et appuyez sur [[action:edit.delete]].
- Ou faites un clic droit sur une note et choisissez **supprimer la note** (ou **supprimer N notes** pour une sélection).

## déplacer, redimensionner et transposer {#moving-resizing-and-transposing}

- Faites glisser le corps d’une note pour la déplacer dans le temps et en hauteur.
- Faites glisser l’un des bords d’une note pour changer son début ou sa fin.
- Quand plusieurs notes sont sélectionnées, elles se déplacent ensemble et les intervalles entre elles sont conservés.

Les déplacements et les redimensionnements se calent sur la grille. Chaque glissement est une étape de l’[historique d’annulation](/docs/basics/undo-and-history).

### entendre les notes pendant l’édition {#hearing-notes-as-you-edit}

Appuyer sur une note la joue sur l’instrument de la piste tant que vous la maintenez. Un glissement vertical la rejoue à chaque nouvelle hauteur. Appuyer sur une touche du clavier à gauche joue aussi cette hauteur.

Le bouton casque de la barre d’outils active ou désactive cette écoute. Elle est activée par défaut, et votre choix est mémorisé dans ce navigateur ou cette application.

## sélectionner des notes {#selecting-notes}

- Cliquez sur une note pour la sélectionner.
- Un [[key:Shift]]-clic ajoute des notes à la sélection.
- Un [[key:Mod]]-clic ajoute une note à la sélection ou l’en retire. Le changement se fait quand vous relâchez le bouton, car un [[key:Mod]]-glisser duplique à la place (voir plus bas).
- Faites glisser sur une partie vide de la grille pour tracer un rectangle de sélection.
- [[action:edit.selectAll]] sélectionne toutes les notes affichées. [[action:edit.deselect]] efface la sélection.

### sélectionner toutes les notes d’une hauteur {#selecting-every-note-of-one-pitch}

Cliquez sur une touche du clavier à gauche : elle joue la hauteur et sélectionne toutes les notes de cette rangée. [[key:Shift]] et [[key:Mod]] ajoutent à la sélection comme sur les notes. C’est un moyen rapide de modifier tous les coups d’un même son de batterie.

## vélocité {#velocity}

La vélocité est la force avec laquelle une note est jouée. Les vélocités élevées sont en général plus fortes ou plus brillantes, selon l’instrument.

Le plus rapide pour la changer est de maintenir [[key:Alt]] au-dessus d’une note : le curseur devient une flèche haut/bas. Faites alors glisser verticalement. Si la note fait partie d’une sélection, toutes les notes sélectionnées changent de la même quantité. Un badge affiche la valeur pendant le glissement. Ajoutez [[key:Shift]] pour un réglage plus fin.

> [!note]
> Dans Ableton Live, [[key:Alt]]-glisser duplique les notes. Ici, [[key:Alt]]-glisser modifie la vélocité, et [[key:Mod]]-glisser duplique.

### le couloir de vélocité {#the-velocity-lane}

Sous la grille, le couloir de vélocité affiche une barre par note. Faites glisser une barre vers le haut ou le bas pour changer la vélocité de cette note. Si la barre appartient à une note sélectionnée, toute la sélection change de la même quantité. Survolez une barre pour voir sa valeur.

## dupliquer des notes {#duplicating-notes}

Maintenez [[key:Mod]] en faisant glisser une note : les originaux restent en place et vous faites glisser des copies. Vous pouvez appuyer sur [[key:Mod]] au début ou au milieu du glissement. Si vous relâchez [[key:Mod]] avant de déposer, les copies disparaissent et vous déplacez de nouveau les originaux. Le curseur affiche un signe de copie pendant que vous copiez.

[[action:edit.duplicate]] duplique les notes sélectionnées juste après elles-mêmes.

## désactiver le magnétisme {#turning-snapping-off}

La touche qui désactive le magnétisme dépend de ce que vous faites glisser :

- **Déplacer des notes :** maintenez [[key:Shift]]. [[key:Alt]] fonctionne aussi si vous l’enfoncez pendant le déplacement. Si vous maintenez [[key:Alt]] *avant* d’appuyer sur une note, vous modifiez sa vélocité à la place.
- **Tout le reste** (dessin, redimensionnement, rectangles de sélection, clic pour placer le marqueur, la barre d’étirement, les points d’expression) : maintenez [[key:Alt]].

## décaler avec les flèches {#nudging-with-the-arrow-keys}

Avec des notes sélectionnées :

- [[action:nudge.up]] et [[action:nudge.down]] les déplacent d’un demi-ton.
- [[action:pianoRoll.octaveUp]] et [[action:pianoRoll.octaveDown]] les déplacent d’une octave.
- [[action:nudge.left]] et [[action:nudge.right]] les déplacent d’un pas de grille plus tôt ou plus tard. La distance suit la grille : zoomer avec une grille adaptative la change donc.

Vous pouvez changer ces touches dans [raccourcis clavier](/docs/reference/keyboard-shortcuts).

## choisir où démarre la lecture {#setting-where-playback-starts}

Un clic sur une partie vide de la grille y place le marqueur d’insertion, calé sur la grille ([[key:Alt]] : sans magnétisme). Quand le morceau est arrêté, la lecture démarre depuis ce point. Cela ne déplace jamais la tête de lecture pendant la lecture. Cliquer sur la règle au-dessus de la grille déplace directement la position de lecture.

## sections {#sections}

Une section est une plage de temps qui couvre les rangées que vous avez survolées. Elle change ce que font copier, coller et supprimer (voir la partie suivante).

- Un simple glissement sur un espace vide sélectionne des notes.
- Un [[key:Shift]]-glisser sur un espace vide crée une section *et* sélectionne les notes qu’elle contient. Vous pouvez appuyer sur [[key:Shift]] ou le relâcher pendant le glissement pour basculer.
- Faire glisser sur la fine bande sous la règle crée aussi une section, sur toutes les rangées.
- [[action:edit.selectAll]] sélectionne toutes les notes affichées et crée une section couvrant tout le clip.

Appuyer sur une note, ou [[action:edit.deselect]], quitte la section.

### modifier avec une section {#editing-with-a-section}

Quand une section existe :

- [[action:edit.copy]], [[action:edit.cut]], [[action:edit.paste]] et [[action:edit.duplicate]] agissent sur la plage de temps exacte de la section, espace vide compris. Les notes qui dépassent les bords de la section sont coupées aux bords. Dupliquer place la copie juste après la section, ce qui peut laisser du silence après la dernière note.
- [[action:edit.delete]] vide le contenu de la section.

Sans section, ces raccourcis agissent sur les notes sélectionnées.

## quantifier {#quantize}

La quantification cale les notes sur la grille. Le bouton **quantifier** et [[action:pianoRoll.quantize]] appliquent les réglages de la fenêtre **quantifier…** :

- **grille** : **grille du piano roll** (le pas actuel du piano roll, par défaut) ou une valeur fixe de 1/4 à 1/32, triolets compris.
- **force** : à quel point les notes se rapprochent de la grille (100 % les cale exactement).
- **swing** : retarde un pas de grille sur deux.
- **quantifier les fins** : déplace aussi la fin des notes.

Elle agit sur les notes sélectionnées, ou sur toutes les notes du clip quand rien n’est sélectionné. Les réglages de quantification et d’humanisation sont mémorisés sur cet ordinateur, pour tous les projets.

> [!tip]
> Le bouton **quantifier** en un clic réutilise la force et le swing que vous avez réglés en dernier dans la fenêtre. Ouvrez d’abord **quantifier…** pour les vérifier.

La fenêtre **humaniser…** à côté fait l’inverse : elle ajoute de petites variations aléatoires au timing et à la vélocité. Voir [groove, swing et humanisation](/docs/advanced/groove).

## gammes {#scales}

Le bouton de gamme de la barre d’outils affiche la gamme active (par exemple **c mineur…**). Sa fenêtre règle :

- **piste** : **suivre la gamme du projet** (par défaut), **gamme propre à la piste** ou **chromatique / aucune**.
- la tonique et le type de la gamme.
- **surligner** : marque les notes de la gamme et sa tonique, et assombrit les autres.
- **notes de la gamme uniquement** : n’affiche que les rangées de la gamme.

Une gamme n’est qu’un guide visuel. Rien n’est forcé dans la gamme : l’entrée MIDI, l’enregistrement, le [clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard) et les flèches peuvent toujours produire n’importe quelle hauteur.

### notes de la gamme uniquement {#scale-notes-only}

Avec **notes de la gamme uniquement** activé, les rangées hors de la gamme sont repliées. Les notes de ces rangées ne sont pas supprimées : elles sont conservées et jouent toujours, mais vous ne les voyez pas. [[action:edit.selectAll]] ne sélectionne que les notes affichées. Désactivez l’option pour revoir toutes les hauteurs ou pour dessiner hors de la gamme.

### gamme du projet ou gamme de la piste {#project-scale-or-track-scale}

Quand la piste suit la gamme du projet, changer la tonique ou le type dans cette fenêtre change la gamme *du projet*. Toutes les pistes qui la suivent changent aussi. Le libellé à côté des sélecteurs indique **projet** ou **piste** pour montrer laquelle vous modifiez. Pour ne changer que cette piste, passez d’abord **piste** sur **gamme propre à la piste**.

## étirer et inverser des notes {#stretching-and-reversing-notes}

Quand des notes sont sélectionnées, une barre sur la règle les couvre :

- Faites glisser l’une des extrémités de la barre pour étirer ou resserrer les notes dans le temps ([[key:Alt]] : sans magnétisme).
- Faites glisser le milieu de la barre pour les déplacer.
- **×2** et **÷2** dans la barre d’outils doublent ou divisent par deux leur longueur, à partir de leur début.

**inverser** dans la barre d’outils, ou [[action:clip.reverse]], inverse dans le temps les notes sélectionnées (ou les notes de la section, s’il y en a une).

## hauteur des touches et triolets {#key-height-and-triplets}

- Faites glisser la poignée au-dessus du clavier vers le haut ou le bas pour agrandir ou réduire les touches. Double-cliquez dessus pour revenir à la hauteur par défaut.
- Le bouton **3** passe la grille en triolets.

## clips en boucle {#looping-clips}

Quand un clip boucle, les répétitions après le premier passage affichent les notes de la boucle comme des fantômes estompés. Seul le premier passage est réel. Si vous dessinez dans une répétition, la note est écrite dans la boucle elle-même : elle apparaît donc à chaque passage.

## modifier plusieurs clips à la fois {#editing-several-clips-at-once}

Sélectionnez plusieurs clips MIDI dans l’arrangement, puis ouvrez l’un d’eux, ou faites un clic droit et choisissez **modifier N clips dans le piano roll**. Le piano roll les affiche alors tous sur la ligne de temps du morceau :

- Une bande sous la règle montre la zone de chaque clip, avec son nom et sa boucle.
- Les notes prennent la couleur de leur clip. Les notes des clips autres que le clip actif sont estompées.
- Cliquez sur la zone d’un clip, ou sur l’une de ses notes, pour rendre ce clip actif. Le titre de la barre d’outils, la gamme et les couloirs d’expression suivent le clip actif.
- La sélection, le déplacement, le décalage, la quantification, la vélocité, copier et coller fonctionnent tous à travers les clips. Un glissement est une seule étape d’annulation, même s’il modifie plusieurs clips.

### limites quand vous modifiez plusieurs clips {#limits-when-editing-several-clips}

- Une note ne peut pas quitter son clip. Un déplacement s’arrête au bord de la zone du clip.
- Rien ne se dessine hors d’un clip. Une nouvelle note va dans le clip sous le pointeur.
- Coller place chaque note dans le clip situé dessous et abandonne les notes qui tombent hors de tout clip.
- Les clips ne s’agrandissent jamais quand plusieurs sont ouverts.

## les pistes drum rack ouvrent l’éditeur de rythmes {#drum-rack-tracks-open-the-beat-editor}

Sur une piste avec un [drum rack](/docs/making-music/drum-rack), les clips s’ouvrent plutôt dans l’éditeur de rythmes : une grille de pas avec une rangée par pad et une case par double croche.

- Cliquez sur une case pour ajouter ou retirer un coup. Un clic droit sur une case en retire un.
- Faites glisser le long d’une rangée pour peindre plusieurs coups, ou pour les effacer si vous avez commencé sur un coup ou utilisé le bouton droit.
- La luminosité d’une case indique sa vélocité.

Les onglets **éditeur de rythmes** et **piano roll**, à droite de la barre d’outils, passent d’un éditeur à l’autre pour cette piste.

## et ensuite {#where-next}

- [Le clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard) : jouez des notes sans aucun matériel.
- [Expression et MPE](/docs/making-music/expression-and-mpe) : pitch bend, molette de modulation et courbes par note.
- [Enregistrement](/docs/making-music/recording) et [capture MIDI](/docs/making-music/capture-midi) : enregistrez ce que vous jouez.
- [Appareils et presets](/docs/making-music/devices-and-presets) : choisissez le son que jouent vos notes.
