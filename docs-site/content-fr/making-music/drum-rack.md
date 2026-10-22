---
title: le drum rack
order: 80
summary: Construisez un kit de batterie à partir d’échantillons, un son par pad, avec des groupes d’étouffement et des effets par pad.
platform: all
audience: beginner
covers:
  - racks-modulation/drum-rack-pads #pads
  - racks-modulation/choke-groups #choke-groups
  - racks-modulation/pad-solo-isn-t-saved-or-undoable #mute-and-solo-pads
  - racks-modulation/slice-a-sample-into-a-drum-rack #slicing-a-sample-to-pads
---

Un drum rack est un instrument qui joue un son différent sur chaque note : une grosse caisse sur C1, une caisse claire sur D1, et ainsi de suite. Chaque son vit sur un **pad**, et chaque pad a son propre volume, son panoramique et sa chaîne d’appareils. Il vous faut une piste MIDI : un drum rack est un instrument, il va donc dans l’emplacement d’instrument de la piste (voir [appareils et presets](/docs/making-music/devices-and-presets#adding-a-device)).

## créer un kit {#making-a-kit}

1. Sur une piste MIDI, ajoutez **drum rack** depuis la bibliothèque d’instruments. Sa fenêtre s’ouvre.
2. Ouvrez le [navigateur](/docs/sounds-and-plugins/browser) et faites glisser un échantillon sur un pad vide. Le pad est créé avec un sampler qui joue cet échantillon.
3. Recommencez pour chaque son. Déposer un échantillon sur un pad qui en a déjà un remplace l’échantillon.
4. Écrivez le rythme : un clip MIDI sur une piste de drum rack ouvre l’[éditeur de rythme](/docs/making-music/piano-roll#drum-rack-tracks-open-the-beat-editor), où chaque ligne est un pad.

## découper un échantillon en pads {#slicing-a-sample-to-pads}

Une boucle de batterie peut devenir un kit : Ethereal la coupe à ses transitoires (le début de chaque coup) et met chaque tranche sur son propre pad, avec un sampler qui ne joue que cette tranche.

- Dans la fenêtre d’un drum rack, cliquez sur **découper en pads** et choisissez un échantillon, ou faites glisser un échantillon du [navigateur](/docs/sounds-and-plugins/browser) sur ce bouton. Les tranches vont sur les touches juste au-dessus des pads du rack (à partir de C1 sur un rack vide).
- Dans le navigateur, faites un clic droit sur un échantillon : **découper dans un nouveau drum rack** crée une nouvelle piste MIDI avec un drum rack de ses tranches ; **découper dans le drum rack sélectionné** les ajoute au drum rack de la piste sélectionnée.
- Faites un clic droit sur un clip audio et choisissez **découper en drum rack** : son échantillon est découpé dans un drum rack sur une nouvelle piste MIDI, juste sous la piste du clip.
- Un [sampler en mode tranches](/docs/making-music/instruments#slice-mode) a aussi un bouton **découper en drum rack**, qui utilise les tranches réglées à la main.

Chacune de ces actions est une seule étape d’annulation.

## les pads {#pads}

La fenêtre affiche 16 pads (4 × 4), soit une **banque**. Le menu **banque** affiche d’autres groupes de 16 notes. La première banque commence sur **C1** (note 36), là où commencent la plupart des kits de batterie. Ethereal nomme les notes avec C3 = 60, comme Ableton Live.

- **Cliquez** sur un pad pour le sélectionner. Ses réglages et sa chaîne apparaissent sous la grille.
- **Maintenez appuyé** un pad pour l’entendre. Le son passe par la chaîne du pad exactement comme en lecture, que la piste soit armée ou non et que le morceau soit en lecture ou non. Le bouton de lecture d’un pad fait la même chose.
- **Double-cliquez** sur le nom d’un pad pour le renommer. Un nom vide rétablit le nom de la note.
- **Faites un clic droit** sur un pad pour **renommer**, **muet**, **solo**, un groupe d’étouffement rapide, et **supprimer le pad**.

Les réglages du pad sélectionné :

- **volume**, **panoramique** et **accord** (l’accord transpose le sampler du pad, en demi-tons) ;
- **étouffement** : son groupe d’étouffement ;
- sa **chaîne** : ajoutez des effets à ce pad seulement (une réverbe sur la caisse claire, un saturateur sur la grosse caisse). La chaîne d’un pad fonctionne comme celle d’une piste, avec son propre emplacement d’instrument.

Un pad vide a aussi un emplacement d’instrument : y ajouter un instrument crée le pad avec lui, pour qu’un pad puisse jouer un synthé au lieu d’un échantillon.

## groupes d’étouffement {#choke-groups}

Les pads d’un même **groupe d’étouffement** (1 à 16) se coupent les uns les autres : quand l’un démarre, les autres de son groupe s’arrêtent. L’usage classique, c’est le charleston : mettez le charleston fermé et le charleston ouvert dans le même groupe, pour que le fermé coupe l’ouvert, comme sur une vraie batterie.

## pads muets et en solo {#mute-and-solo-pads}

Chaque pad a des boutons muet et solo, comme une piste.

- **Solo** : un clic met seulement ce pad en solo, un [[key:Mod]]-clic l’ajoute aux pads en solo ou l’en retire.
- Le **muet** d’un pad fait partie du projet, comme le muet d’une piste.
- Le **solo** d’un pad ne sert qu’à écouter sur le moment : il n’est pas enregistré avec le projet et ne compte pas comme une étape d’annulation.

## macros {#macros}

Un drum rack a **8 macros**, comme les autres [racks](/docs/making-music/racks-and-macros#macros), en haut de sa fenêtre (et dans sa liste de paramètres). Associez une macro aux paramètres des appareils de n’importe quel pad, même dans un rack posé sur un pad : une seule macro peut raccourcir la décroissance de tous les échantillons ou ouvrir les filtres de tout le kit. Une macro d’un rack qui contient le drum rack atteint aussi ses pads. Les macros s’automatisent et s’associent à un contrôleur MIDI comme n’importe quel bouton, et un preset de drum rack les garde.

## et ensuite {#where-next}

- [Effets](/docs/making-music/effects) : des effets pour chaque pad.
- [Racks et macros](/docs/making-music/racks-and-macros) : des racks pour superposer des sons et passer de l’un à l’autre.
- [Appareils et presets](/docs/making-music/devices-and-presets#presets) : enregistrez le son des appareils d’un pad en presets.
