---
title: exemple : rythme house
order: 40
summary: Refaites l’exemple rythme house à partir d’un projet vide : un kit dans un drum rack, un rythme quatre temps et une basse à contretemps.
platform: all
audience: beginner
covers:
---

Le **rythme house** est l’une des boucles d’exemple de l’écran d’accueil : 124 bpm, deux mesures, un kit de batterie et une basse, faits uniquement avec les instruments fournis avec Ethereal. Sur l’écran d’accueil, sous **exemples**, cliquez sur **rythme house** pour ouvrir la boucle terminée, ou sur **pas à pas** pour une visite guidée. Cette page la refait à partir d’un projet vide, étape par étape.

Il ne vous faut rien d’autre : les sons de batterie sont des presets du **synthé poly**, donc tout fonctionne dans l’application de bureau comme dans la version web.

## 1. créer un projet et régler le tempo {#1-start-a-project-and-set-the-tempo}

1. Sur l’écran d’accueil, cliquez sur **nouveau projet**. Le projet contient une piste `synth` : cliquez sur son en-tête et appuyez sur [[action:edit.delete]], ce guide crée ses propres pistes.
2. Dans la barre du haut, cliquez sur le champ **bpm**, tapez `124` et appuyez sur [[key:Enter]] (voir [tempo et signature rythmique](/docs/basics/tempo-and-time-signature)).
3. Activez le bouton **boucle** de la barre du haut, puis faites glisser la fin de l’accolade de boucle, au-dessus des pistes, jusqu’à la fin de la mesure 2. Voir [lire et se déplacer](/docs/basics/playing-and-navigating#looping-a-section).

## 2. créer le kit de batterie {#2-make-the-drum-kit}

1. Cliquez sur **ajouter une piste** au-dessus des en-têtes de pistes et choisissez **piste MIDI**. La bibliothèque d’instruments s’ouvre : choisissez **drum rack**. Sa fenêtre s’ouvre avec une grille de pads vides.
2. Double-cliquez sur le nom de la nouvelle piste et appelez-la `drums` (n’importe quel nom convient).
3. Cliquez sur le pad **C1**. Sous la grille, la chaîne du pad a un emplacement d’instrument : cliquez sur **ajouter un instrument**, choisissez **synthé poly**, puis ouvrez **presets** sur la carte du poly synth et choisissez `kick`.
4. Faites de même pour les autres sons :

| pad | preset |
|---|---|
| C1 | kick |
| D#1 | clap |
| F#1 | closed hat |
| A#1 | open hat |

Maintenez un pad enfoncé pour l’entendre. Plus de détails sur les pads dans [le drum rack](/docs/making-music/drum-rack#pads).

## 3. dessiner le rythme {#3-draw-the-beat}

1. Double-cliquez dans le couloir vide de la piste `drums` au début de la mesure 1 en gardant le second clic enfoncé, puis faites glisser jusqu’à la fin de la mesure 2 : un clip de deux mesures apparaît (voir [les clips](/docs/making-music/clips)).
2. Cliquez sur le clip. Il s’ouvre dans l’**éditeur de rythmes** : une ligne par pad, une case par double croche. Cliquez sur une case pour ajouter un coup, cliquez de nouveau pour le retirer.
3. Dessinez ce motif dans les deux mesures (`x` est un coup, `o` un coup plus doux, `.` un silence ; chaque groupe de quatre cases est un temps) :

```
kick        x . . .  x . . .  x . . .  x . . .
clap        . . . .  x . . .  . . . .  x . . .
closed hat  . o . o  . o . o  . o . o  . o . o
open hat    . . x .  . . x .  . . x .  . . x .
```

Les charlestons plus doux sont facultatifs. Pour les faire, ouvrez l’onglet **piano roll** et baissez leur vélocité (voir [le piano roll](/docs/making-music/piano-roll)).

## 4. ajouter la basse {#4-add-the-bass}

1. Cliquez sur **ajouter une piste**, choisissez **piste MIDI**, puis **synthé poly** dans la bibliothèque d’instruments. Appelez la piste `bass`.
2. Survolez la carte du poly synth dans l’inspecteur, ouvrez **presets** et choisissez **deep sub**.

## 5. écrire la ligne de basse {#5-write-the-bass-line}

Créez un clip de deux mesures sur la piste `bass` comme à l’étape 3. Il s’ouvre dans le piano roll. Dessinez ces notes, chacune d’une double croche (les positions sont mesure.temps.double croche, comme les affiche la barre du haut ; C3 est le do du milieu) :

| commence à | note |
|---|---|
| 1.1.3, 1.2.3, 1.3.3, 1.4.3 | A1 |
| 2.1.3, 2.2.3 | F1 |
| 2.3.3, 2.4.3 | G1 |

Chaque note tombe sur un contretemps, entre deux grosses caisses. Plus de détails dans [le piano roll](/docs/making-music/piano-roll).

## 6. équilibrer et écouter {#6-balance-and-play}

Appuyez sur [[action:transport.play]]. Baissez le volume de la piste `drums` à environ -3 dB et celui de `bass` à environ -4 dB dans leurs en-têtes, pour que le mixage ne sature pas (voir [le mixage](/docs/making-music/mixing)). Puis faites-la vôtre : déplacez un clap, essayez un autre preset de basse, ou ajoutez une troisième mesure.

## et ensuite {#where-next}

- [Boucle hip-hop](/docs/welcome/example-hip-hop-loop) : un rythme swingué avec des accords.
- [Le drum rack](/docs/making-music/drum-rack) : pads, groupes d’étouffement et effets par pad.
- [Les instruments](/docs/making-music/instruments) : ce que le poly synth sait faire.
