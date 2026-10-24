---
title: une visite de l’écran
order: 10
summary: Découvrez à quoi sert chaque partie de l’éditeur, et comment les panneaux latéraux et le tiroir du bas s’ouvrent, flottent et restent épinglés.
platform: desktop, web
audience: beginner
covers:
  - app-shell/floating-panels-pin-click-away-collapse #panels-that-float
  - app-shell/show-hide-the-editor-drawer-cmd-ctrl-j #the-editor-drawer
  - app-shell/toasts-which-ones-stay #messages-at-the-bottom
  - transport-tempo/cpu-disk-and-engine-indicators #cpu-disk-and-engine-status
---

Cette page est une visite rapide de l’éditeur : la fenêtre dans laquelle vous travaillez une fois un projet ouvert. Vous n’avez rien à configurer avant. Si vous êtes sur un téléphone, voyez plutôt [la bêta mobile](/docs/mobile/mobile-beta) : sa disposition est différente.

De haut en bas, l’éditeur comporte une **barre du haut**, l’**arrangement** au milieu, une fine **barre d’état** en bas, et trois panneaux qui glissent par-dessus l’arrangement : le **navigateur** à gauche, l’**inspecteur** à droite et le **tiroir d’édition** en bas.

## la barre du haut {#the-top-bar}

De gauche à droite :

![L’éditeur avec quatre pistes de clips MIDI : la barre du haut, les en-têtes de piste, l’arrangement et la barre d’état](images/the-screen.jpg "L’éditeur : la barre du haut, les en-têtes de piste et l’arrangement.")

- **Le menu du projet.** Le bouton ☰ ouvre le menu du projet (nouveau, ouvrir, importer, exporter, versions…). À côté se trouve le nom du projet : cliquez dessus pour renommer le projet. Un point après le nom signifie que certaines modifications ne sont pas encore enregistrées. Voir [projets et enregistrement](/docs/basics/projects).
- **Le transport.** Lecture, arrêt, enregistrement, puis la position, le tempo et la signature rythmique, puis la boucle et le métronome. Voir [lire et se déplacer](/docs/basics/playing-and-navigating) et [tempo, signature rythmique et gamme](/docs/basics/tempo-and-time-signature).
- **Annuler et rétablir**, avec le nom de l’étape que chacun annulerait ou rétablirait quand vous le survolez.
- **Les indicateurs du moteur** : la charge cpu, une alerte disque et un voyant d’état. Ils sont décrits [plus bas](#cpu-disk-and-engine-status).
- **Les réglages du métronome, les commandes d’enregistrement** (enregistrer les pistes armées, décompte, punch, entrées). Pendant un export audio dont la fenêtre est fermée, un bouton **export %** en montre la progression.

## l’arrangement {#the-arrangement}

L’arrangement, c’est votre morceau étalé dans le temps : une ligne par piste, le temps allant de gauche à droite, avec la règle en haut. Les clips sont posés sur les pistes. L’en-tête de chaque piste, à gauche, contient son nom, muet, solo, armer, le volume et le panoramique.

La petite barre d’outils dans le coin de l’arrangement contient le réglage de la grille et le bouton **suivre** (voir [suivre la tête de lecture](/docs/basics/playing-and-navigating#follow-the-playhead)).

Pour en savoir plus, lisez [les pistes](/docs/making-music/tracks) et [les clips](/docs/making-music/clips).

## le rail de gauche et le navigateur {#the-left-rail-and-the-browser}

L’étroite colonne d’icônes tout à gauche est le **rail**. Chaque icône ouvre un panneau :

- **bibliothèque** : le navigateur, avec vos échantillons, presets et appareils. Voir [le navigateur](/docs/sounds-and-plugins/browser).
- **assignation MIDI** : assigner des boutons matériels à des commandes (application de bureau).
- **historique** : toutes vos modifications, pour revenir à n’importe laquelle. Voir [annulation et historique](/docs/basics/undo-and-history).
- **demander à l’IA** : l’assistant IA.
- Le dernier bouton ouvre la [palette de commandes](/docs/basics/command-palette-and-settings).

## l’inspecteur {#the-inspector}

L’inspecteur s’ouvre à droite quand vous sélectionnez quelque chose, et se masque de nouveau quand rien n’est sélectionné :

- Quand une **piste** est sélectionnée, il affiche les réglages de cette piste et ses **appareils** : les instruments et les effets de la piste, de haut en bas. Voir [appareils et presets](/docs/making-music/devices-and-presets).
- Quand des **clips** sont sélectionnés, il affiche leurs réglages (gain, transposition, warp…). Avec plusieurs clips sélectionnés, une modification s’applique à tous.

## le tiroir d’édition {#the-editor-drawer}

Le tiroir du bas contient les éditeurs de clips, dans trois onglets : **piano roll** (clips MIDI), **warp** (clips audio) et **automation**. Cliquer sur un clip ouvre son éditeur ici. Avec plusieurs clips sélectionnés, il en affiche un et ses modifications s’appliquent à tous.

[[action:view.editor]] affiche ou masque le tiroir à tout moment.

## les panneaux flottants {#panels-that-float}

Le navigateur, l’inspecteur et le tiroir flottent par-dessus l’arrangement : ils ne lui prennent pas de place quand vous ne les utilisez pas.

- Un navigateur **non épinglé** se ferme dès que vous cliquez dans l’arrangement. Un tiroir non épinglé se ferme quand vous cliquez n’importe où en dehors, sauf sur des clips (cliquer sur un autre clip fait passer le tiroir sur ce clip).
- Le bouton **épingler** dans l’en-tête d’un panneau le garde ouvert, à côté de l’arrangement au lieu de par-dessus. Cliquez de nouveau pour laisser le panneau flotter.
- Le **×** de l’inspecteur et du tiroir le réduit à une petite barre le long du bord. Cliquez sur la barre pour le déplier. Il reste replié jusqu’à ce que vous le dépliiez (l’inspecteur suit quand même la sélection). Un double-clic sur un clip déplie les deux : voir [double-cliquer sur un clip](/docs/making-music/clips#double-clicking-a-clip).
- Faites glisser le bord intérieur d’un panneau pour le redimensionner.

La palette de commandes propose aussi **épingler le navigateur**, **épingler l’inspecteur** et **épingler le tiroir d’édition** (et leurs versions **désépingler**).

## la barre d’état {#the-status-bar}

La fine barre en bas de la fenêtre montre ce qui demande votre attention, tant que ça dure : **plug-ins désactivés (mode sans échec)** avec **charger les plug-ins** (voir [le mode sans échec](/docs/advanced/safe-mode)), les plug-ins ou samples manquants, et les imports en cours. La roue dentée des **réglages** est à son extrémité droite.

## les messages en bas {#messages-at-the-bottom}

De courts messages (notifications) apparaissent dans un coin de la fenêtre :

- Les messages d’**info** disparaissent d’eux-mêmes après environ 6 secondes. Les survoler les garde affichés.
- Les **avertissements** et les **erreurs** restent jusqu’à ce que vous les fermiez, pour que vous ne les manquiez pas.
- 3 au plus sont affichés à la fois ; les plus anciens sont retirés. Il n’y a pas de liste des messages passés : lisez un avertissement avant de le fermer.

## cpu, disque et état du moteur {#cpu-disk-and-engine-status}

À droite de la barre du haut :

- **cpu** indique à quel point le moteur audio est occupé (sa charge DSP). Si elle approche 100 %, vous pouvez entendre des clics et des coupures. Cliquez dessus pour **arrêter le moteur** : rien ne joue, les plug-ins sont déchargés et le périphérique audio est libre pour d’autres applications, et l’indicateur affiche **moteur arrêté**. Cliquez à nouveau et choisissez **démarrer le moteur** pour recharger le projet. Ouvrir un autre projet le redémarre aussi.
- **Disque** n’apparaît qu’après une coupure de l’audio lu depuis le disque pendant la lecture, avec le nombre de coupures. Cliquez dessus pour remettre le compteur à zéro.
- **Le voyant d’état**, avec le nom du système audio à côté, montre l’état du moteur. Il prend une couleur d’alerte pendant la connexion ou après des coupures, et une couleur d’erreur quand le moteur a échoué ou que l’audio s’est arrêté. Il affiche **fermé** quand le moteur a libéré le périphérique audio. Survolez-le pour les détails : le système audio, la fréquence d’échantillonnage, la taille du buffer et le nombre de coupures.

## et ensuite {#where-next}

- [Lire et se déplacer](/docs/basics/playing-and-navigating)
- [Projets et enregistrement](/docs/basics/projects)
- [La palette de commandes et les réglages](/docs/basics/command-palette-and-settings)
