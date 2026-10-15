---
title: modulation et sidechain
order: 190
summary: Faites bouger des paramètres avec des LFO, des enveloppes et d’autres modulateurs, et laissez une piste contrôler un appareil sur une autre.
platform: all
audience: advanced
covers:
  - racks-modulation/adding-a-modulator #adding-a-modulator
  - racks-modulation/mapping-a-modulator-to-parameters #mapping-a-modulator
  - racks-modulation/modulation-depth-chips-and-rings #depth-rings-and-chips
  - racks-modulation/modulator-types #modulator-types
  - racks-modulation/sidechain-a-device #sidechain
  - racks-modulation/sidechain-from-your-own-track-or-a-muted-track #keying-from-your-own-track-or-a-muted-track
---

La **modulation** fait bouger un paramètre tout seul : un filtre qui ondule avec un LFO, une coupure qui s’ouvre à chaque note, un volume qui suit une autre piste. Un **sidechain** permet à un appareil d’écouter une autre piste, l’exemple classique étant un compresseur sur la basse qui l’abaisse à chaque coup de grosse caisse. Lisez d’abord [appareils et presets](/docs/making-music/devices-and-presets).

La modulation est différente de l’[automation](/docs/making-music/automation#automation-or-modulation) : l’automation écrit des valeurs sur la timeline, alors que la modulation fait bouger un paramètre autour de la valeur que vous avez réglée, en continu.

## ajouter un modulateur {#adding-a-modulator}

Survolez l’en-tête d’un appareil et cliquez sur son bouton **+** (ajouter un modulateur), puis choisissez un type (voir plus bas). Le menu du clic droit de l’appareil propose les mêmes choix (**ajouter un LFO**, **ajouter une enveloppe**…). Le modulateur apparaît sur la carte de l’appareil, avec ses propres réglages.

Seuls les appareils placés directement dans la chaîne d’une piste peuvent porter des modulateurs : un appareil dans une chaîne de rack ou sur un pad de batterie ne le peut pas. Pour moduler les appareils d’un rack, ajoutez le modulateur au rack lui-même.

## assigner un modulateur {#mapping-a-modulator}

Un modulateur ne fait rien tant que vous ne l’assignez pas à des paramètres. Deux façons :

- Cliquez sur le bouton **assigner** du modulateur : tous les paramètres qu’il peut atteindre s’allument. Cliquez sur les paramètres qu’il doit faire bouger. Maintenez [[key:Shift]] en cliquant pour continuer à en assigner ; appuyez sur [[key:Escape]] pour arrêter.
- Faites glisser la poignée du modulateur sur un paramètre.

Un modulateur peut atteindre les paramètres de son propre appareil. Un modulateur sur un rack peut atteindre tous les appareils du rack (et le sélecteur de chaîne du rack).

### anneaux et pastilles de profondeur {#depth-rings-and-chips}

Chaque assignation apparaît sur son paramètre :

- un **anneau** sur le bouton rotatif, à la couleur du modulateur, qui va de la valeur du bouton à cette valeur plus la profondeur. Pendant la lecture du morceau, un point montre où la modulation a amené le paramètre à cet instant ;
- une **pastille** sous le réglage. Faites glisser la pastille vers le haut ou le bas pour régler la profondeur, double-cliquez dessus pour la remettre à 0, faites un clic droit pour **inverser** la modulation ou la **retirer**.

Les nouvelles assignations commencent à 50 % de profondeur. La carte du modulateur liste aussi ses cibles avec leur profondeur.

## types de modulateurs {#modulator-types}

| type | ce qu’il fait | réglages principaux |
|---|---|---|
| **LFO** | une onde qui se répète | forme (sinus, triangle, dent de scie montante ou descendante, carré), vitesse libre ou synchronisée, phase, redéclenchement (**libre**, à chaque **note**, ou depuis le **transport**), montée progressive |
| **enveloppe** | une forme qui démarre à chaque note | attaque, déclin, maintien, relâchement, vélocité |
| **suiveur d’enveloppe** | suit le volume d’un son | attaque, relâchement, gain ; il écoute l’entrée de l’appareil, ou une autre piste que vous choisissez |
| **pas à pas** | une séquence de 1 à 16 valeurs | nombre de pas, vitesse, lissage, et une valeur par pas |
| **aléatoire** | une nouvelle valeur au hasard à chaque pas | vitesse libre ou synchronisée, lissage |
| **suivi de clavier** | suit la note que vous jouez | les touches basse et haute de sa plage |
| **vélocité** | suit la force avec laquelle vous jouez | les vélocités basse et haute de sa plage |

## sidechain {#sidechain}

Certains appareils ont une entrée sidechain : le **compresseur**, le **limiteur**, le **gate**, le **compresseur multibande** et le **filtre auto**, ainsi que les plug-ins qui acceptent un sidechain. Leur en-tête affiche un menu **sidechain**.

1. Dans le menu sidechain, choisissez la piste source. **pas de sidechain** le désactive.
2. Un second menu choisit ensuite où écouter sur cette piste : **pré-fx** (avant ses appareils), **post-fx** (après ses appareils) ou **post-mixeur** (après son volume et son panoramique), sur la piste elle-même ou sur l’une de ses chaînes de rack ou l’un de ses pads de batterie.

L’appareil réagit alors à la source au lieu de sa propre entrée : un compresseur sur la basse, déclenché par la grosse caisse, baisse la basse à chaque coup. Les choix qui créeraient une boucle sont grisés, avec la raison.

### déclencher depuis votre propre piste ou une piste muette {#keying-from-your-own-track-or-a-muted-track}

- Un appareil peut écouter le signal **pré-fx** de sa propre piste, ou une chaîne d’un rack placé avant lui sur la même piste. Sa propre piste apparaît dans la liste comme **(cette piste)**.
- Mettre en muet ou en solo la piste source ne coupe pas le sidechain : une piste muette déclenche toujours l’appareil. Vous pouvez utiliser une piste muette comme simple déclencheur, par exemple une grosse caisse muette qui ne sert qu’à piloter un compresseur de ducking.

## et ensuite {#where-next}

- [Racks et macros](/docs/making-music/racks-and-macros#macros) : les macros, les cousines des modulateurs qu’on contrôle à la main.
- [Effets](/docs/making-music/effects) : les appareils que vous modulerez le plus.
- [Automation](/docs/making-music/automation) : des changements écrits sur la timeline.
