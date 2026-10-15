---
title: mixage
order: 120
summary: Équilibrez vos pistes avec le volume, le panoramique, le muet et le solo, routez-les vers des groupes et des pistes de retour, et utilisez les envois.
platform: all
audience: everyone
covers:
---

Mixer, c’est régler le volume de chaque piste, sa place de gauche à droite, et les effets qu’elle partage avec d’autres pistes. Cette page présente les réglages de mixage d’une piste et le trajet de l’audio des pistes jusqu’au master. Il vous faut quelques pistes qui produisent du son.

> [!note]
> Sur ordinateur, il n’y a pas de fenêtre de mixeur séparée : les réglages de mixage sont dans l’en-tête de chaque piste et dans l’inspecteur. La bêta mobile a sa propre vue de mixeur.

## le mixeur d’une piste {#a-tracks-mixer}

Chaque piste a ses réglages de mixage à deux endroits : en version compacte dans son en-tête dans l’arrangement, et en plus grand dans la section **mixeur** de l’inspecteur quand la piste est sélectionnée.

- **volume** : du silence jusqu’à +6 dB. 0 dB laisse le niveau inchangé. Double-cliquez pour le remettre à 0 dB.
- **panoramique** : de gauche à droite. Double-cliquez pour le centrer.
- **muet** coupe la piste.
- **solo** ne fait jouer que cette piste (et les autres pistes en solo). Un clic met seulement cette piste en solo ; [[key:Mod]]-clic l’ajoute aux pistes en solo ou l’en retire.
- L’**indicateur de niveau** dans l’en-tête de la piste montre son niveau pendant la lecture.

Tous ces réglages peuvent être automatisés (voir [automation](/docs/making-music/automation)) et assignés à un contrôleur MIDI.

## où va le son {#where-the-sound-goes}

Chaque piste envoie son audio vers une **sortie**, choisie dans le menu **sortie** de l’inspecteur :

- par défaut, vers la piste **master**, ou vers son groupe si la piste est dans un [groupe](/docs/making-music/tracks) ;
- vers une piste de **groupe** ou de **retour** de votre choix ;
- vers **aucune sortie** : la piste ne s’entend pas, mais elle peut toujours déclencher un sidechain.

La piste **master** est le bout de la chaîne : tout ce que vous entendez et exportez passe par elle. Elle a un volume, un panoramique et un muet, et ses propres appareils (un limiteur est un dernier appareil courant), mais pas de solo ni de menu de sortie.

### les groupes comme bus {#groups-as-buses}

Quand vous groupez des pistes (sélectionnez-les et appuyez sur [[action:track.group]]), la piste de groupe devient un bus : ses enfants passent par elle. Le volume, le panoramique et les appareils du groupe agissent sur toutes à la fois, par exemple un seul compresseur sur toutes vos batteries.

## envois et pistes de retour {#sends-and-return-tracks}

Une **piste de retour** contient un effet que plusieurs pistes partagent, comme une seule réverb pour tout le morceau. Chaque piste lui envoie une partie de son son, avec un bouton d’**envoi**.

- Pour ajouter une piste de retour, ouvrez le menu **ajouter une piste** et choisissez **piste de retour**. On la trouve aussi dans le menu du clic droit d’une piste (**ajouter une piste de retour**), dans le menu de l’espace vide sous les pistes, et dans la palette de commandes ([[action:palette.open]]). Sur la bêta mobile, elle est dans les actions de la fiche de piste. Une nouvelle piste de retour se place juste après le retour sélectionné, ou sous les autres retours, au-dessus du master. Vous pouvez aussi faire glisser **piste de retour** hors du menu **ajouter une piste** et la déposer entre deux retours. Mettez-y une réverb ou un délai.
- Pour réordonner les pistes de retour, faites glisser l’en-tête d’un retour vers le haut ou le bas entre les autres retours (voir [déplacer des pistes](/docs/making-music/tracks#moving-tracks)). Sur la bêta mobile, utilisez **monter** et **descendre** dans sa fiche de piste. Chaque envoi continue d’aller vers le même retour.
- Quand un projet a des pistes de retour, la section mixeur de l’inspecteur affiche un bouton d’**envoi** par piste de retour, dans l’ordre des retours. Les envois partent du silence : montez-en un pour envoyer cette piste vers ce retour. Le premier réglage crée l’envoi.
- Sous chaque bouton d’envoi, un petit bouton **post** / **pre** règle d’où l’envoi est prélevé. **post** (par défaut) le prélève après le volume de la piste (post-fader) : baisser une piste baisse aussi ce qu’elle envoie. **pre** le prélève avant le volume (pre-fader) : l’envoi garde son niveau quoi que fasse le fader, ce qui est pratique pour un mix casque séparé ou un effet qui doit continuer pendant que la piste s’éteint en fondu. Le bouton reste grisé tant que l’envoi n’existe pas. Le basculer compte pour une étape d’annulation.
- Les niveaux d’envoi peuvent être automatisés comme le volume.

## et ensuite {#where-next}

- [Effets](/docs/making-music/effects) : le compresseur, l’égaliseur et le limiteur pour le mixage.
- [Modulation et sidechain](/docs/making-music/modulation-and-sidechain#sidechain) : abaisser une piste sous une autre.
- [Automation](/docs/making-music/automation) : faire évoluer le mix dans le temps.
