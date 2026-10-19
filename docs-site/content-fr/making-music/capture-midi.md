---
title: capturer le midi
order: 160
summary: Transformez ce que vous venez de jouer en clip, même si vous n’enregistriez pas.
platform: all
audience: everyone
covers:
  - recording/capture-midi-rescue-what-you-just-played #rescue-what-you-just-played
  - recording/capture-while-stopped-sets-the-tempo-empty-projects-only #played-while-stopped
  - recording/capture-keeps-only-the-latest-take-then-empties #what-capture-keeps
  - recording/capture-respects-the-target-track-s-midi-filter #when-capture-finds-nothing
---

Vous avez joué quelque chose de génial en improvisant, et vous n’enregistriez pas. Capturer le MIDI vous le rend. Ethereal écoute en permanence le MIDI que vous jouez, pour que vous puissiez en faire un clip après coup.

Cela fonctionne avec un contrôleur MIDI dans l’application de bureau, et avec le [clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard) partout, y compris sur le web.

## récupérer ce que vous venez de jouer {#rescue-what-you-just-played}

Cliquez sur le bouton de capture dans la barre de transport. Son infobulle indique « capturer le MIDI : transformer ce que vous venez de jouer en clip ». Le bouton reste estompé tant que vous n’avez rien joué. Il est aussi dans la palette de commandes ([[action:palette.open]]) sous le nom **capturer le MIDI**.

Inutile d’armer une piste ou d’appuyer sur enregistrer. Le nouveau clip va :

1. sur la piste sélectionnée, si c’est une piste MIDI,
2. sinon sur la première piste MIDI armée,
3. sinon sur la première piste MIDI,
4. sinon sur une nouvelle piste MIDI.

Le nouveau clip est sélectionné. Toute la capture est une seule étape de l’[historique d’annulation](/docs/basics/undo-and-history).

## joué pendant la lecture du morceau {#played-while-the-song-was-playing}

Les notes jouées pendant que le transport tournait gardent leur position dans le morceau. Le clip couvre les mesures que vous avez jouées. Si vous avez joué sur une boucle, les passages sont fusionnés dans la boucle.

## joué à l’arrêt {#played-while-stopped}

Les notes jouées pendant que le morceau était arrêté deviennent un clip à la tête de lecture. La capture prend votre dernière phrase : une pause de plus de 6 secondes commence une nouvelle phrase, et tout ce qui la précède est laissé de côté. La longueur du clip est arrondie à la mesure entière supérieure.

Si le projet est vide (aucun clip), la capture devine aussi le tempo à partir de ce que vous avez joué, entre 60 et 180 bpm, en prenant votre première note comme premier temps fort. Elle règle ensuite le tempo du projet et place la boucle autour du nouveau clip. Dans un projet qui contient déjà des clips, la capture ne change jamais le tempo.

## ce que la capture conserve {#what-capture-keeps}

- Chaque lecture ou arrêt commence une nouvelle prise de capture. La capture n’utilise que la dernière prise qui contient des notes, pas quelque chose d’il y a plusieurs cycles lecture/arrêt.
- Après une capture, la mémoire tampon est vidée, et le bouton s’estompe jusqu’à ce que vous jouiez à nouveau.
- Elle contient jusqu’à 10 minutes de jeu.
- Elle est effacée quand vous changez de projet. Elle n’est jamais sauvegardée.

Le pitch bend, la molette de modulation et les autres contrôleurs que vous avez joués sont aussi capturés, sous forme de [couloirs d’expression](/docs/making-music/expression-and-mpe) sur le clip.

## quand la capture ne trouve rien {#when-capture-finds-nothing}

La capture ne garde que le MIDI que la piste cible accepterait : son port d’entrée et son filtre de canal (voir [filtres d’entrée MIDI](/docs/making-music/recording#midi-input-filters)). Si la piste sélectionnée écoute un autre port ou un autre canal que celui sur lequel vous avez joué, la capture indique que rien n’a été joué. Sélectionnez une autre piste, ou réglez son entrée sur **tous les ports** et **tous les canaux**.

## et ensuite {#where-next}

- [Le piano roll](/docs/making-music/piano-roll) : nettoyer les notes capturées.
- [Enregistrement](/docs/making-music/recording) : enregistrer volontairement, avec décompte et prises.
