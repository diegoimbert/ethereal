---
title: automation
order: 110
summary: Dessinez point par point l’évolution du volume, du panoramique et de n’importe quel paramètre d’appareil au fil du morceau.
platform: all
audience: beginner
covers:
  - automation/show-automation-lanes #showing-automation-lanes
  - automation/adding-points #adding-points
  - automation/editing-points #editing-points
  - automation/shift-snaps-values-to-round-numbers #round-values-with-shift
  - automation/curves #curves
  - automation/set-an-exact-value #set-an-exact-value
  - automation/time-ranges-in-a-lane #copying-and-moving-a-time-range
  - automation/lane-height-and-value-zoom #lane-height-and-value-zoom
  - automation/moving-an-automated-knob-doesn-t-stick #automated-knobs-follow-their-lane
  - automation/hide-vs-disable-vs-delete-a-lane #hide-disable-or-delete-a-lane
  - automation/device-on-off-automation #turning-a-device-on-and-off
  - automation/clip-envelopes #clip-envelopes
  - automation/automation-vs-modulation #automation-or-modulation
---

L’automation fait varier un paramètre dans le temps : un filtre qui s’ouvre pendant la montée, un fondu de sortie à la fin, une réverb qui grandit sur le dernier refrain. Chaque paramètre automatisé a un **couloir** sous sa piste, avec des points que vous placez sur la timeline. La lecture suit la ligne entre les points. Il vous faut une piste avec quelque chose à automatiser : son volume et son panoramique sont toujours là, et chaque paramètre d’appareil peut aussi être automatisé.

## afficher les couloirs d’automation {#showing-automation-lanes}

- Cliquez sur le bouton **automation** dans l’en-tête d’une piste pour ouvrir ses couloirs sous la piste. La première fois, il affiche les couloirs qui contiennent déjà de l’automation, ou le couloir de volume s’il n’y en a aucun. La piste master affiche d’abord son couloir de tempo.
- **+ paramètre…** ajoute un autre couloir : choisissez n’importe quel paramètre de la piste (volume, panoramique, envois, l’activation et les paramètres de chaque appareil). Activez le filtre **actifs** dans ce sélecteur pour ne lister que les paramètres qui ont déjà de l’automation. Ce choix est mémorisé sur cet ordinateur.
- Depuis un appareil : faites un clic droit sur n’importe quel bouton rotatif et choisissez **afficher le couloir d’automation**.

L’onglet **automation** du tiroir de l’éditeur ([[action:view.editor]]) affiche les couloirs de la piste sélectionnée avec leur propre règle, si vous préférez une vue plus grande.

## ajouter des points {#adding-points}

**Double-cliquez** à un endroit vide d’un couloir pour ajouter un point. Le point apparaît au second clic : vous pouvez garder le bouton enfoncé et faire glisser pour le placer précisément avant de relâcher. Un couloir est créé par son premier point.

Il n’y a ni crayon ni dessin à main levée : vous construisez la ligne point par point.

## modifier des points {#editing-points}

- **Sélectionner** : cliquez sur un point. [[key:Shift]]-clic ajoute des points à la sélection, [[key:Mod]]-clic en ajoute ou en retire un. [[key:Mod]]-glisser dans un espace vide trace un rectangle de sélection.
- **Déplacer** : faites glisser les points sélectionnés. Le temps avance par pas de grille entiers, donc un point entre deux lignes de la grille garde sa place par rapport à elles, et un glissement surtout vers le haut ou le bas ne le décale jamais dans le temps ; maintenez [[key:Alt]] pour un placement précis, et [[key:Mod]] pour limiter le déplacement à une seule direction (temps ou valeur). Une infobulle affiche la valeur et ces touches pendant le glissement.
- **Supprimer** : double-cliquez sur un point, ou sélectionnez des points et appuyez sur [[action:edit.delete]].
- **Tout sélectionner** dans le couloir actif : [[action:edit.selectAll]].
- **Décaler** avec les flèches : haut et bas changent la valeur (d’un cran, ou de 1 %), gauche et droite déplacent d’un pas de grille. Avec [[key:Shift]], haut et bas sautent à la valeur entière suivante (par exemple 1 dB) et gauche et droite déplacent d’un quart de pas. [[action:automation.preciseUp]] et [[action:automation.preciseDown]] changent la valeur d’un dixième du pas normal.
- **Copier, couper, coller, dupliquer** des points avec [[action:edit.copy]], [[action:edit.cut]], [[action:edit.paste]] et [[action:edit.duplicate]].
- Un clic droit sur un point propose **couper**, **copier**, **coller**, **dupliquer**, **définir la valeur…**, le type de courbe et **supprimer**.

### valeurs rondes avec maj {#round-values-with-shift}

Pendant que vous faites glisser un point, maintenez [[key:Shift]] pour changer la valeur par pas entiers de ce qu’elle affiche : 1 dB, 1 %, 1 demi-ton, 0,01 de panoramique, etc. Le point tombe sur une valeur nette. Les paramètres qui n’ont que des crans (demi-tons, choix, activé/désactivé) s’y aimantent toujours.

Quand un pixel est trop grossier pour la valeur voulue, maintenez [[key:Alt]] pendant le glissement : le point se déplace dix fois moins que la souris, et hors de la grille. Vous pouvez l’enfoncer ou le relâcher en plein glissement ; le point repart de là où il est, sans sauter. Avec [[key:Shift]] et [[key:Alt]] ensemble, la valeur avance toujours par pas entiers et seul le temps ralentit.

### courbes {#curves}

- [[key:Alt]]-glissez un segment (la ligne entre deux points) pour le courber.
- Le menu **courbe** dans l’en-tête du couloir règle **linéaire**, **palier** ou **lisse** pour les segments qui suivent les points sélectionnés. Un clic droit sur un point propose les mêmes choix.

### définir une valeur exacte {#set-an-exact-value}

Faites un clic droit sur un point et choisissez **définir la valeur…**. Tapez la valeur dans l’unité du paramètre (dB, Hz, %…), puis appuyez sur [[key:Enter]] pour l’appliquer ou sur [[key:Escape]] pour annuler.

## copier et déplacer une plage de temps {#copying-and-moving-a-time-range}

Faites glisser dans un espace vide d’un couloir pour sélectionner une plage de temps dans ce couloir. Ensuite :

- [[action:edit.copy]], [[action:edit.cut]] et [[action:edit.delete]] copient, coupent ou suppriment l’automation dans la plage. La courbe en dehors de la plage reste telle quelle.
- [[action:edit.paste]] colle après la plage, et [[action:edit.duplicate]] répète la plage juste après elle-même.

Vous pouvez coller dans le couloir d’un autre paramètre : les valeurs sont converties dans la plage du nouveau paramètre.

## hauteur du couloir et zoom des valeurs {#lane-height-and-value-zoom}

- Faites glisser le bord inférieur d’un couloir pour l’agrandir ou le réduire. Il se déplace par crans ; maintenez [[key:Alt]] pour redimensionner librement. Double-cliquez sur le bord pour le réinitialiser.
- L’**échelle des valeurs** à côté de l’en-tête du couloir : faites-la glisser ou faites défiler pour déplacer la plage de valeurs visible, [[key:Mod]]-défilement pour zoomer sur les valeurs, et double-cliquez pour réinitialiser.
- Les paramètres à crans s’ouvrent zoomés, pour que chaque cran soit visible (un couloir de transposition affiche d’abord ±3 demi-tons). Redimensionnez le couloir ou zoomez l’échelle pour en voir plus.

## masquer, désactiver ou supprimer un couloir {#hide-disable-or-delete-a-lane}

L’en-tête du couloir a trois boutons différents :

| bouton | ce qu’il fait | les points |
|---|---|---|
| **×** | masque le couloir | conservés, et toujours lus |
| **⏻** | désactive l’automation : la lecture l’ignore | conservés |
| **⌫** | supprime le couloir | supprimés (annuler les fait revenir) |

## les boutons automatisés suivent leur couloir {#automated-knobs-follow-their-lane}

Dès qu’un paramètre a un couloir activé, son bouton suit le couloir, pendant la lecture comme à l’arrêt. Si vous tournez le bouton, le changement ne dure que jusqu’à ce que le moteur renvoie la valeur automatisée : il ne reste pas. Il n’y a ni enregistrement d’automation ni bouton « override ».

Pour régler à nouveau le paramètre à la main, désactivez le couloir avec son bouton **⏻**, ou supprimez le couloir.

## activer et désactiver un appareil {#turning-a-device-on-and-off}

L’interrupteur d’un appareil peut être automatisé comme n’importe quel paramètre : dans la liste **+ paramètre…**, il s’appelle **appareil activé**, dans le groupe des paramètres de cet appareil. Tant qu’il est automatisé, le bouton **alimentation** de l’appareil indique si l’automation l’active ou le contourne. Cliquer sur le bouton ne change l’appareil que là où le couloir ne décide pas.

## enveloppes de clip {#clip-envelopes}

Un clip peut porter sa propre automation, appelée **enveloppes de clip**. Elles appartiennent au clip : elles se déplacent avec lui, se répètent quand le clip boucle, et sont copiées avec lui.

- Dans le piano roll, le sélecteur de couloir sous les notes liste tous les paramètres de la piste. Les paramètres qui ont déjà une enveloppe dans ce clip viennent en premier.
- Pour les clips audio, l’éditeur de warp a une bande d’enveloppes sous la forme d’onde.

Pendant la lecture d’un clip, son enveloppe **remplace** le couloir de la piste pour ce paramètre ; en dehors du clip, c’est le couloir de la piste qui s’applique. **Désactivez** une enveloppe pour la garder tout en laissant jouer le couloir de la piste.

Les clips audio ont aussi deux enveloppes à eux, **gain du clip** et **transposition**. Elles s’ajoutent aux réglages de gain et de transposition du clip au lieu de remplacer un couloir de piste.

## automation ou modulation {#automation-or-modulation}

- L’**automation** écrit des valeurs exactes sur la timeline, un couloir par paramètre. Utilisez-la pour les changements liés à un moment du morceau.
- La **modulation** (LFO, enveloppes, macros) fait bouger un paramètre autour de sa valeur actuelle, en continu, d’une quantité que vous réglez. Utilisez-la pour un mouvement qui se répète ou qui suit les notes. Voir [modulation et sidechain](/docs/making-music/modulation-and-sidechain).

Les deux peuvent piloter le même paramètre : la modulation bouge autour de la valeur automatisée.

## et ensuite {#where-next}

- [Modulation et sidechain](/docs/making-music/modulation-and-sidechain) : un mouvement qui suit un LFO, les notes ou une autre piste.
- [Mixage](/docs/making-music/mixing) : volume, panoramique et envois, que vous pouvez aussi automatiser.
- [Appareils et presets](/docs/making-music/devices-and-presets#the-parameter-menu) : le menu des paramètres.
