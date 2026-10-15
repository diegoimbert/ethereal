---
title: exemple : ligne de basse simple
order: 43
summary: Refaites l’exemple ligne de basse simple à partir d’un projet vide : une basse acid en croches sur une grosse caisse et des charlestons.
platform: all
audience: beginner
covers:
---

La **ligne de basse simple** est l’une des boucles d’exemple de l’écran d’accueil : 120 bpm, deux mesures, une ligne de basse acid sur le rythme le plus simple, faite uniquement avec les instruments fournis avec Ethereal. Sur l’écran d’accueil, sous **exemples**, cliquez sur **ligne de basse simple** pour ouvrir la boucle terminée, ou sur **pas à pas** pour une visite guidée. Cette page la refait à partir d’un projet vide, étape par étape.

Il ne vous faut rien d’autre : les sons de batterie sont des presets du **synthé poly**, donc tout fonctionne dans l’application de bureau comme dans la version web.

## 1. créer un projet et régler le tempo {#1-start-a-project-and-set-the-tempo}

1. Sur l’écran d’accueil, cliquez sur **nouveau projet**. Le projet contient une piste `synth` : cliquez sur son en-tête et appuyez sur [[action:edit.delete]], ce guide crée ses propres pistes.
2. Un nouveau projet joue déjà à 120 bpm : rien à changer. Les autres tempos se règlent dans le champ **bpm** de la barre du haut (voir [tempo et signature rythmique](/docs/basics/tempo-and-time-signature)).
3. Activez le bouton **boucle** de la barre du haut, puis faites glisser la fin de l’accolade de boucle, au-dessus des pistes, jusqu’à la fin de la mesure 2. Voir [lire et se déplacer](/docs/basics/playing-and-navigating#looping-a-section).

## 2. créer le rythme {#2-make-the-beat}

1. Cliquez sur **ajouter une piste** au-dessus des en-têtes de pistes et choisissez **piste MIDI**. La bibliothèque d’instruments s’ouvre : choisissez **drum rack**. Double-cliquez sur le nom de la piste et appelez-la `drums`.
2. Cliquez sur le pad **C1**. Sous la grille, la chaîne du pad a un emplacement d’instrument : cliquez sur **ajouter un instrument**, choisissez **synthé poly**, puis ouvrez **presets** sur sa carte et choisissez `kick`. Faites de même pour **F#1** avec `closed hat`. Plus de détails dans [le drum rack](/docs/making-music/drum-rack#pads).
3. Double-cliquez dans le couloir vide de la piste `drums` au début de la mesure 1 en gardant le second clic enfoncé, puis faites glisser jusqu’à la fin de la mesure 2 : un clip de deux mesures apparaît (voir [les clips](/docs/making-music/clips)).
4. Cliquez sur le clip : il s’ouvre dans l’**éditeur de rythmes**. Dessinez ce motif dans les deux mesures (`x` est un coup, `.` un silence ; chaque groupe de quatre cases est un temps) :

```
kick        x . . .  x . . .  x . . .  x . . .
closed hat  . . x .  . . x .  . . x .  . . x .
```

## 3. ajouter la basse {#3-add-the-bass}

1. Cliquez sur **ajouter une piste**, choisissez **piste MIDI**, puis **synthé poly** dans la bibliothèque d’instruments. Appelez la piste `bass`.
2. Survolez la carte du poly synth dans l’inspecteur, ouvrez **presets** et choisissez **acid bass** : une onde en dents de scie dans un filtre résonant qui claque à chaque note. Voir [les instruments](/docs/making-music/instruments).

## 4. écrire la ligne de basse {#4-write-the-bassline}

Créez un clip de deux mesures sur la piste `bass` comme à l’étape 2. Il s’ouvre dans le piano roll. Dessinez ces notes (les positions sont mesure.temps.double croche, comme les affiche la barre du haut ; une croche dure deux doubles croches ; C3 est le do du milieu) :

| commence à | durée | note |
|---|---|---|
| 1.1.1 | croche | E1 |
| 1.1.4 | double croche | E1 |
| 1.2.3 | croche | G1 |
| 1.3.1 | croche | E1 |
| 1.3.4 | double croche | E1 |
| 1.4.1 | croche | A1 |
| 1.4.3 | croche | G1 |
| 2.1.1 | croche | E1 |
| 2.1.4 | double croche | E1 |
| 2.2.3 | croche | B1 |
| 2.3.1 | croche | A1 |
| 2.3.3 | croche | G1 |
| 2.4.1 | croche | D1 |
| 2.4.3 | croche | E1 |

Surtout une note grave, E1, avec quelques notes plus aiguës qui lui répondent. Plus de détails dans [le piano roll](/docs/making-music/piano-roll).

## 5. équilibrer et écouter {#5-balance-and-play}

Dans les en-têtes de pistes, baissez le volume de `drums` à environ -4 dB et celui de `bass` à environ -4 dB (voir [le mixage](/docs/making-music/mixing)). Appuyez sur [[action:transport.play]]. Puis faites-la vôtre : faites glisser des notes vers le haut ou le bas, vous les entendez en les déplaçant, ou essayez le preset **reese bass**.

## et ensuite {#where-next}

- [Rythme house](/docs/welcome/example-house-beat) : un kit house complet avec une basse à contretemps.
- [Le piano roll](/docs/making-music/piano-roll) : sélectionner, déplacer et quantifier des notes.
- [L’automation](/docs/making-music/automation) : ouvrir le filtre au fil des mesures.
