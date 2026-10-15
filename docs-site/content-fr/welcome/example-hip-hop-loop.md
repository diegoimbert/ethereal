---
title: exemple : boucle hip-hop
order: 41
summary: Refaites l’exemple boucle hip-hop à partir d’un projet vide : un rythme boom-bap swingué sous deux accords doux.
platform: all
audience: beginner
covers:
---

La **boucle hip-hop** est l’une des boucles d’exemple de l’écran d’accueil : 90 bpm, deux mesures, un rythme de batterie swingué et deux accords de septième doux, faits uniquement avec les instruments fournis avec Ethereal. Sur l’écran d’accueil, sous **exemples**, cliquez sur **boucle hip-hop** pour ouvrir la boucle terminée, ou sur **pas à pas** pour une visite guidée. Cette page la refait à partir d’un projet vide, étape par étape.

Il ne vous faut rien d’autre : les sons de batterie sont des presets du **synthé poly**, donc tout fonctionne dans l’application de bureau comme dans la version web.

## 1. créer un projet et régler le tempo {#1-start-a-project-and-set-the-tempo}

1. Sur l’écran d’accueil, cliquez sur **nouveau projet**. Le projet contient une piste `synth` : cliquez sur son en-tête et appuyez sur [[action:edit.delete]], ce guide crée ses propres pistes.
2. Dans la barre du haut, cliquez sur le champ **bpm**, tapez `90` et appuyez sur [[key:Enter]] (voir [tempo et signature rythmique](/docs/basics/tempo-and-time-signature)).
3. Activez le bouton **boucle** de la barre du haut, puis faites glisser la fin de l’accolade de boucle, au-dessus des pistes, jusqu’à la fin de la mesure 2. Voir [lire et se déplacer](/docs/basics/playing-and-navigating#looping-a-section).

## 2. créer le kit de batterie {#2-make-the-drum-kit}

1. Cliquez sur **ajouter une piste** au-dessus des en-têtes de pistes et choisissez **piste MIDI**. La bibliothèque d’instruments s’ouvre : choisissez **drum rack**. Sa fenêtre s’ouvre avec une grille de pads vides.
2. Double-cliquez sur le nom de la nouvelle piste et appelez-la `drums`.
3. Cliquez sur le pad **C1**. Sous la grille, la chaîne du pad a un emplacement d’instrument : cliquez sur **ajouter un instrument**, choisissez **synthé poly**, puis ouvrez **presets** sur la carte du poly synth et choisissez `kick`.
4. Faites de même pour les autres sons :

| pad | preset |
|---|---|
| C1 | kick |
| D1 | snare |
| F#1 | closed hat |

Plus de détails sur les pads dans [le drum rack](/docs/making-music/drum-rack#pads).

## 3. dessiner le rythme {#3-draw-the-beat}

1. Double-cliquez dans le couloir vide de la piste `drums` au début de la mesure 1 en gardant le second clic enfoncé, puis faites glisser jusqu’à la fin de la mesure 2 : un clip de deux mesures apparaît (voir [les clips](/docs/making-music/clips)).
2. Cliquez sur le clip. Il s’ouvre dans l’**éditeur de rythmes** : une ligne par pad, une case par double croche. Cliquez sur une case pour ajouter un coup, cliquez de nouveau pour le retirer.
3. Dessinez ce motif (`x` est un coup, `o` un coup plus doux, `.` un silence ; chaque groupe de quatre cases est un temps). La grosse caisse change à la mesure 2 ; la caisse claire et les charlestons sont les mêmes dans les deux mesures :

```
bar 1
kick        x . . .  . . . x  . . x .  . . . .
snare       . . . .  x . . .  . . . .  x . . .
closed hat  x . o .  x . o .  x . o .  x . o .

bar 2
kick        x . x .  . . . .  . . x .  . . . .
snare       . . . .  x . . .  . . . .  x . . .
closed hat  x . o .  x . o .  x . o .  x . o .
```

Les charlestons plus doux sont facultatifs : ouvrez l’onglet **piano roll** et baissez leur vélocité (voir [le piano roll](/docs/making-music/piano-roll)).

## 4. ajouter du swing {#4-swing-it}

Quand le clip est sélectionné, l’inspecteur affiche une ligne **groove** : réglez le swing sur **50 %** avec une grille de **1/16**. Tous les clips MIDI du projet jouent maintenant leurs doubles croches à contretemps un peu en retard, ce qui fait rebondir les charlestons. Rien ne bouge dans le clip : remettez 0 % pour un rythme droit. Voir [groove, swing et humanisation](/docs/advanced/groove#project-swing-vs-quantize-swing).

## 5. ajouter le clavier {#5-add-the-keys}

1. Cliquez sur **ajouter une piste**, choisissez **piste MIDI**, puis **synthé poly** dans la bibliothèque d’instruments. Appelez la piste `keys`.
2. Survolez la carte du poly synth dans l’inspecteur, ouvrez **presets** et choisissez **soft keys**.
3. Sous l’instrument, cliquez sur **ajouter un effet audio** et choisissez **réverbération**, puis son preset **small room**. Voir [les effets](/docs/making-music/effects).

## 6. écrire les accords {#6-write-the-chords}

Créez un clip de deux mesures sur la piste `keys` comme à l’étape 3. Il s’ouvre dans le piano roll. Dessinez deux accords de quatre notes (C3 est le do du milieu) :

| commence à | durée | notes |
|---|---|---|
| 1.1.1 | 3 temps et demi | C3, D#3, G3, A#3 |
| 2.1.1 | 3 temps et demi | G#2, C3, D#3, G3 |

Le premier est un do mineur 7, le second un la bémol majeur 7. Plus de détails dans [le piano roll](/docs/making-music/piano-roll).

## 7. équilibrer et écouter {#7-balance-and-play}

Appuyez sur [[action:transport.play]]. Baissez le volume de la piste `drums` à environ -3 dB dans son en-tête, et laissez `keys` à 0 dB (voir [le mixage](/docs/making-music/mixing)). Puis faites-la vôtre : déplacez une grosse caisse, essayez un autre accord, ou changez le swing.

## et ensuite {#where-next}

- [Accords lo-fi](/docs/welcome/example-lofi-chords) : quatre accords à travers trois effets.
- [Groove, swing et humanisation](/docs/advanced/groove) : d’autres façons d’assouplir un rythme.
- [Les effets](/docs/making-music/effects) : tout ce que vous pouvez mettre après un instrument.
