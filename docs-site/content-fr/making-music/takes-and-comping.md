---
title: prises et comping
order: 150
summary: Retrouvez vos prises enregistrées et construisez une seule interprétation à partir des meilleurs passages de chacune.
platform: all
audience: everyone
covers:
  - clips/takes-where-they-come-from #where-takes-come-from
  - clips/swipe-comping-and-choosing-takes #comping-picking-the-best-parts
  - clips/take-lane-menu-crossfades-promote-flatten #the-take-lane-menu
---

Quand vous enregistrez plusieurs fois la même partie, chaque passage est conservé comme une **prise**. Le comping (de l’anglais « compiling », compiler) consiste à construire une seule interprétation à partir des meilleurs passages de chaque prise : le couplet de la prise 2, la dernière phrase de la prise 4.

## d’où viennent les prises {#where-takes-come-from}

Les prises sont créées automatiquement quand vous [enregistrez](/docs/making-music/recording) :

- plusieurs passages de la boucle,
- avec le **punch** activé,
- sur des clips déjà présents.

Chaque passage va sur son propre **couloir de prises** sous la piste, et la plus récente est lue. Les clips qui se trouvaient déjà dans la plage enregistrée passent sur le premier couloir de prises : rien n’est perdu.

Une piste qui a des prises affiche un bouton des prises dans son en-tête, avec le nombre de prises. Cliquez dessus pour afficher ou masquer les couloirs de prises. Vous pouvez aussi faire un clic droit sur l’en-tête de la piste et choisir **afficher les couloirs de prises**, ou **nouveau couloir de prises** pour ajouter un couloir vide à la main.

## comping : choisir les meilleurs passages {#comping-picking-the-best-parts}

Chaque couloir de prises dessine ses clips, et met en évidence les plages qui sont actuellement lues depuis cette prise.

- **Balayer** : faites glisser sur un couloir de prises. Au relâchement, cette plage de la piste est lue depuis cette prise. La plage est aimantée à la grille ([[key:Alt]] : sans aimantation).
- **Cliquez** sur un couloir de prises pour choisir cette prise pour le passage sous le pointeur. Ce passage est alors sélectionné.
- Avec un passage sélectionné, [[action:take.previous]] et [[action:take.next]] parcourent les prises pour ce passage, pour les comparer rapidement.
- Le bouton casque dans l’en-tête d’un couloir permet d’**écouter** cette prise : elle est lue seule, pour que vous l’entendiez en entier.

Chaque modification du comping est une étape de l’[historique d’annulation](/docs/basics/undo-and-history).

## le menu du couloir de prises {#the-take-lane-menu}

Faites un clic droit sur un couloir de prises pour plus d’options :

| élément | ce qu’il fait |
|---|---|
| **écouter la prise** / **arrêter l’écoute** | lit cette prise seule |
| **utiliser toute la prise** | toute l’étendue de cette prise est lue depuis elle |
| **promouvoir le clip sur le couloir principal** | sort le clip sous le pointeur des prises, pour le placer sur la piste |
| **retirer du comping** | le passage sous le pointeur n’est plus lu depuis cette prise |
| **pas de fondu enchaîné**, **fondu enchaîné 5 ms**, **20 ms**, **100 ms** | le fondu enchaîné aux bords du passage sous le pointeur, pour masquer les clics là où les prises se rejoignent |
| **renommer** | renomme la prise |
| **monter** / **descendre** | réordonne les couloirs de prises |
| **supprimer la prise** | retire la prise |

### aplatir {#flattening}

Faites un clic droit sur l’en-tête de la piste pour deux choix de plus, une fois que la piste a un comping :

- **aplatir le comping** fige le comping en clips normaux sur la piste et garde les couloirs de prises.
- **aplatir le comping et supprimer les prises** fige le comping et supprime toutes les prises. Une confirmation est demandée d’abord.

> [!warning]
> **Aplatir le comping et supprimer les prises** jette les autres prises. Vous pouvez l’annuler tout de suite, mais une fois sorti de l’historique d’annulation, les prises sont perdues. Utilisez **aplatir le comping** si vous pourriez en avoir besoin plus tard.

## et ensuite {#where-next}

- [Enregistrement](/docs/making-music/recording) : enregistrer d’autres prises.
- [Clips audio](/docs/making-music/audio-clips) : fondus et fondus enchaînés sur le résultat.
- [Geler, aplatir et faire un rendu](/docs/advanced/freeze-and-bounce) : convertir une piste en audio de façon définitive.
