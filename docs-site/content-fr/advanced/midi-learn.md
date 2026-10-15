---
title: apprentissage midi
order: 40
summary: Assignez les potentiomètres, faders, pads et touches d’un contrôleur MIDI à des commandes d’Ethereal.
platform: desktop
audience: advanced
covers:
  - audio-midi-setup/midi-controllers-just-work-no-device-list #your-controller-just-works
  - audio-midi-setup/midi-mapping-turn-on-midi-mode-click-a-control-move-a-knob #mapping-a-control
  - audio-midi-setup/right-click-any-control-learn-midi-remove-midi-mapping #from-a-controls-right-click-menu
  - audio-midi-setup/mappings-are-saved-in-the-project-not-globally #mappings-belong-to-the-project
  - audio-midi-setup/one-mapping-per-knob-learning-again-replaces-it #one-knob-one-target
  - audio-midi-setup/mapping-options-mode-min-and-max #mode-minimum-and-maximum
  - audio-midi-setup/mapped-keys-still-play-notes #mapped-keys-still-play-notes
  - audio-midi-setup/midi-learn-needs-the-desktop-app #desktop-only
---

L’apprentissage MIDI relie une commande de votre contrôleur MIDI (un potentiomètre, un fader, un pad ou une touche) à une commande d’Ethereal : un paramètre d’appareil, le volume d’une piste, lecture/arrêt, etc. Vous l’assignez une fois, et bouger le matériel fait bouger la commande à l’écran.

## votre contrôleur fonctionne tout seul {#your-controller-just-works}

Il n’y a pas de liste de périphériques MIDI à configurer. Chaque entrée MIDI branchée sur votre ordinateur s’ouvre d’elle-même. Chaque piste MIDI choisit les ports qu’elle écoute dans son panneau **in ▾** (tous les ports par défaut) : voir [l’enregistrement](/docs/making-music/recording).

## assigner une commande {#mapping-a-control}

1. Ouvrez l’onglet **assignation MIDI** dans la barre latérale gauche.
2. Activez le **mode MIDI**. Toutes les commandes assignables s’allument, et celles qui sont déjà assignées sont marquées.
3. Cliquez sur une commande en surbrillance. Le panneau indique qu’il est en apprentissage (cliquez de nouveau sur la commande, ou sur **annuler**, pour arrêter).
4. Tournez un potentiomètre ou appuyez sur une touche de votre contrôleur. L’assignation apparaît dans la liste du panneau.

[[key:Escape]] annule un apprentissage en cours. Désactivez le **mode MIDI** pour utiliser de nouveau l’application normalement : vos assignations continuent de fonctionner.

### depuis le menu du clic droit d’une commande {#from-a-controls-right-click-menu}

Quand le mode MIDI est activé, faites un clic droit sur une commande assignable pour **apprendre le MIDI** et **supprimer l’assignation MIDI**.

### cibles sans commande {#targets-without-a-control}

Certaines cibles n’ont pas de commande à cliquer à l’écran. Choisissez-les depuis **apprendre une autre cible…** dans le panneau :

- actions de transport : lecture/arrêt, lecture, arrêt, enregistrement, boucle, métronome, tap tempo, marqueur précédent et marqueur suivant ;
- armement, muet ou solo de la piste sélectionnée.

## les assignations appartiennent au projet {#mappings-belong-to-the-project}

Les assignations sont enregistrées dans le fichier du projet, pas dans l’application. Elles ne passent pas dans vos autres projets, et il n’y a pas de configuration globale du contrôleur. Ajouter, changer et supprimer une assignation sont des modifications normales, que vous pouvez annuler.

## un potentiomètre, une cible {#one-knob-one-target}

Chaque commande de votre contrôleur (son port, son canal et son numéro de CC ou sa note) ne peut piloter qu’une seule cible. Si vous faites apprendre un potentiomètre déjà assigné, il passe à la nouvelle cible.

## mode, minimum et maximum {#mode-minimum-and-maximum}

Chaque assignation de la liste a :

- un **mode** :
  - **absolu** : la position du potentiomètre fixe la valeur (la plupart des potentiomètres et des faders).
  - **relatif (compl. à 2)**, **relatif (décalage 64)**, **relatif (bit de signe)** : pour les encodeurs sans fin, qui envoient des pas « vers le haut » ou « vers le bas ». Chaque contrôleur utilise l’un de ces trois codages : consultez son manuel, ou essayez-les.
  - **bascule** : chaque appui active ou désactive la cible (pads, boutons).
- un **minimum** et un **maximum**, en pourcentage de la plage de la cible. Régler le minimum au-dessus du maximum inverse la commande.

Faites un clic droit sur une assignation pour **réapprendre** ou **supprimer l’assignation**, ou cliquez sur son **✕**.

## les touches assignées jouent toujours des notes {#mapped-keys-still-play-notes}

Le MIDI utilisé par une assignation atteint toujours vos pistes. Si vous assignez un pad à lecture/arrêt, appuyer dessus joue aussi une note sur toute piste d’instrument armée ou en monitoring.

## application de bureau uniquement [[desktop]] {#desktop-only}

La version web n’a pas encore d’entrée MIDI matérielle, donc son interrupteur **mode MIDI** est désactivé et indique **l’apprentissage MIDI demande l’application de bureau**.

## et ensuite {#where-next}

- [enregistrement](/docs/making-music/recording) : les entrées MIDI et les filtres de canal de chaque piste.
- [le clavier midi de l’ordinateur](/docs/making-music/computer-midi-keyboard) : jouez des notes sans contrôleur.
- [configuration audio et midi](/docs/making-music/audio-and-midi-setup)
