---
title: racks et macros
order: 180
summary: Combinez des appareils dans des racks à chaînes parallèles, passez d’un son à l’autre et contrôlez de nombreux paramètres avec 8 boutons de macro.
platform: all
audience: advanced
covers:
  - racks-modulation/group-a-device-into-a-rack #making-a-rack
  - racks-modulation/rack-chains #chains
  - racks-modulation/chain-zones-keys-velocity-chain-select #zones-keys-velocity-and-chain-select
  - racks-modulation/rack-content-rules-and-nesting-depth #what-a-rack-can-hold
  - racks-modulation/rack-macros #macros
  - racks-modulation/rack-presets-keep-the-whole-rack #rack-presets
---

Un rack est un appareil qui contient d’autres appareils, dans une ou plusieurs **chaînes** qui jouent côte à côte. Avec un rack, vous pouvez superposer deux synthés, partager le clavier entre une basse et un lead, faire tourner un effet en parallèle du son sec, ou changer de son avec un seul bouton. Chaque rack a aussi **8 boutons de macro** qui peuvent chacun contrôler de nombreux paramètres à l’intérieur. Lisez d’abord [appareils et presets](/docs/making-music/devices-and-presets).

Il en existe trois sortes :

- **rack d’instruments** : des chaînes d’instruments (avec leurs propres effets), pour les pistes MIDI ;
- **rack d’effets audio** : des chaînes d’effets audio ;
- **rack d’effets MIDI** : des chaînes d’effets MIDI.

## créer un rack {#making-a-rack}

Deux façons :

- Ajoutez **rack d’instruments**, **rack d’effets audio** ou **rack d’effets MIDI** depuis la bibliothèque d’appareils, comme n’importe quel appareil.
- Faites un clic droit sur l’en-tête d’un appareil sur une piste et choisissez **grouper dans un rack d’instruments** (ou **rack d’effets audio**, **rack d’effets MIDI**, selon l’appareil). L’appareil passe dans un nouveau rack, sur sa première chaîne. Ce n’est pas proposé pour les drum racks, ni pour les appareils qui sont déjà dans un rack ou sur un pad de batterie.

Un rack n’a pas de fenêtre : ses chaînes et ses macros sont sur sa carte.

## chaînes {#chains}

La carte du rack affiche une ligne par chaîne. Une ligne montre le nom de la chaîne, puis les mêmes réglages qu’un en-tête de piste : **muet**, **solo**, **volume** et **panoramique**, un bouton **zones**, et **×** pour supprimer la chaîne.

- Cliquez sur la flèche d’une ligne pour la déplier et voir ses appareils, dans les mêmes sections que la chaîne d’une piste (effets MIDI, instrument, effets audio).
- **Ajouter une chaîne** : dans un rack d’instruments, **ajouter un instrument** crée une nouvelle chaîne avec l’instrument que vous choisissez. Dans un rack d’effets, **ajouter une chaîne** ajoute une chaîne vide.
- **Solo** : un clic met seulement cette chaîne en solo ; un [[key:Mod]]-clic l’ajoute aux chaînes en solo ou l’en retire.
- **Déplacer un appareil** d’une chaîne à l’autre : faites glisser sa carte sur la ligne d’une autre chaîne.
- Double-cliquez sur le nom d’une chaîne, ou faites un clic droit sur sa ligne, pour la renommer. Une chaîne sans nom affiche le nom de son instrument ou de ses effets.

Toutes les chaînes reçoivent la même entrée et jouent en même temps ; leurs sorties sont mélangées.

### zones : touches, vélocité et sélection de chaîne {#zones-keys-velocity-and-chain-select}

Le bouton **zones** d’une chaîne règle les notes auxquelles elle répond, avec trois plages de 0 à 127 :

- **touches** : les notes qu’elle joue. Donnez les touches graves à une chaîne de basse et les touches aiguës à une chaîne de lead pour partager le clavier.
- **vél** : les vélocités qu’elle joue. Une couche douce pour les notes légères, une plus dure pour les notes fortes.
- **sélection de chaîne** : les valeurs du **sélecteur de chaîne** du rack qui l’activent.

Chaque rack a un paramètre **sélecteur de chaîne** (0 à 127). Donnez à chaque chaîne une plage de sélection différente, puis tournez le sélecteur, automatisez-le ou associez-lui une macro, pour passer d’un son à l’autre avec un seul réglage. Par défaut, chaque chaîne répond à toutes les touches, toutes les vélocités et toutes les valeurs du sélecteur.

### ce qu’un rack peut contenir {#what-a-rack-can-hold}

- Un rack d’effets audio n’accepte que des effets audio, et un rack d’effets MIDI que des effets MIDI.
- Un rack d’instruments accepte tout, y compris un drum rack (sauf si le rack est lui-même sur un pad de batterie).
- Les racks peuvent aller dans des racks, jusqu’à 6 niveaux de profondeur. Les chaînes les plus profondes ne proposent pas de racks.

## macros {#macros}

Chaque rack a 8 macros. Ouvrez la liste des paramètres du rack (le bouton à curseurs de son en-tête) pour les voir sous forme de boutons, avec le sélecteur de chaîne.

Pour associer une macro :

1. Faites glisser la petite poignée d’association de la macro sur un paramètre d’un appareil du rack, ou cliquez sur la poignée puis sur le paramètre. Vous pouvez aussi faire un clic droit sur le paramètre et choisir **assigner à une macro**, puis la macro (dans un rack placé dans un autre rack, d’abord le rack).
2. L’association apparaît sur ce paramètre sous forme d’anneau coloré et de pastille de profondeur, comme pour la [modulation](/docs/making-music/modulation-and-sidechain#depth-rings-and-chips). Faites glisser la pastille pour régler l’amplitude du mouvement de la macro.

Une macro peut piloter de nombreux paramètres, chacun avec sa propre profondeur. Par exemple, une macro « brillance » peut ouvrir un filtre et ajouter un peu de drive en même temps. Une macro peut aussi piloter le sélecteur de chaîne du rack lui-même. Les macros peuvent être automatisées et associées à un contrôleur MIDI comme n’importe quel bouton.

Une macro peut aussi **activer et contourner un appareil** : déposez-la sur le bouton d’alimentation de l’appareil (ou faites un clic droit sur le bouton et choisissez **assigner à une macro**). Un appareil activé se contourne quand la macro passe le milieu, un appareil contourné s’active ; un clic droit sur la pastille colorée à côté du bouton l’inverse ou la retire. Le bouton montre l’état de l’appareil en direct. Les modulateurs peuvent commuter leur propre appareil de la même façon.

## presets de rack {#rack-presets}

Enregistrer un preset sur un rack (avec son bouton **presets**) enregistre tout le rack : ses chaînes, leurs appareils et leurs réglages, ses modulateurs et ses associations de macros. Le charger reconstruit tout cela, en une seule étape d’annulation. C’est ainsi que vous gardez un son superposé ou une chaîne d’effets pour d’autres projets.

## et ensuite {#where-next}

- [Modulation et sidechain](/docs/making-music/modulation-and-sidechain) : des LFO et des enveloppes dans un rack.
- [Le drum rack](/docs/making-music/drum-rack) : un rack avec une chaîne par note.
- [Appareils et presets](/docs/making-music/devices-and-presets#presets) : les presets.
