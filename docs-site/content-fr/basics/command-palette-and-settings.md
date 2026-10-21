---
title: la palette de commandes et les réglages
order: 80
summary: Lancez n’importe quelle commande en tapant son nom, trouvez les commandes qui n’existent que dans la palette, et parcourez les réglages.
platform: all
audience: beginner
covers:
  - app-shell/command-palette-cmd-ctrl-k #the-command-palette
  - shortcuts-commands/command-palette-k-ctrl-k #the-command-palette
  - shortcuts-commands/commands-only-in-the-palette #commands-only-in-the-palette
  - app-shell/settings-overview #settings
---

La palette de commandes est un champ de recherche pour tout ce que l’application sait faire. C’est le moyen le plus rapide de trouver une commande quand vous ne savez pas où est son bouton, et quelques commandes ne se trouvent que là. Cette page fait aussi le tour des réglages.

## la palette de commandes {#the-command-palette}

Appuyez sur [[action:palette.open]], ou cliquez sur le dernier bouton du rail de gauche, pour ouvrir la palette. Ensuite :

1. Tapez une partie du nom d’une commande. Les lettres n’ont pas besoin de se suivre : « mtr » trouve **activer le métronome**. Le nom anglais d’une commande la trouve aussi (« print shortcuts » trouve **imprimer les raccourcis clavier**).
2. Utilisez [[key:ArrowUp]] et [[key:ArrowDown]] pour parcourir les résultats.
3. Appuyez sur [[key:Enter]] pour lancer la commande en surbrillance, ou sur [[key:Escape]] pour fermer la palette.

Les commandes lancées récemment apparaissent en premier. Quand une commande a un raccourci clavier, la palette l’affiche, selon vos propres [raccourcis clavier](/docs/reference/keyboard-shortcuts).

La palette ne s’ouvre pas pendant que vous tapez dans un champ de texte : le raccourci ne gêne jamais votre saisie.

La palette liste des commandes pour les pistes, le transport, l’édition, le projet, les panneaux, les modèles, les appareils (**ajouter …** insère un appareil sur la piste sélectionnée), l’apparence et l’aide.

## commandes propres à la palette {#commands-only-in-the-palette}

Ces commandes n’ont de bouton ni d’entrée de menu nulle part ailleurs :

- **imprimer les raccourcis clavier** ;
- **réglages de la souris et de la molette…** ;
- **enregistrer sous…**, **dupliquer le projet**, **renommer le projet** et **nouveau projet depuis un modèle…** ;
- **relier les échantillons manquants…** et **tout rassembler et enregistrer** ;
- **ajuster le zoom**, pour voir tout le morceau ;
- **aller à la piste …**, pour sauter à une piste par son nom ;
- **épingler** et **désépingler** le navigateur, l’inspecteur et le tiroir d’édition ;
- **nommer un point de repère…** (voir [annulation et historique](/docs/basics/undo-and-history#checkpoints)) ;
- **activer le mode MIDI** et **désactiver le mode MIDI** (voir [apprentissage MIDI](/docs/advanced/midi-learn)) [[desktop]] ;
- **passer au thème clair** et **passer au thème sombre** ;
- **debug : …**, les panneaux de débogage (pour le dépannage).

## réglages {#settings}

La **roue dentée** à l’extrémité droite de la barre d’état (en bas) ouvre les réglages. Ils sont rangés en catégories, listées à gauche ; une catégorie qui a plusieurs pages les liste en dessous quand vous l’ouvrez :

| catégorie | contenu |
|---|---|
| **général** | **langue et démarrage** : la langue de l’interface, et **démarrage** (rouvrir le dernier projet au lancement, voir [projets et enregistrement](/docs/basics/projects#reopening-your-last-project)). **bibliothèque** : dans le navigateur, **stockage** (si le navigateur garde vos projets en sécurité, voir [où vivent vos projets](/docs/welcome/where-your-projects-live)) ; **fichiers de la bibliothèque** (les fichiers que liste le navigateur). |
| **apparence** | le thème (clair ou sombre), le zoom de l’interface, les animations, les taches de couleur et le halo, et la transparence. |
| **audio** | le pilote, les périphériques de sortie et d’entrée, la fréquence d’échantillonnage et la taille du buffer, et un contrôle de l’entrée. Dans le navigateur, c’est le système qui choisit les périphériques. Voir [configuration audio et MIDI](/docs/making-music/audio-and-midi-setup). |
| **souris et clavier** | **souris et trackpad** : la molette et les boutons de la souris : sensibilité du zoom et du défilement, modificateur qui zoome, sens inversés, bouton du milieu et boutons précédent et suivant. Voir [zoom et défilement](/docs/basics/playing-and-navigating#zoom-and-scroll). **raccourcis clavier** : **modifier les raccourcis clavier…** ouvre l’éditeur de raccourcis (aussi **raccourcis clavier…** dans la palette). |
| **plug-ins** | [[desktop]] les dossiers analysés à la recherche de plug-ins, et la nouvelle analyse. Voir [les plug-ins](/docs/sounds-and-plugins/plugins). |
| **avancé** | **moteur** : le serveur de moteur (une fois activé). **diagnostic** : le journal du moteur (application de bureau) et les panneaux de débogage. Voir [la référence des réglages](/docs/advanced/settings). |

Les réglages d’apparence, de souris et de clavier sont gardés sur cet ordinateur (ou dans ce navigateur). Les autres réglages demandent que le moteur audio tourne.

## et ensuite {#where-next}

- [Raccourcis clavier](/docs/reference/keyboard-shortcuts)
- [La référence des réglages](/docs/advanced/settings)
- [Une visite de l’écran](/docs/basics/the-screen)
