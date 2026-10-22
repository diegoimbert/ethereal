---
title: référence des réglages
order: 90
summary: Chaque page de la fenêtre des réglages et ce que font ses options, avec des liens vers les pages qui les expliquent.
platform: all
audience: everyone
covers:
  - app-shell/debug-panels-advanced #advanced
---

Ouvrez les réglages avec le bouton en forme de roue dentée à droite de la barre d’état (en bas), ou lancez **réglages audio…** ou **réglages de la souris et de la molette…** depuis la [palette de commandes](/docs/basics/command-palette-and-settings). Les réglages sont rangés en catégories (la liste à gauche) ; une catégorie qui a plusieurs pages les liste en dessous quand vous l’ouvrez. Chaque changement s’applique immédiatement, et **terminé** ferme la fenêtre. Les options qui ont besoin du moteur audio indiquent **aucun moteur connecté** quand il ne tourne pas.

Certains réglages appartiennent à un projet (tempo, swing, assignations MIDI) et se trouvent dans le projet, pas ici.

## général {#general}

**langue et démarrage** :

- **langue** : la langue de l’interface, sur cet appareil.
- **démarrage** : **rouvrir le dernier projet au lancement** (désactivé par défaut). Voir [projets et enregistrement](/docs/basics/projects).

**bibliothèque** :

- **stockage** [[web]] : si le navigateur a accepté de garder vos projets, l’espace qu’ils occupent, et **redemander** si ce n’est pas le cas. Voir [où vivent vos projets](/docs/welcome/where-your-projects-live).
- **fichiers de la bibliothèque** : les fichiers que liste le navigateur. Chaque règle est un type et son texte : un **type de fichier** (des extensions comme `wav, aif`), ou un nom qui **commence par**, **finit par**, **est** ou **contient** un texte. **utiliser des regex (avancé)** affiche plutôt les règles sous forme d’expressions régulières. **rétablir les réglages par défaut** demande d’abord, puis rétablit les règles par défaut. Voir [le navigateur](/docs/sounds-and-plugins/browser).

## apparence {#appearance}

- **thème** : l’un des six thèmes intégrés ou l’un des vôtres (**créer votre thème**, **importer un thème…**), ou suivre le mode clair ou sombre du système. Voir [thèmes](/docs/advanced/themes).
- **zoom** : met à l’échelle toute l’interface (texte, commandes et panneaux).
- **animations** : classiques, rapides (chaque animation dure moitié moins longtemps) ou mouvements réduits (les changements s’appliquent immédiatement).
- **taches de couleur** : les couleurs douces qui flottent derrière les boîtes de dialogue et, discrètement, derrière l’arrangement.
- **halo** : le halo autour des notes, des clips et des pistes sélectionnés, de la tête de lecture et des voyants d’état.
- **transparence** : le degré de transparence des panneaux, ou **désactiver la transparence**.

Ces réglages sont gardés sur cet appareil.

## audio {#audio}

Le pilote audio, le **périphérique de sortie**, le **périphérique d’entrée**, la **fréquence d’échantillonnage** et la **taille du buffer**, l’état du moteur et un test d’entrée. Dans le navigateur, c’est votre système qui choisit les périphériques. Voir [configuration audio et midi](/docs/making-music/audio-and-midi-setup).

## souris et clavier {#mouse-keyboard}

**souris et trackpad** : le comportement de la molette et des boutons de la souris, gardé sur cet appareil :

- **molette** : sensibilité du zoom et du défilement, si la molette seule fait défiler ou zoomer, quel modificateur fait l’autre, zoom et défilement inversés, et si [[key:Shift]] + molette fait défiler horizontalement.
- **boutons de la souris** : ce que fait un glissement avec le bouton du milieu (déplacer la vue, ou rien) et ce que font les boutons précédent / suivant (rien, ou annuler / rétablir).

**raccourcis clavier** : **modifier les raccourcis clavier…** ouvre l’éditeur de raccourcis. Voir [raccourcis clavier](/docs/reference/keyboard-shortcuts).

## plug-ins [[desktop]] {#plugins}

**dossiers et analyse** : les dossiers analysés pour trouver des plug-ins : les dossiers du système (chacun peut être désactivé) et vos propres dossiers, ainsi que les boutons d’analyse. Dans la version web, cette page indique seulement que les plug-ins demandent l’application de bureau. Voir [plug-ins](/docs/sounds-and-plugins/plugins).

**liste des plug-ins** : les plug-ins que la bibliothèque d’appareils affiche, conservé sur cet appareil. Voir [le même plug-in dans plusieurs formats](/docs/sounds-and-plugins/plugins#the-same-plugin-in-several-formats).

- **afficher chaque plug-in une seule fois** (activé par défaut) : un plug-in installé dans plusieurs formats n’est listé qu’une fois, dans le **format préféré** (clap, puis vst3, au, vst2). Désactivez-le pour lister tous les formats et en choisir un précis.
- **plug-ins masqués** : les plug-ins que la bibliothèque n’affiche pas, chacun dans **tous les formats** ou dans un seul format. **masquer un plug-in…** en ajoute un, **×** l’affiche de nouveau, et **réinitialiser** (après confirmation) rétablit les valeurs par défaut.

## avancé {#advanced}

Pour des configurations particulières. La plupart des gens n’y touchent jamais. **moteur** :

- **serveur de moteur** : fait tourner cette fenêtre sur un moteur situé sur une autre machine. Masqué tant que vous n’avez pas lancé **serveur de moteur…** depuis la palette de commandes (ou utilisé un serveur auparavant). Voir [le serveur de moteur](/docs/advanced/engine-server).

**diagnostic** :

- **journal du moteur** [[desktop]] : **ouvrir le journal du moteur** affiche ce que signale le moteur (avertissements, problèmes de plug-ins, raison pour laquelle il a cessé de répondre) dans une fenêtre de terminal. Voir [dépannage](/docs/reference/troubleshooting).
- **panneaux de débogage** : des outils pour diagnostiquer les problèmes, surtout destinés aux développeurs.

## autres réglages {#other-settings}

Quelques réglages se trouvent là où vous les utilisez :

- le fournisseur d’IA, le modèle, la clé et l’interrupteur des agents externes : dans les réglages du panneau **demander à l’IA**. Voir [l’assistant ia](/docs/advanced/ai-assistant).
- les raccourcis clavier : **modifier les raccourcis clavier…** dans **souris et clavier**, ou **raccourcis clavier…** dans la palette de commandes. Voir [raccourcis clavier](/docs/reference/keyboard-shortcuts).
- le métronome : un clic droit sur le bouton du métronome, dans la barre de transport. Voir [lire et se déplacer](/docs/basics/playing-and-navigating).

## et ensuite {#where-next}

- [configuration audio et midi](/docs/making-music/audio-and-midi-setup)
- [dépannage](/docs/reference/troubleshooting)
