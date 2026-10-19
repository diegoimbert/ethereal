---
title: raccourcis clavier
order: 10
summary: Tous les raccourcis par défaut, générés depuis la table des raccourcis de l’application, et comment les modifier.
platform: all
audience: everyone
covers:
  - shortcuts-commands/transport-shortcuts #all-shortcuts
  - shortcuts-commands/edit-shortcuts #all-shortcuts
  - shortcuts-commands/time-selection-shortcuts #all-shortcuts
  - shortcuts-commands/arrangement-shortcuts #all-shortcuts
  - shortcuts-commands/piano-roll-shortcuts #all-shortcuts
  - shortcuts-commands/automation-lane-shortcuts #all-shortcuts
  - shortcuts-commands/app-and-view-shortcuts #all-shortcuts
  - shortcuts-commands/copy-cut-paste-act-on-the-time-selection-when-there-is-one #the-time-selection-comes-first
  - shortcuts-commands/computer-midi-keyboard-m #the-computer-midi-keyboard
  - shortcuts-commands/mouse-modifiers #all-shortcuts
  - shortcuts-commands/customizing-shortcuts #changing-shortcuts
  - shortcuts-commands/ableton-like-preset #the-ableton-like-preset
  - shortcuts-commands/conflicts #conflicts
  - shortcuts-commands/any-palette-command-can-get-a-shortcut #any-command-can-get-a-shortcut
  - shortcuts-commands/print-the-cheat-sheet #printing-a-cheat-sheet
  - shortcuts-commands/where-shortcuts-are-saved #where-your-shortcuts-are-saved
  - shortcuts-commands/ctrl-vs-on-mac #mac-and-pc-keys
  - shortcuts-commands/browser-shortcuts-that-win-on-the-web #in-the-browser
  - shortcuts-commands/shortcuts-in-text-fields-and-plugin-windows #text-fields-and-plugin-windows
---

Cette page liste tous les raccourcis par défaut. Elle est générée depuis la table des raccourcis de l’application, elle est donc toujours à jour. Utilisez le sélecteur **mac** / **windows / linux** en haut de la page pour voir les touches de votre ordinateur. La plupart des actions sont aussi dans la palette de commandes ([[action:palette.open]]), avec leur raccourci à côté.

## tous les raccourcis {#all-shortcuts}

« Où » indique quelle partie de l’application doit avoir le focus (cliquez d’abord dedans) : l’arrangement, le piano roll ou un couloir d’automation. Les autres fonctionnent partout, sauf pendant que vous tapez dans un champ de texte.

[[shortcuts]]

## la sélection temporelle passe en premier {#the-time-selection-comes-first}

Dans l’arrangement, quand il y a une sélection temporelle (glissement avec [[key:Shift]] sur les couloirs), les touches d’édition agissent sur cette plage de temps plutôt que sur les clips : copier, couper, dupliquer et supprimer prennent le temps sélectionné sur les pistes sélectionnées, coller place une copie juste après la sélection (coller de nouveau la répète donc bout à bout), et scinder coupe les clips aux deux bords. Voir [arrangement](/docs/making-music/arranging).

## le clavier midi de l’ordinateur {#the-computer-midi-keyboard}

[[action:midi.computerKeyboard]] transforme le clavier de votre ordinateur en piano : la rangée du milieu joue les touches blanches, la rangée du dessus les touches noires, [[key:Z]] / [[key:X]] changent l’octave et [[key:C]] / [[key:V]] la vélocité. Tant qu’il est activé, ces lettres jouent des notes au lieu de leurs raccourcis. Les raccourcis avec [[key:Mod]], [[key:Ctrl]] ou [[key:Alt]], et [[action:transport.play]], fonctionnent toujours. Voir [le clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard).

## modifier les raccourcis {#changing-shortcuts}

Ouvrez la palette de commandes et choisissez **raccourcis clavier…**, ou cliquez sur **modifier les raccourcis clavier…** dans **réglages › souris**. L’éditeur liste toutes les actions par groupe ; cherchez par nom ou par touche.

- **+** sur une ligne enregistre un nouveau raccourci : appuyez sur les touches, puis sur **utiliser** (ou [[key:Enter]]). [[key:Escape]] annule. Une action peut avoir jusqu’à 4 raccourcis.
- **×** à côté d’un raccourci le supprime.
- La flèche de réinitialisation d’une ligne rétablit son raccourci par défaut.
- **tout réinitialiser aux raccourcis ethereal** annule toutes les modifications.

### le preset ableton-like {#the-ableton-like-preset}

Le menu **preset** en haut de l’éditeur passe de **ethereal** (par défaut) à **ableton-like**. Le preset Ableton-like ajoute les touches d’Ableton Live pour les mêmes commandes en plus de celles par défaut, et ne retire rien : par exemple [[key:F9]] pour enregistrer, [[key:Mod+L]] pour la boucle, [[key:Mod+T]] et [[key:Mod+Shift+T]] pour de nouvelles pistes audio et MIDI. Le tableau ci-dessus les montre sous chaque action.

### conflits {#conflicts}

Quand deux actions qui fonctionnent au même endroit partagent une touche, les deux lignes sont signalées. **Conflits uniquement** n’affiche que celles-là. Quand vous enregistrez une touche qu’une autre action utilise déjà, l’éditeur la nomme, et vous pouvez **utiliser quand même** ou retirer la touche à l’autre action.

### toute commande peut avoir un raccourci {#any-command-can-get-a-shortcut}

Chaque commande de la palette (ajouter un appareil, ouvrir un panneau…) est dans l’éditeur et peut recevoir un raccourci, sauf celles liées au projet en cours, comme « aller à la piste ».

### imprimer un aide-mémoire {#printing-a-cheat-sheet}

**Imprimer les raccourcis clavier** dans la palette de commandes imprime tous vos raccourcis actuels, avec les modificateurs de la souris.

### où vos raccourcis sont enregistrés {#where-your-shortcuts-are-saved}

Vos raccourcis sont enregistrés dans votre bibliothèque utilisateur : ils sont donc les mêmes dans tous les projets et toutes les fenêtres. Si le moteur ne peut pas les stocker, l’éditeur indique que les modifications durent jusqu’à la fermeture d’Ethereal.

## touches mac et pc {#mac-and-pc-keys}

Sur un Mac, les raccourcis utilisent ⌘ (Commande). Un accord avec Contrôle qu’aucune action n’utilise fonctionne comme sa version avec ⌘. Sous Windows et Linux, les mêmes raccourcis utilisent Ctrl.

## dans le navigateur [[web]] {#in-the-browser}

Certaines touches appartiennent au navigateur et n’atteignent jamais la page : nouvel onglet, rouvrir un onglet, fermer l’onglet et nouvelle fenêtre ([[key:Mod+T]], [[key:Mod+Shift+T]], [[key:Mod+W]], [[key:Mod+N]]). Les touches Ableton-like pour les nouvelles pistes ne fonctionnent donc pas dans la version web. Les autres touches qu’utilise l’application, comme enregistrer, dupliquer ou la palette, fonctionnent dans la page.

## champs de texte et fenêtres de plug-ins {#text-fields-and-plugin-windows}

Les raccourcis sont ignorés pendant que vous tapez dans un champ de texte ([[action:transport.play]] y tape une espace). Sur le bureau, les fenêtres de plug-ins transmettent [[action:transport.play]] à Ethereal et, quand le clavier MIDI de l’ordinateur est activé, ses touches, pour que vous puissiez jouer depuis l’intérieur d’un plug-in.

## et ensuite {#where-next}

- [La palette de commandes et les réglages](/docs/basics/command-palette-and-settings).
- [Arrangement : sélections et modifications temporelles](/docs/making-music/arranging).
- [Le piano roll](/docs/making-music/piano-roll).
