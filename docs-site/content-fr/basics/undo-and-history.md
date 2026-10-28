---
title: annuler et historique
order: 70
summary: Annulez et rétablissez n’importe quelle modification, revenez à n’importe quelle étape avec le panneau d’historique, et nommez des points de repère.
platform: all
audience: beginner
covers:
  - app-shell/undo-and-redo #undo-and-redo
  - app-shell/history-panel-jump-to-any-step #the-history-panel
  - app-shell/checkpoints-in-the-undo-history #checkpoints
  - app-shell/undo-history-isn-t-saved #the-history-isnt-saved
---

Toute modification de votre morceau peut être annulée. Cette page explique ce que couvre l’annulation, comment revenir plusieurs étapes en arrière d’un coup, et ses limites. Pour revenir plus loin que la session en cours, voir [versions et sauvegardes](/docs/basics/versions-and-backups).

## annuler et rétablir {#undo-and-redo}

- [[action:edit.undo]] annule la dernière modification.
- [[action:edit.redo]] la rétablit.

Les boutons **annuler** et **rétablir** de la barre du haut font la même chose. Survolez-les pour voir quelle étape ils annuleraient ou rétabliraient, par exemple « annuler déplacer 3 clips ».

Toute modification du morceau peut être annulée : notes, clips, pistes, appareils et leurs réglages, automation, zone de boucle, tempo et signature rythmique. Un glissement ou un tour de bouton est une seule étape, quelle que soit sa durée.

Certaines choses ne sont pas des modifications du morceau, et ne figurent donc pas dans l’historique d’annulation : couper ou mettre en solo une piste ou un pad (annuler ne change jamais ce que vous entendez), armer une piste pour l’enregistrement, lancer et arrêter la lecture, déplacer la tête de lecture, ouvrir les fenêtres de plug-ins, les réglages audio, et les actions sur les projets comme ouvrir, supprimer ou dupliquer un projet.

## le panneau d’historique {#the-history-panel}

L’icône **historique** de la barre latérale gauche ouvre le panneau d’historique. Il liste vos modifications par nom, la plus ancienne en haut :

- **Cliquez** sur une étape pour tout annuler ou rétablir jusqu’à ce point. Les étapes suivantes s’affichent comme annulées jusqu’à votre prochaine modification.
- **Faites un clic droit** sur une étape pour **annuler jusqu’ici** ou **rétablir jusqu’ici**.
- La première ligne, **projet ouvert**, annule tout depuis l’ouverture du projet.

Le bas du panneau compte les étapes et combien sont annulées.

### points de repère {#checkpoints}

Pour retrouver facilement un moment important, donnez-lui un nom : cliquez sur le **drapeau** d’une étape, ou faites un clic droit dessus et choisissez **nommer un point de repère…**. Le nom s’affiche dans la liste à côté de l’étape. **Nommer un point de repère…** dans la palette de commandes nomme l’étape actuelle depuis n’importe où.

Faites un clic droit sur un point de repère pour le **renommer** ou le **retirer**.

## l’historique n’est pas enregistré {#the-history-isnt-saved}

L’historique d’annulation ne dure que tant que le projet est ouvert. Recharger la page, ouvrir un autre projet ou quitter l’efface, points de repère compris. Le panneau vous le rappelle : **non enregistré avec le projet**.

Seules les 500 étapes les plus récentes sont conservées. Au-delà, les plus anciennes sont retirées et le panneau indique **les étapes plus anciennes ont été retirées de l’historique** ; sa première ligne devient alors **plus ancien état conservé**.

Pour garder plus longtemps un état de votre morceau, enregistrez un [instantané](/docs/basics/versions-and-backups#snapshots).

## et ensuite {#where-next}

- [Versions et sauvegardes](/docs/basics/versions-and-backups)
- [La palette de commandes et les réglages](/docs/basics/command-palette-and-settings)
