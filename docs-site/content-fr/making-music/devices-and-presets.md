---
title: appareils et presets
order: 50
summary: Ajoutez des instruments et des effets à une piste, ouvrez leurs fenêtres, tournez leurs boutons et enregistrez vos sons en presets.
platform: all
audience: beginner
covers:
  - devices/adding-a-device #adding-a-device
  - devices/instruments-only-on-midi-tracks-one-per-chain #what-goes-where
  - devices/chain-order-is-fixed-by-section #the-order-of-a-chain
  - devices/device-window-vs-parameter-list #the-device-window-and-the-parameter-list
  - devices/bypass-rename-copy-paste-devices #bypass-rename-copy-and-paste
  - devices/hover-only-header-buttons #the-device-header
  - devices/knob-gestures #turning-knobs
  - devices/touch-knobs-fine-mode-by-moving-sideways #turning-knobs
  - devices/no-typing-values-into-knobs #exact-values
  - devices/parameter-context-menu #the-parameter-menu
  - devices/show-all-parameters-and-pinning #plugins-with-many-parameters
  - devices/presets-load-save-factory-vs-user #presets
  - devices/presets-are-files-in-your-user-library #where-presets-are-saved
  - devices/current-preset-name-is-per-session #the-preset-name-on-the-button
  - devices/device-library-favourites-sort-and-format-toggles #favourites-sorting-and-plugin-formats
---

Les appareils produisent et façonnent le son d’une piste. Un **instrument** transforme des notes MIDI en audio, un **effet audio** modifie l’audio qui le traverse, et un **effet MIDI** modifie les notes avant qu’elles n’atteignent l’instrument. Cette page montre comment les ajouter, comment utiliser leurs réglages et comment fonctionnent les presets. Il vous faut un projet avec au moins une piste : voir [les pistes](/docs/making-music/tracks) si vous n’en avez pas encore.

## où vivent les appareils {#where-devices-live}

Sélectionnez une piste : ses appareils apparaissent dans l’**inspecteur**, le panneau de droite, sous le nom et le mixeur de la piste. Les appareils d’une piste forment sa **chaîne** : le signal les traverse de haut en bas. Chaque appareil est une carte avec un en-tête (son bouton d’alimentation, son nom et quelques boutons), et les réglages propres à l’appareil s’ouvrent dans une fenêtre.

## ajouter un appareil {#adding-a-device}

Chaque section de la chaîne a son propre bouton d’ajout : le **+** à côté du libellé de la section, ou la carte en pointillés **ajouter un instrument**, **ajouter un effet audio** ou **ajouter un effet MIDI** à la fin de la section. L’un comme l’autre ouvre la bibliothèque d’appareils :

- tapez dans le champ de recherche pour filtrer, puis appuyez sur [[key:Enter]] ou cliquez sur une ligne pour l’ajouter ;
- les puces **tous**, **usine** et **plug-ins** choisissent entre les appareils intégrés et vos plug-ins (les plug-ins sont réservés à l’application de bureau) ;
- l’étoile filtre vos favoris, et le menu de tri classe la liste par nom, par les plus utilisés ou par les plus récents ;
- faites glisser une ligne dans la chaîne pour placer l’appareil à un endroit de sa section : avant un appareil, ou à la fin de la section.

Un appareil que vous ajoutez ouvre aussitôt sa fenêtre, pour que vous puissiez commencer à jouer avec. Vous pouvez aussi ajouter un appareil depuis la palette de commandes ([[action:palette.open]]).

### quoi va où {#what-goes-where}

- Les instruments et les effets MIDI ont besoin de notes, ils ne vont donc que sur les **pistes MIDI**. Les pistes audio n’acceptent que des effets audio.
- Une chaîne a **un seul instrument**. Ajouter un autre instrument remplace l’actuel, en une seule étape d’annulation.
- Les pistes VCA n’acceptent aucun appareil.

Si un appareil ne peut pas aller sur la piste sélectionnée, un message explique pourquoi (par exemple « sélectionnez une piste MIDI »).

### l’ordre d’une chaîne {#the-order-of-a-chain}

Les sections viennent toujours dans le même ordre : d’abord les effets MIDI, puis l’instrument, puis les effets audio. Vous pouvez réordonner les appareils à l’intérieur d’une section en faisant glisser l’en-tête d’une carte vers le haut ou vers le bas, mais un effet MIDI reste toujours avant l’instrument et un effet audio après.

## l’en-tête d’un appareil {#the-device-header}

De gauche à droite, l’en-tête d’un appareil contient :

- le bouton **fenêtre**, qui ouvre la fenêtre propre à l’appareil ;
- le bouton **alimentation**, qui active l’appareil ou le contourne ;
- le **nom** de l’appareil ;
- le bouton **presets** et le bouton **+** qui ajoute un modulateur. Ces deux-là n’apparaissent que quand le pointeur survole la carte, pour garder la chaîne lisible ;
- un sélecteur de **sidechain**, sur les appareils qui peuvent être déclenchés par une autre piste (voir [modulation et sidechain](/docs/making-music/modulation-and-sidechain#sidechain)) ;
- le bouton **paramètres** (icône de curseurs), affiché comme les presets quand le pointeur survole la carte (ou quand la liste est ouverte), et **×** pour supprimer l’appareil.

### la fenêtre de l’appareil et la liste des paramètres {#the-device-window-and-the-parameter-list}

La carte elle-même n’affiche que l’en-tête. Il y a deux façons de voir les réglages d’un appareil, indépendantes l’une de l’autre :

- Le bouton **fenêtre** ouvre l’interface propre à l’appareil dans une fenêtre flottante : la disposition de l’appareil d’usine, ou l’éditeur d’un plug-in. Vous pouvez la déplacer et la redimensionner, et elle revient là où vous l’avez laissée, sur cet ordinateur, la prochaine fois que vous l’ouvrez.
- Le bouton **paramètres** (icône de curseurs) affiche une simple liste de tous les paramètres sur la carte elle-même, avec un champ de recherche quand la liste est longue. Il fonctionne de la même façon pour tous les appareils, intégrés ou plug-ins.

Les racks font exception : un rack n’a pas de fenêtre, car ses chaînes et ses macros sont sur sa carte (voir [racks et macros](/docs/making-music/racks-and-macros)).

### contourner, renommer, copier et coller {#bypass-rename-copy-and-paste}

- Cliquez sur le bouton **alimentation** pour contourner un appareil (le son le traverse sans changement). Cliquez à nouveau pour réactiver l’appareil. Un appareil réactivé repart de zéro : une réverbération ou un délai ne fait pas revenir la queue qu’il avait quand vous l’avez contourné (de même pour tout ce que contient un rack réactivé, et avec l’automation marche/arrêt).
- Double-cliquez sur le nom pour renommer l’appareil. Un nom vide rétablit son nom d’origine.
- Cliquez sur une carte pour la sélectionner, puis appuyez sur [[action:edit.copy]] pour la copier et sur [[action:edit.paste]] pour coller la copie après l’appareil sélectionné. La copie garde tout : les réglages, l’état d’un plug-in, les chaînes d’un rack ou les pads d’un drum rack.
- [[key:Mod]]-clic ajoute une carte à la sélection ou l’en retire, et [[key:Shift]]-clic sélectionne les cartes entre les deux : effets MIDI, instrument et effets audio ensemble. Un collage place chaque copie dans sa propre section, en une seule étape d’annulation. Un instrument collé remplace celui de la chaîne, après confirmation. [[action:edit.cut]] coupe les appareils sélectionnés : ils s’estompent, et le collage suivant les déplace. [[action:edit.delete]] les supprime. [[action:edit.selectAll]] sélectionne tous les appareils de la chaîne. Sans appareil sélectionné, [[action:edit.paste]] colle à la fin de la chaîne de la piste sélectionnée. Faites glisser l’en-tête d’un appareil sur une piste de l’arrangement (son en-tête ou son couloir) pour l’y déplacer, avec les appareils sélectionnés ; maintenez [[key:Mod]] en le déposant pour copier à la place. Appuyez sur [[action:device.group]] pour grouper les appareils sélectionnés dans un rack (ils doivent être côte à côte) : un rack d’instrument quand l’instrument en fait partie, sinon un rack d’effets MIDI ou audio.
- Faites un clic droit sur l’en-tête pour les mêmes actions et d’autres : **copier**, **coller après**, **renommer…**, **contourner**, **grouper dans un rack …**, une entrée **ajouter …** par type de modulateur, et **supprimer l’appareil**.

## tourner les boutons {#turning-knobs}

- **Faites glisser** un bouton vers le haut ou vers le bas pour le changer. Maintenez [[key:Shift]] en faisant glisser pour un réglage fin (dix fois plus lent).
- **Double-cliquez** sur un bouton pour le remettre à sa valeur par défaut.
- Un bouton qui a le focus répond aussi aux flèches, à [[key:PageUp]], [[key:PageDown]], [[key:Home]] et [[key:End]].
- Curseurs horizontaux : cliquez sur la glissière pour y sauter, ou faites glisser le curseur. [[key:Shift]] le rend plus fin.

Sur un écran tactile [[mobile]], faites glisser votre doigt vers le haut ou vers le bas sur le bouton. Plus votre doigt s’éloigne du bouton sur le côté pendant le glissement, plus le changement est fin : vous pouvez faire de grands mouvements puis des réglages précis sans lever le doigt (voir [modifier au toucher](/docs/mobile/editing-on-touch)).

### valeurs exactes {#exact-values}

Vous ne pouvez pas saisir de valeur dans un bouton. Pour régler une valeur précise, faites glisser en maintenant [[key:Shift]], ou donnez le focus au bouton et utilisez les flèches. Quelques réglages acceptent des nombres saisis, comme les champs numériques et les réglages de zone du multisampler. Pour les points d’automation, faites un clic droit sur un point et choisissez **définir la valeur…** (voir [l’automation](/docs/making-music/automation#set-an-exact-value)).

### le menu des paramètres {#the-parameter-menu}

Faites un clic droit sur n’importe quel paramètre d’un appareil pour :

- **afficher le couloir d’automation** : ouvre le couloir d’automation de ce paramètre sous la piste ;
- **réinitialiser**.

Un paramètre automatisé affiche un petit repère, et son bouton suit l’automation (voir [l’automation](/docs/making-music/automation#automated-knobs-follow-their-lane)).

### les plug-ins avec beaucoup de paramètres {#plugins-with-many-parameters}

Un plug-in peut avoir des centaines de paramètres. Sa liste de paramètres sur la carte a une hauteur limitée et défile. **afficher les n paramètres…** ouvre une liste consultable de tous les paramètres, où vous pouvez **épingler** ceux que vous utilisez. Les paramètres épinglés viennent en premier sur la carte.

Les épingles appartiennent au plug-in, pas à une instance : elles s’appliquent à toutes les copies de ce plug-in dans tous les projets. Elles ne sont gardées que sur cet ordinateur.

## presets {#presets}

Un preset est un son enregistré pour un type d’appareil. Le bouton **presets** dans l’en-tête de l’appareil (il apparaît quand vous survolez la carte, et affiche le nom du preset actuel, ou **presets**) ouvre la liste :

- les presets **usine** sont fournis avec Ethereal, les presets **utilisateur** sont ceux que vous avez enregistrés ;
- la recherche filtre par nom et par tags ;
- cliquez sur un preset pour le charger. Le chargement est une seule étape d’annulation.

Pour enregistrer le son actuel, choisissez **enregistrer le preset…**, donnez-lui un nom et, si vous le souhaitez, quelques tags séparés par des virgules. Si l’un de vos presets porte déjà ce nom, le bouton devient **remplacer** : cliquez à nouveau pour l’écraser. Faites un clic droit sur l’un de vos presets (ou utilisez son bouton **…**) pour le renommer ou le supprimer. Les presets d’usine ne peuvent pas être modifiés.

Les plug-ins ont aussi des presets : un preset de plug-in enregistre l’état complet du plug-in.

### où les presets sont enregistrés {#where-presets-are-saved}

Les presets utilisateur sont des fichiers de votre **bibliothèque utilisateur** : ils sont donc disponibles dans tous les projets et apparaissent aussi dans les presets du [navigateur](/docs/sounds-and-plugins/browser) :

- dans l’application de bureau, dans `Documents/Ethereal/User Library/Presets/<device>/` sous forme de fichiers `.etherpreset` (les presets de plug-ins sous `Presets/plugins/<format>/<plugin id>/`) ;
- dans l’application web, dans le stockage du navigateur, comme vos projets.

Un preset de sampler ou de multisampler copie aussi ses échantillons dans le dossier `Samples` de la bibliothèque utilisateur, pour que le preset fonctionne encore dans d’autres projets.

### le nom du preset sur le bouton {#the-preset-name-on-the-button}

Le bouton presets affiche le dernier preset que vous avez chargé sur cet appareil **pendant cette session**. Quand vous rouvrez un projet, il n’indique plus d’où vient le son d’un appareil, et tourner un bouton après avoir chargé un preset ne le marque pas comme modifié.

## favoris, tri et formats de plug-ins {#favourites-sorting-and-plugin-formats}

La bibliothèque d’appareils retient quelques choix sur cet ordinateur :

- cliquez sur l’étoile d’une ligne pour en faire un favori, et sur l’étoile du haut pour n’afficher que les favoris ;
- le menu de tri : **nom**, **plus utilisés** ou **plus récents** ;
- quand vos plug-ins existent en plusieurs formats, les puces **CLAP**, **VST3** et **AU** choisissent les formats affichés. Un plug-in installé en plusieurs formats n’apparaît qu’une fois, dans votre format préféré, sauf si vous désactivez **afficher chaque plug-in une seule fois** dans **réglages › plug-ins › liste des plug-ins** (voir [le même plug-in dans plusieurs formats](/docs/sounds-and-plugins/plugins#the-same-plugin-in-several-formats)). Au moins un format reste activé.

## et ensuite {#where-next}

- [Instruments](/docs/making-music/instruments) : ce que fait chaque instrument intégré.
- [Effets](/docs/making-music/effects) : les effets audio et MIDI intégrés.
- [L’automation](/docs/making-music/automation) : faites bouger les paramètres dans le temps.
