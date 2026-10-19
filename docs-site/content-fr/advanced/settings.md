---
title: référence des réglages
order: 90
summary: Chaque onglet de la fenêtre des réglages et ce que font ses options, avec des liens vers les pages qui les expliquent.
platform: all
audience: everyone
covers:
  - app-shell/debug-panels-advanced #advanced
---

Ouvrez les réglages avec le bouton en forme de roue dentée dans la barre du haut, ou lancez **réglages audio…** ou **réglages de la souris et de la molette…** depuis la [palette de commandes](/docs/basics/command-palette-and-settings). Chaque changement s’applique immédiatement, et **terminé** ferme la fenêtre. Les options qui ont besoin du moteur audio indiquent **aucun moteur connecté** quand il ne tourne pas.

Certains réglages appartiennent à un projet (tempo, swing, assignations MIDI) et se trouvent dans le projet, pas ici.

## général {#general}

- **démarrage** : **rouvrir le dernier projet au lancement** (désactivé par défaut). Voir [projets et enregistrement](/docs/basics/projects).
- **stockage** [[web]] : si le navigateur a accepté de garder vos projets, l’espace qu’ils occupent, et **redemander** si ce n’est pas le cas. Voir [où vivent vos projets](/docs/welcome/where-your-projects-live).
- **fichiers de la bibliothèque** : les fichiers que liste le navigateur, sous forme de règles appliquées aux noms de fichiers. **rétablir les réglages par défaut** annule vos changements. Voir [le navigateur](/docs/sounds-and-plugins/browser).

## apparence {#appearance}

- **thème** : sombre ou clair.
- **zoom** : met à l’échelle toute l’interface (texte, commandes et panneaux).
- **animations** : classiques, rapides (chaque animation dure moitié moins longtemps) ou mouvements réduits (les changements s’appliquent immédiatement).
- **taches de couleur** : les couleurs douces qui flottent derrière les boîtes de dialogue et, discrètement, derrière l’arrangement.
- **halo** : le halo autour des notes, des clips et des pistes sélectionnés, de la tête de lecture et des voyants d’état.
- **transparence** : le degré de transparence des panneaux, ou **désactiver la transparence**.

Ces réglages sont gardés sur cet appareil.

## audio {#audio}

Le pilote audio, le **périphérique de sortie**, le **périphérique d’entrée**, la **fréquence d’échantillonnage** et la **taille du buffer**, l’état du moteur et un test d’entrée. Dans le navigateur, c’est votre système qui choisit les périphériques. Voir [configuration audio et midi](/docs/making-music/audio-and-midi-setup).

## souris {#input}

Le comportement de la molette et des boutons de la souris, gardé sur cet appareil :

- **molette** : sensibilité du zoom et du défilement, si la molette seule fait défiler ou zoomer, quel modificateur fait l’autre, zoom et défilement inversés, et si [[key:Shift]] + molette fait défiler horizontalement.
- **boutons de la souris** : ce que fait un glissement avec le bouton du milieu (déplacer la vue, ou rien) et ce que font les boutons précédent / suivant (rien, ou annuler / rétablir).

## plug-ins [[desktop]] {#plugins}

Les dossiers analysés pour trouver des plug-ins : les dossiers du système (chacun peut être désactivé) et vos propres dossiers, ainsi que les boutons d’analyse. Dans la version web, cet onglet indique seulement que les plug-ins demandent l’application de bureau. Voir [plug-ins](/docs/sounds-and-plugins/plugins).

## avancé {#advanced}

Pour des configurations particulières. La plupart des gens n’y touchent jamais :

- **serveur de moteur** : fait tourner cette fenêtre sur un moteur situé sur une autre machine. Masqué tant que vous n’avez pas lancé **serveur de moteur…** depuis la palette de commandes (ou utilisé un serveur auparavant). Voir [le serveur de moteur](/docs/advanced/engine-server).
- **journal du moteur** [[desktop]] : **ouvrir le journal du moteur** affiche ce que signale le moteur (avertissements, problèmes de plug-ins, raison pour laquelle il a cessé de répondre) dans une fenêtre de terminal. Voir [dépannage](/docs/reference/troubleshooting).
- **panneaux de débogage** : des outils pour diagnostiquer les problèmes, surtout destinés aux développeurs.

## autres réglages {#other-settings}

Quelques réglages se trouvent là où vous les utilisez :

- le fournisseur d’IA, le modèle, la clé et l’interrupteur des agents externes : dans les réglages du panneau **demander à l’IA**. Voir [l’assistant ia](/docs/advanced/ai-assistant).
- les raccourcis clavier : **modifier les raccourcis clavier…** dans l’onglet **souris**, ou **raccourcis clavier…** dans la palette de commandes. Voir [raccourcis clavier](/docs/reference/keyboard-shortcuts).
- le métronome : dans la barre de transport. Voir [lire et se déplacer](/docs/basics/playing-and-navigating).

## et ensuite {#where-next}

- [configuration audio et midi](/docs/making-music/audio-and-midi-setup)
- [dépannage](/docs/reference/troubleshooting)
