---
title: exemple : accords lo-fi
order: 42
summary: Refaites l’exemple accords lo-fi à partir d’un projet vide : quatre accords de septième à travers un chorus, un bitcrusher et une reverb, sur un rythme nonchalant.
platform: all
audience: beginner
covers:
---

La boucle **accords lo-fi** est l’une des boucles d’exemple de l’écran d’accueil : 80 bpm, quatre mesures, quatre accords chauds patinés par trois effets, sur un rythme nonchalant et swingué. Tout est fourni avec Ethereal. Sur l’écran d’accueil, sous **exemples**, cliquez sur **accords lo-fi** pour ouvrir la boucle terminée, ou sur **pas à pas** pour une visite guidée. Cette page la refait à partir d’un projet vide, étape par étape.

Il ne vous faut rien d’autre : les sons de batterie sont des presets du **synthé poly**, donc tout fonctionne dans l’application de bureau comme dans la version web.

## 1. créer un projet et régler le tempo {#1-start-a-project-and-set-the-tempo}

1. Sur l’écran d’accueil, cliquez sur **nouveau projet**. Le projet contient une piste `synth` : cliquez sur son en-tête et appuyez sur [[action:edit.delete]], ce guide crée ses propres pistes.
2. Dans la barre du haut, cliquez sur le champ **bpm**, tapez `80` et appuyez sur [[key:Enter]] (voir [tempo et signature rythmique](/docs/basics/tempo-and-time-signature)).
3. Activez le bouton **boucle** de la barre du haut. Dans un nouveau projet, l’accolade de boucle au-dessus des pistes couvre quatre mesures, exactement ce qu’il faut ici. Voir [lire et se déplacer](/docs/basics/playing-and-navigating#looping-a-section).

## 2. donner le son des accords {#2-make-the-chords-sound}

1. Cliquez sur **ajouter une piste** au-dessus des en-têtes de pistes et choisissez **piste MIDI**. La bibliothèque d’instruments s’ouvre : choisissez **synthé poly**. Double-cliquez sur le nom de la piste et appelez-la `chords`.
2. Survolez la carte du poly synth dans l’inspecteur, ouvrez **presets** et choisissez **soft keys**.
3. Sous l’instrument, cliquez trois fois sur **ajouter un effet audio** pour ajouter, dans cet ordre, chacun avec son preset :

| effet | preset |
|---|---|
| chorus | classic chorus |
| bitcrusher | lo-fi sampler |
| reverb | plate |

Le son les traverse de haut en bas : le chorus l’élargit et le fait onduler, le bitcrusher l’use comme un vieil échantillonneur, la reverb le place dans une pièce. Voir [les effets](/docs/making-music/effects).

## 3. écrire les accords {#3-write-the-chords}

Double-cliquez dans le couloir vide de la piste `chords` au début de la mesure 1 en gardant le second clic enfoncé, puis faites glisser jusqu’à la fin de la mesure 4 : un clip de quatre mesures apparaît (voir [les clips](/docs/making-music/clips)). Cliquez dessus pour l’ouvrir dans le piano roll, et dessinez un accord par mesure, chacun tenu 3 temps trois quarts (C3 est le do du milieu) :

| mesure | accord | notes |
|---|---|---|
| 1 | fa majeur 7 | F2, A2, C3, E3 |
| 2 | mi mineur 7 | E2, G2, B2, D3 |
| 3 | ré mineur 7 | D2, F2, A2, C3 |
| 4 | do majeur 7 | C2, E2, G2, B2 |

Chaque accord est le précédent descendu d’un degré dans la gamme. Plus de détails dans [le piano roll](/docs/making-music/piano-roll).

## 4. ajouter un rythme nonchalant {#4-add-a-lazy-beat}

1. Cliquez sur **ajouter une piste**, choisissez **piste MIDI**, puis **drum rack**. Appelez la piste `drums`.
2. Cliquez sur le pad **C1**, cliquez sur **ajouter un instrument** dans la chaîne du pad sous la grille, choisissez **synthé poly**, puis ouvrez **presets** sur sa carte et choisissez `kick`. Faites de même pour **D1** avec `snare` et **F#1** avec `closed hat`. Plus de détails dans [le drum rack](/docs/making-music/drum-rack#pads).
3. Créez un clip de quatre mesures sur la piste `drums` et cliquez dessus : il s’ouvre dans l’**éditeur de rythmes**. Dessinez ce motif dans chacune des quatre mesures (`x` est un coup, `o` un coup plus doux, `.` un silence ; chaque groupe de quatre cases est un temps) :

```
kick        x . . .  . . . .  . . x .  . . . .
snare       . . . .  x . . .  . . . .  x . . .
closed hat  o . o .  o . o .  o . o .  o . o o
```

Les charlestons doux sont facultatifs : ouvrez l’onglet **piano roll** et baissez leur vélocité (voir [le piano roll](/docs/making-music/piano-roll)).

## 5. swing et équilibre {#5-swing-and-balance}

1. Sélectionnez un clip : l’inspecteur affiche une ligne **groove**. Réglez le swing sur **35 %** avec une grille de **1/16**. Voir [groove, swing et humanisation](/docs/advanced/groove#project-swing-vs-quantize-swing).
2. Dans les en-têtes de pistes, montez le volume de `chords` à environ +4 dB et baissez `drums` à environ -8 dB, pour que les accords mènent (voir [le mixage](/docs/making-music/mixing)).
3. Appuyez sur [[action:transport.play]].

Puis faites-la vôtre : essayez d’autres accords, un autre preset sur le bitcrusher, ou plus de reverb.

## et ensuite {#where-next}

- [Ligne de basse simple](/docs/welcome/example-simple-bassline) : une basse acid sur une grosse caisse.
- [Les effets](/docs/making-music/effects) : tous les effets intégrés.
- [Appareils et presets](/docs/making-music/devices-and-presets#presets) : enregistrez votre propre version du son.
